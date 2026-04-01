use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tar::Builder;
use flate2::{write::GzEncoder, read::GzDecoder, Compression};
use tracing::info;

use crate::domain::{DomainTableCounts, FpbxDomain};

pub const BUNDLE_EXT: &str = "fpbx";
pub const MANIFEST_NAME: &str = "manifest.json";
pub const DB_DUMP_NAME: &str = "db.sql.gz";
pub const FILES_TAR_NAME: &str = "files.tar.gz";
pub const CHECKSUM_NAME: &str = "checksum.sha256";

/// The manifest embedded in every .fpbx bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub version: u8,
    pub created_at: DateTime<Utc>,
    pub source_host: String,
    pub domain: FpbxDomain,
    pub table_counts: DomainTableCounts,
    pub file_paths: Vec<String>,
    pub db_dump_bytes: u64,
    pub files_tar_bytes: u64,
}

impl BundleManifest {
    pub fn new(
        source_host: &str,
        domain: FpbxDomain,
        table_counts: DomainTableCounts,
        file_paths: Vec<String>,
        db_dump_bytes: u64,
        files_tar_bytes: u64,
    ) -> Self {
        Self {
            version: 1,
            created_at: Utc::now(),
            source_host: source_host.to_string(),
            domain,
            table_counts,
            file_paths,
            db_dump_bytes,
            files_tar_bytes,
        }
    }
}

/// Build a .fpbx bundle from its component files in `staging_dir`.
/// Returns the path to the final bundle file.
pub fn create_bundle(
    manifest: &BundleManifest,
    staging_dir: &Path,
    output_dir: &Path,
    progress: &mut dyn FnMut(&str),
) -> Result<PathBuf> {
    fs::create_dir_all(output_dir).context("create output dir")?;

    let timestamp = manifest.created_at.format("%Y%m%d-%H%M%S");
    let safe_name = manifest.domain.domain_name.replace('.', "_");
    let bundle_name = format!("{}-{}.{}", safe_name, timestamp, BUNDLE_EXT);
    let bundle_path = output_dir.join(&bundle_name);

    progress(&format!("Creating bundle {}…", bundle_name));

    // Write manifest.json into staging.
    let manifest_path = staging_dir.join(MANIFEST_NAME);
    let manifest_json = serde_json::to_string_pretty(manifest).context("serialize manifest")?;
    fs::write(&manifest_path, &manifest_json).context("write manifest")?;

    // Build outer tar containing: manifest.json, db.sql.gz, files.tar.gz, checksum.sha256.
    let bundle_file = fs::File::create(&bundle_path).context("create bundle file")?;
    let gz_enc = GzEncoder::new(bundle_file, Compression::best());
    let mut tar = Builder::new(gz_enc);

    let components = [MANIFEST_NAME, DB_DUMP_NAME, FILES_TAR_NAME];
    let mut hasher = Sha256::new();

    for name in &components {
        let path = staging_dir.join(name);
        if !path.exists() {
            continue;
        }
        progress(&format!("Packing {}…", name));
        tar.append_path_with_name(&path, name)
            .with_context(|| format!("pack {}", name))?;

        // Feed into checksum.
        let mut f = fs::File::open(&path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        hasher.update(&buf);
    }

    // Write checksum file.
    let checksum = hex::encode(hasher.finalize());
    let checksum_path = staging_dir.join(CHECKSUM_NAME);
    fs::write(&checksum_path, format!("{}\n", checksum))?;
    tar.append_path_with_name(&checksum_path, CHECKSUM_NAME)
        .context("pack checksum")?;

    tar.finish().context("finalize bundle tar")?;

    info!("Bundle created: {:?}", bundle_path);
    Ok(bundle_path)
}

/// Extract and validate a .fpbx bundle. Returns the manifest and staging dir.
pub fn open_bundle(bundle_path: &Path, staging_dir: &Path) -> Result<BundleManifest> {
    fs::create_dir_all(staging_dir).context("create staging dir")?;

    info!("Opening bundle {:?}", bundle_path);
    let file = fs::File::open(bundle_path).context("open bundle file")?;
    let gz = GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.unpack(staging_dir).context("unpack bundle")?;

    // Verify checksum.
    let checksum_path = staging_dir.join(CHECKSUM_NAME);
    let expected = fs::read_to_string(&checksum_path)
        .context("read checksum")?
        .trim()
        .to_string();

    let mut hasher = Sha256::new();
    for name in &[MANIFEST_NAME, DB_DUMP_NAME, FILES_TAR_NAME] {
        let path = staging_dir.join(name);
        if !path.exists() {
            continue;
        }
        let mut f = fs::File::open(&path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        hasher.update(&buf);
    }
    let actual = hex::encode(hasher.finalize());
    if actual != expected {
        anyhow::bail!(
            "Bundle checksum mismatch!\n  expected: {}\n  actual:   {}",
            expected,
            actual
        );
    }

    // Parse manifest.
    let manifest_path = staging_dir.join(MANIFEST_NAME);
    let manifest_json = fs::read_to_string(&manifest_path).context("read manifest")?;
    let manifest: BundleManifest =
        serde_json::from_str(&manifest_json).context("parse manifest")?;

    info!("Bundle valid — domain: {}", manifest.domain.domain_name);
    Ok(manifest)
}

/// List .fpbx bundles in the default backup directory.
pub fn list_bundles(dir: &Path) -> Result<Vec<(PathBuf, BundleManifest)>> {
    let mut bundles = Vec::new();
    if !dir.exists() {
        return Ok(bundles);
    }
    for entry in fs::read_dir(dir).context("read backup dir")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some(BUNDLE_EXT) {
            let tmp = tempdir(&path)?;
            if let Ok(manifest) = open_bundle(&path, &tmp) {
                bundles.push((path, manifest));
            }
            let _ = fs::remove_dir_all(&tmp);
        }
    }
    // Sort newest first.
    bundles.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
    Ok(bundles)
}

/// Default backup output directory.
pub fn default_backup_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".fpbx")
        .join("backups")
}

/// Default staging directory (for temp work).
pub fn default_staging_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".fpbx")
        .join("staging")
}

fn tempdir(hint: &Path) -> Result<PathBuf> {
    let stem = hint
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("fpbx");
    let p = std::env::temp_dir().join(format!("fpbx-open-{}", stem));
    Ok(p)
}
