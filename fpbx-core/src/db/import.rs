use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

use crate::ssh::SshSession;

use super::DomainRename;
use super::adapt::adapt_sql_for_dest;
use super::uuid_remap::{query_uuid_schemas, remap_domain_uuids};

/// Import a gzipped SQL file into PostgreSQL on the destination server.
///
/// Automatically adapts the SQL to the destination schema using column intersection:
/// columns present in the bundle but absent from the destination are silently dropped,
/// and columns present only on the destination receive their default values.
pub fn import_domain_sql(
    session: &SshSession,
    local_sql_gz: &Path,
    rename: Option<&DomainRename>,
    progress: &mut dyn FnMut(&str),
) -> Result<()> {
    progress("Adapting SQL to destination schema…");
    let mut adapted_sql = adapt_sql_for_dest(local_sql_gz, session, progress)?;

    if let Some(r) = rename {
        // Step 1: swap domain UUID and name throughout the SQL.
        progress(&format!(
            "Renaming domain {} → {}…",
            r.src_name, r.dest_name
        ));
        adapted_sql = adapted_sql.replace(&r.src_uuid, &r.dest_uuid);
        adapted_sql = adapted_sql.replace(&r.src_name, &r.dest_name);

        // Step 2: query destination schema to find UUID PK/FK columns per table.
        progress("Querying destination UUID schema…");
        let schemas = query_uuid_schemas(session);

        // Step 3: regenerate every per-record PK UUID and propagate the mapping to
        // FK columns within the dump.  This guarantees no PK conflicts regardless
        // of whether the original domain already exists on the target server.
        progress("Remapping record UUIDs (schema-aware)…");
        adapted_sql = remap_domain_uuids(&adapted_sql, &r.dest_uuid, &schemas);
    }

    progress("Uploading adapted SQL to destination…");
    let remote_path = "/tmp/fpbx-import.sql.gz";
    let adapted_gz = {
        use flate2::{Compression, write::GzEncoder};
        use std::io::Write;
        let mut buf = Vec::new();
        let mut gz = GzEncoder::new(&mut buf, Compression::best());
        gz.write_all(adapted_sql.as_bytes())
            .context("gz adapted SQL")?;
        gz.finish().context("gz finish")?;
        buf
    };
    let tmp = local_sql_gz.with_extension("adapted.gz");
    std::fs::write(&tmp, &adapted_gz).context("write adapted gz")?;
    session
        .upload(&tmp, Path::new(remote_path), 0o600)
        .context("upload adapted SQL")?;
    let _ = std::fs::remove_file(&tmp);

    progress("Importing SQL into PostgreSQL…");
    let import_cmd = format!(
        "zcat {} | sudo -u postgres psql -v ON_ERROR_STOP=1 -d fusionpbx 2>&1",
        remote_path
    );
    let (out, err, code) = session.exec(&import_cmd)?;
    let combined = format!("{}{}", out, err).trim().to_string();
    if code != 0 {
        anyhow::bail!("psql import failed (exit {}):\n{}", code, combined);
    }
    if !combined.is_empty() {
        info!("psql import output: {}", combined);
    }

    let _ = session.exec(&format!("rm -f {}", remote_path));
    info!("SQL import complete");
    Ok(())
}
