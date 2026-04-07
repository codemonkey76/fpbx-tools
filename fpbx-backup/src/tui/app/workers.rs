use anyhow::Result;
use fpbx_core::{
    bundle::{BundleManifest, create_bundle, default_staging_dir},
    db::export_domain_sql_v2,
    domain::{DomainFilePaths, FpbxDomain, count_domain_rows},
    ssh::SshSession,
    version::detect_version,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::types::WorkerState;

pub(super) fn run_backup(
    host: String,
    user: String,
    domain: FpbxDomain,
    output_dir: PathBuf,
    progress: &mut dyn FnMut(&str),
) -> Result<PathBuf> {
    progress("Connecting to source server…");
    let session = SshSession::connect(&host, &user)?;

    let staging = default_staging_dir().join(&domain.domain_uuid);
    std::fs::create_dir_all(&staging)?;

    // Count rows.
    progress("Counting domain records…");
    let table_counts = count_domain_rows(&session, &domain.domain_uuid)?;

    // Export SQL.
    progress("Exporting database records…");
    let sql_path = staging.join("db.sql.gz");
    let db_bytes = export_domain_sql_v2(&session, &domain.domain_uuid, &sql_path, progress)?;

    // Export files.
    progress("Discovering domain file paths…");
    let file_paths_spec = DomainFilePaths::for_domain(&domain.domain_name);
    let existing_paths = file_paths_spec.existing(&session);

    progress("Archiving voicemail + recordings…");
    let files_tar_path = staging.join("files.tar.gz");
    let files_bytes = export_domain_files(&session, &existing_paths, &files_tar_path, progress)?;

    // Detect source server version.
    progress("Detecting server version…");
    let source_version = detect_version(&session).ok();

    // Build manifest.
    let manifest = BundleManifest::new(
        &host,
        domain,
        table_counts,
        existing_paths,
        db_bytes,
        files_bytes,
        source_version,
    );

    // Create bundle.
    progress("Assembling .fpbx bundle…");
    let bundle_path = create_bundle(&manifest, &staging, &output_dir, progress)?;

    // Cleanup staging.
    let _ = std::fs::remove_dir_all(&staging);

    Ok(bundle_path)
}

pub(super) fn export_domain_files(
    session: &SshSession,
    paths: &[String],
    local_tar: &std::path::Path,
    progress: &mut dyn FnMut(&str),
) -> Result<u64> {
    if paths.is_empty() {
        // Create empty tar.gz.
        let f = std::fs::File::create(local_tar)?;
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::best());
        tar::Builder::new(gz).finish()?;
        return Ok(0);
    }

    let remote_tar = "/tmp/fpbx-files.tar.gz";
    let path_args = paths
        .iter()
        .map(|p| format!("'{}'", p))
        .collect::<Vec<_>>()
        .join(" ");

    progress("Compressing remote files…");
    let cmd = format!("tar czf {} {} 2>/dev/null || true", remote_tar, path_args);
    session.exec(&cmd)?;

    progress("Downloading file archive…");
    let bytes = session.download(std::path::Path::new(remote_tar), local_tar)?;
    let _ = session.exec(&format!("rm -f {}", remote_tar));

    Ok(bytes)
}

/// Drives the per-domain backup loop, updating worker state throughout.
pub(super) fn run_backup_worker(
    host: String,
    user: String,
    domains: Vec<FpbxDomain>,
    output_dir: PathBuf,
    wstate: Arc<Mutex<WorkerState>>,
) {
    let n = domains.len();
    for (idx, domain) in domains.into_iter().enumerate() {
        {
            let mut w = wstate.lock().unwrap();
            w.progress = idx as f64 / n as f64;
            w.log.push(format!(
                "--- {} ({}/{}) ---",
                domain.domain_name,
                idx + 1,
                n
            ));
            w.current_task = format!("Backing up {}…", domain.domain_name);
        }

        let ws = wstate.clone();
        let mut progress = move |msg: &str| {
            let mut w = ws.lock().unwrap();
            w.log.push(msg.to_string());
            w.current_task = msg.to_string();
        };

        match run_backup(
            host.clone(),
            user.clone(),
            domain,
            output_dir.clone(),
            &mut progress,
        ) {
            Ok(path) => {
                let mut w = wstate.lock().unwrap();
                w.log.push(format!("✓ Bundle saved: {}", path.display()));
                w.bundle_paths.push(path);
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
