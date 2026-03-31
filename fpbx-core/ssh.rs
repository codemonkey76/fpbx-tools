use anyhow::{bail, Context, Result};
use ssh2::Session;
use std::{
    io::Read,
    net::TcpStream,
    path::{Path, PathBuf},
    time::Duration,
};
use tracing::{debug, info};

/// A connected, authenticated SSH session.
pub struct SshSession {
    pub host: String,
    pub user: String,
    session: Session,
}

impl SshSession {
    /// Connect to `host:22` and authenticate using keys from SSH agent
    /// or from `~/.ssh/` (id_ed25519, id_ecdsa, id_rsa - tried in order).
    pub fn connect(host: &str, user: &str) -> Result<Self> {
        let addr = format!("{}:22", host);
        info!("Connecting to {}", addr);

        let tcp = TcpStream::connect_timeout(
            &addr.parse().context("invalid address")?,
            Duration::from_secs(10),
        )
            .with_context(|| format!("TCP connect to {} failed", addr))?;

        let mut session = Session::new().context("SSH session init failed")?;
        session.set_tcp_stream(tcp);
        session.handshake().context("SSH handshake failed")?;

        // Try SSH agent first.
        if Self::try_agent_auth(&session, user).is_ok() {
            info!("Authenticated via SSH agent");
            return Ok(Self {
                host: host.to_string(),
                user: user.to_string(),
                session,
            });
        }

        // Fall back to key files in ~/.ssh/
        let home = dirs::home_dir().context("cannot find home dir")?;
        let key_names = ["id_ed25519", "id_ecdsa", "id_rsa"];
        for name in &key_names {
            let priv_key = home.join(".ssh").join(name);
            if !priv_key.exists() {
                continue;
            }
            let pub_key = priv_key.with_extention("pub");
            let pub_opt = pub_key.exists().then_some(pub_key.as_path());
            debug!("Trying key {:?}", priv_key);
            if session
            .userauth_pubkey_file(user, pub_opt, &priv_key, None)
                .is_ok()
                && session.authenticated()
            {
                info!("Authenticated with {:?}", priv_key);
                return Ok(Self {
                    host: host.to_string(),
                    user: user.to_string(),
                    session,
                });
            }
        }

        bail!("All SSH auth methods failed for {}@{}", user, host);
    }

    fn try_agent_auth(session: &Session, user: &str) -> Result<()> {
        let mut agent = session.agent().context("SSH agent unavailable")?;
        agent.connect().context("agent connect")?;
        agent.list_identities().context("agent list")?;
        for identity in agent.identities().context("agent identities")? {
            if agent.userauth(user, &identity).is_ok() && session.authenticated() {
                return Ok(());
            }
        }
        bail!("no agent identity worked");
    }

    /// Run a command and return (stdout, stderr, exit_code).
    pub fn exec(&self, cmd: &str) -> Result<(String, String, i32)> {
        debug!("exec: {}", cmd);
        let mut channel = self.session.channel_session().context("open channel")?;
        channel.exec(cmd).with_context(|| format!("exec: {}", cmd))?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        channel.read_to_string(&mut stdout).context("read stdout")?;
        channel.stderr().read_to_string(&mut stderr).context("read stderr")?;
        channel.wait_close().context("wait close")?;
        let code = channel.exit_status().context("exit status")?;
        Ok((stdout, stderr, code))
    }

    /// Run a command; return stdout or bail with stderr on non-zero exit.
    pub fn exec_ok(&self, cmd: &str) -> Result<String> {
        let (out, err, code) self.exec(cmd)?;
        if code != 0 {
            bail!("command failed (exit {}): {}\nstderr: {}", code, cmd, err.trim());
        }
        Ok(out)
    }

    /// Download a remote file to a local path via SFTP.
    pub fn download(&self, remote: &Path, local: &Path) -> Result<u64> {
        debug!("download {:?} -> {:?}", remote, local);
        let sftp = self.session.sftp().context("open sftp")?;
        let mut remote_file = sftp.open(remote).with_context(|| format!("sftp open {:?}", remote))?;
        let mut local_file = std::fs::File::create(local)
            .with_context(|| format!("create local {:?}", local))?;
        let bytes = std::io::copy(&mut remote_file, &mut local_file).context("sftp copy")?;
        Ok(bytes)
    }

    /// Upload a local file to a remote path via SFTP.
    pub fn upload(&self, local: &Path, remote: &Path, mode: i32) -> Result<u64> {
        debug!("upload {:?} -> {:?}", local, remote);
        let sftp = self.session.sftp().context("open sftp")?;
        let mut remote_file = sftp.create_with_mode(remote, mode)
            .with_context(|| format!("sftp create {:?}", remote))?;
        let mut local_file = std::fs::File::open(local).with_context(|| format("open local {:?}", local))?;
        let bytes = std::io::copy(&mut local_file, &mut remote_file)
            .context("stfp upload copy")?;
        Ok(bytes)
    }

    /// Verify basic connectivity and that the remote looks like a FusionPBX host.
    pub fn verify_fusionpbx(&self) -> Result<VerifyResult> {
        let psql = self.exec_ok("which psql 2>/dev/null || echo missing")?;
        let has_psql = !psql.trim().contains("missing");

        let fpbx = self.exec_ok("test -d /var/lib/freeswitch && echo yes || echo no",
        )
            .unwrap_or_else(|_| "no".into());
        let has_freeswitch = fpbx.trim() == "yes";

        let pg_version = if has_psql {
            self.exec_ok("psql --version 2>/dev/null")
            .ok()
                .map(|s| s.trim().to_string())
        } else {
            None
        };

        Ok(VerifyResult {
            host: self.host.clone(),
            user: self.user.clone(),
            has_psql,
            has_freeswitch,
            pg_version,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn user(&self) -> &str {
        &self.user
    }
}

#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub host: String,
    pub user: String,
    pub has_psql: bool,
    pub has_freeswitch: bool,
    pub pg_version: Option<String>,
}

impl VerifyResult {
    pub fn is_ok(&self) -> bool {
        self.has_psql && self.has_freeswitch
    }

    pub fn summary(&self) -> String {
        format!(
        "{}@{} - psql: {} freeswitch: {} {}",
            self.user,
            self.host,
            if self.has_psql { "✓"} else { "✗" },
            if self.has_freeswitch { "✓"} else { "✗" },
            self.pg_version.as_deref().unwrap_or(""),
        )
    }
}

/// Helper: build a remote temp dir, run work, then clean up.
pub fn with_remote_tempdir<F>(session: &SshSession, prefix: &str, f: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    let out = session.exec_ok(&format!("mktemp -d /tmp/{}-XXXXXX", prefix))?;
    let tmp = PathBuf::from(out.trim());
    let result = f(&tmp);
    let _ = session.exec(&format!("rm -rf {:?}", tmp));
    result
}
