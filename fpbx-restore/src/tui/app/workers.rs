use fpbx_core::{
    bundle::{BundleManifest, DB_DUMP_NAME, FILES_TAR_NAME, default_staging_dir, open_bundle},
    db::{DomainRename, import_domain_sql},
    ssh::SshSession,
    version::{FpbxVersion, check_compat},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::types::WorkerState;

pub(super) fn build_rename(manifest: &BundleManifest, dest_name: &str) -> Option<DomainRename> {
    let dest_name = dest_name.trim();
    if dest_name.is_empty() || dest_name == manifest.domain.domain_name {
        return None;
    }
    let new_uuid = uuid::Uuid::new_v4().to_string();
    Some(DomainRename {
        src_uuid: manifest.domain.domain_uuid.clone(),
        src_name: manifest.domain.domain_name.clone(),
        dest_uuid: new_uuid,
        dest_name: dest_name.to_string(),
    })
}

pub(super) fn run_restore(
    host: String,
    user: String,
    bundle_path: PathBuf,
    dest_version: Option<FpbxVersion>,
    rename: Option<&DomainRename>,
    progress: &mut dyn FnMut(&str),
) -> anyhow::Result<()> {
    let stem = bundle_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bundle");
    let staging = default_staging_dir().join(stem);
    std::fs::create_dir_all(&staging)?;

    progress("Opening and verifying bundle…");
    let manifest = open_bundle(&bundle_path, &staging)?;

    // Version compatibility check.
    if let (Some(src_v), Some(dst_v)) = (&manifest.source_version, &dest_version) {
        let compat = check_compat(src_v, dst_v);
        if !compat.is_ok() {
            anyhow::bail!("{}", compat.status_line());
        }
        progress(&compat.status_line());
    } else {
        progress("Version info unavailable — proceeding with column intersection");
    }

    progress("Connecting to destination server…");
    let session = SshSession::connect(&host, &user)?;

    let sql_path = staging.join(DB_DUMP_NAME);
    import_domain_sql(&session, &sql_path, rename, progress)?;

    let files_tar = staging.join(FILES_TAR_NAME);
    if files_tar.exists() && files_tar.metadata().map(|m| m.len()).unwrap_or(0) > 100 {
        progress("Uploading file archive to destination…");
        let remote_tar = "/tmp/fpbx-restore-files.tar.gz";
        session.upload(&files_tar, std::path::Path::new(remote_tar), 0o600)?;
        progress("Creating destination directories…");
        for dir in &[
            "/var/lib/freeswitch/storage/voicemail/default",
            "/var/lib/freeswitch/recordings",
            "/etc/freeswitch/dialplan",
            "/etc/freeswitch/directory",
        ] {
            let _ = session.exec(&format!("sudo mkdir -p {}", dir));
        }
        progress("Extracting files on destination server…");
        session.exec_ok(&format!("sudo tar xzf {} -C /", remote_tar))?;
        let _ = session.exec(&format!("rm -f {}", remote_tar));
    }

    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// Drives the per-bundle restore loop, updating worker state throughout.
pub(super) fn run_restore_worker(
    host: String,
    user: String,
    dest_version: Option<FpbxVersion>,
    bundles_with_rename: Vec<(PathBuf, Option<DomainRename>)>,
    wstate: Arc<Mutex<WorkerState>>,
) {
    let n = bundles_with_rename.len();
    for (idx, (bundle_path, rename)) in bundles_with_rename.into_iter().enumerate() {
        {
            let mut w = wstate.lock().unwrap();
            w.progress = idx as f64 / n as f64;
            w.log.push(format!(
                "--- Bundle {}/{}: {} ---",
                idx + 1,
                n,
                bundle_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
            w.current_task = format!("Restoring bundle {}/{}…", idx + 1, n);
        }

        let ws = wstate.clone();
        let mut progress = move |msg: &str| {
            let mut w = ws.lock().unwrap();
            w.log.push(msg.to_string());
            w.current_task = msg.to_string();
        };

        match run_restore(
            host.clone(),
            user.clone(),
            bundle_path,
            dest_version.clone(),
            rename.as_ref(),
            &mut progress,
        ) {
            Ok(()) => {
                let mut w = wstate.lock().unwrap();
                w.log.push("✓ Restore complete".to_string());
            }
            Err(e) => {
                let mut w = wstate.lock().unwrap();
                w.error = Some(e.to_string());
                w.done = true;
                return;
            }
        }
    }

    let mut w = wstate.lock().unwrap();
    w.progress = 1.0;
    w.done = true;
}
