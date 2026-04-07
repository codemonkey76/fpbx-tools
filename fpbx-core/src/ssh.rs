use anyhow::{bail, Context, Result};
use ssh2::Session;
use std::{
    io::Read,
    net::TcpStream,
    path::Path,
    time::Duration,
};
use tracing::{debug, info};
use crate::version::FpbxVersion;

/// A connected, authenticated SSH session.
pub struct SshSession {
    pub host: String,
    pub user: String,
    session: Session,
}

impl SshSession {
    /// Connect to `host:22` and authenticate using keys from the SSH agent
    /// or from `~/.ssh/` (id_ed25519, id_ecdsa, id_rsa — tried in order).
    pub fn connect(host: &str, user: &str) -> Result<Self> {
        let addr = format!("{}:22", host);
        info!("Connecting to {}", addr);

        use std::net::ToSocketAddrs;
        let sock_addr = addr.to_socket_addrs().with_context(|| format!("could not resolve host: {}", addr))?
        .next()
        .context("no addresses found for host")?;

        let tcp = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(10))
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
            let pub_key = priv_key.with_extension("pub");
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
        let (out, err, code) = self.exec(cmd)?;
        if code != 0 {
            bail!("command failed (exit {}): {}\nstderr: {}", code, cmd, err.trim());
        }
        Ok(out)
    }

    /// Download a remote file to a local path via SFTP.
    pub fn download(&self, remote: &Path, local: &Path) -> Result<u64> {
        debug!("download {:?} -> {:?}", remote, local);
        let sftp = self.session.sftp().context("open sftp")?;
        let mut remote_file = sftp
            .open(remote)
            .with_context(|| format!("sftp open {:?}", remote))?;
        let mut local_file = std::fs::File::create(local)
            .with_context(|| format!("create local {:?}", local))?;
        let bytes = std::io::copy(&mut remote_file, &mut local_file)
            .context("sftp copy")?;
        Ok(bytes)
    }

    /// Upload a local file to a remote path via SFTP.
    pub fn upload(&self, local: &Path, remote: &Path) -> Result<u64> {
        debug!("upload {:?} -> {:?}", local, remote);
        let sftp = self.session.sftp().context("open sftp")?;
        let mut remote_file = sftp
            .create(remote)
            .with_context(|| format!("sftp create {:?}", remote))?;
        let mut local_file =
            std::fs::File::open(local).with_context(|| format!("open local {:?}", local))?;
        let bytes = std::io::copy(&mut local_file, &mut remote_file)
            .context("sftp upload copy")?;
        Ok(bytes)
    }

    /// Verify basic connectivity and that the remote looks like a FusionPBX host.
    /// Also detects the FusionPBX deployment version.
    pub fn verify_fusionpbx(&self) -> Result<VerifyResult> {
        let psql = self.exec_ok("which psql 2>/dev/null || echo missing")?;
        let has_psql = !psql.trim().contains("missing");

        let fpbx = self
            .exec_ok(
                "test -d /var/lib/freeswitch && echo yes || echo no",
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

        let fpbx_version = if has_psql && has_freeswitch {
            crate::version::detect_version(self).ok()
        } else {
            None
        };

        Ok(VerifyResult {
            host: self.host.clone(),
            user: self.user.clone(),
            has_psql,
            has_freeswitch,
            pg_version,
            fpbx_version,
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
    pub fpbx_version: Option<FpbxVersion>,
}

impl VerifyResult {
    pub fn is_ok(&self) -> bool {
        self.has_psql && self.has_freeswitch
    }

    pub fn summary(&self) -> String {
        format!(
            "{}@{} — psql:{} freeswitch:{} {} {}",
            self.user,
            self.host,
            if self.has_psql { "✓" } else { "✗" },
            if self.has_freeswitch { "✓" } else { "✗" },
            self.pg_version.as_deref().unwrap_or(""),
            self.fpbx_version.as_ref().map(|v| v.label()).unwrap_or_default(),
        )
    }
}

// ── SSH config helpers ──────────────────────────────────────────────────────

/// A single entry parsed from `~/.ssh/config`.
#[derive(Debug, Clone)]
pub struct SshHostEntry {
    pub hostname: String,
    pub user: String,
}

/// Parse `~/.ssh/config` and return a map of `alias → SshHostEntry`.
/// Wildcard entries (`Host *`) are ignored.
pub fn parse_ssh_config() -> std::collections::HashMap<String, SshHostEntry> {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    let config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".ssh")
        .join("config");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return map;
    };
    let mut current_alias: Option<String> = None;
    let mut current_hostname: Option<String> = None;
    let mut current_user: Option<String> = None;

    let flush = |map: &mut HashMap<String, SshHostEntry>,
                 alias: &mut Option<String>,
                 hostname: &mut Option<String>,
                 user: &mut Option<String>| {
        if let (Some(a), Some(h), Some(u)) = (alias.take(), hostname.take(), user.take()) {
            map.insert(a.to_lowercase(), SshHostEntry { hostname: h, user: u });
        }
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = match line.split_once(|c: char| c.is_whitespace()) {
            Some(pair) => (pair.0.to_lowercase(), pair.1.trim().to_string()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                flush(&mut map, &mut current_alias, &mut current_hostname, &mut current_user);
                if !val.contains('*') {
                    current_alias = Some(val);
                }
            }
            "hostname" => current_hostname = Some(val),
            "user" => current_user = Some(val),
            _ => {}
        }
    }
    flush(&mut map, &mut current_alias, &mut current_hostname, &mut current_user);
    map
}

/// Return the current OS user (falls back to `"root"`).
pub fn whoami_current_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "root".to_string())
}

/// Resolve a host alias against a parsed SSH config map.
/// Returns the `HostName` value if the alias is found, otherwise returns `input` trimmed.
pub fn resolve_host(input: &str, hosts: &std::collections::HashMap<String, SshHostEntry>) -> String {
    let key = input.trim().to_lowercase();
    hosts
        .get(&key)
        .map(|e| e.hostname.clone())
        .unwrap_or_else(|| input.trim().to_string())
}
