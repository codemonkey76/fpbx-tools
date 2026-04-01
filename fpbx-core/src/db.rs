use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

use crate::{domain::DOMAIN_TABLES, ssh::SshSession};

/// Export all domain-scoped data as a gzipped SQL dump.
/// The dump is written to `local_path` on the jump box.
///
/// Strategy:
///   1. Build a combined SQL script using COPY TO STDOUT with a WHERE clause.
///   2. Stream it through gzip on the remote.
///   3. Download via SFTP.
pub fn export_domain_sql(
    session: &SshSession,
    domain_uuid: &str,
    local_path: &Path,
    progress: &mut dyn FnMut(&str),
) -> Result<()> {
    info!("Exporting SQL for domain {}", domain_uuid);

    // Build remote temp file path.
    let remote_sql = format!("/tmp/fpbx-export-{}.sql.gz", &domain_uuid[..8]);

    // Build a pg_dump using --table and inject row filtering via a wrapper script.
    // We use a COPY approach: for each table emit the schema then COPY data.
    let table_flags: String = DOMAIN_TABLES
        .iter()
        .map(|t| format!("-t {}", t))
        .collect::<Vec<_>>()
        .join(" ");

    // Step 1: dump schema-only (DDL) for all domain tables.
    progress("Dumping table schemas…");
    let schema_cmd = format!(
        "sudo -u postgres pg_dump -d fusionpbx --schema-only {} | gzip > {}",
        table_flags, remote_sql
    );
    session
        .exec_ok(&schema_cmd)
        .context("pg_dump schema failed")?;

    // Step 2: for each table, append filtered COPY data.
    // We append to the same gz by streaming through gzip -c in append mode.
    for table in DOMAIN_TABLES {
        progress(&format!("Exporting {}…", table));

        let copy_sql = format!(
            r#"COPY (SELECT * FROM {} WHERE domain_uuid = '{}') TO STDOUT"#,
            table, domain_uuid
        );
        // Use psql -c "COPY ..." and append gzipped output.
        // We separate schema and data files; reassemble at restore.
        let append_cmd = format!(
            r#"sudo -u postgres psql -d fusionpbx -c "{}" 2>/dev/null | gzip >> {} || true"#,
            copy_sql.replace('"', "\\\""),
            remote_sql
        );
        session
            .exec(&append_cmd)
            .context("copy table failed")?;
    }

    // Step 3: download.
    progress("Downloading SQL dump…");
    session
        .download(Path::new(&remote_sql), local_path)
        .context("download SQL dump")?;

    // Cleanup remote temp.
    let _ = session.exec(&format!("rm -f {}", remote_sql));

    info!("SQL export complete -> {:?}", local_path);
    Ok(())
}

/// Proper per-table export using separate schema + filtered COPY data.
/// Produces a plain SQL file suitable for psql import.
pub fn export_domain_sql_v2(
    session: &SshSession,
    domain_uuid: &str,
    local_path: &Path,
    progress: &mut dyn FnMut(&str),
) -> Result<u64> {
    let remote_dir = format!("/tmp/fpbx-{}", &domain_uuid[..8]);
    session.exec_ok(&format!("mkdir -p {}", remote_dir))?;

    let mut sql_parts: Vec<String> = vec![
        "SET client_min_messages = warning;".into(),
        "BEGIN;".into(),
    ];

    for table in DOMAIN_TABLES {
        progress(&format!("Exporting {}…", table));

        // Check if table exists first.
        let exists_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name='{}')\" 2>/dev/null",
            table
        );
        let exists = session
            .exec_ok(&exists_cmd)
            .unwrap_or_else(|_| "f".into());
        if exists.trim() != "t" {
            continue;
        }

        // Check if the table has a domain_uuid column.
        let col_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"SELECT EXISTS (SELECT 1 FROM information_schema.columns \
              WHERE table_name='{}' AND column_name='domain_uuid')\" 2>/dev/null",
            table
        );
        let has_col = session.exec_ok(&col_cmd).unwrap_or_else(|_| "f".into());

        let where_clause = if has_col.trim() == "t" {
            format!(" WHERE domain_uuid = '{}'", domain_uuid)
        } else {
            String::new()
        };

        // Get column list.
        let cols_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"SELECT string_agg(column_name, ',' ORDER BY ordinal_position) \
              FROM information_schema.columns WHERE table_name='{}'\" 2>/dev/null",
            table
        );
        let cols = session
            .exec_ok(&cols_cmd)
            .unwrap_or_default();
        let cols = cols.trim();
        if cols.is_empty() {
            continue;
        }

        // Export data as INSERT statements.
        let copy_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"COPY (SELECT {} FROM {}{}) TO STDOUT WITH CSV HEADER\" 2>/dev/null",
            cols, table, where_clause
        );
        let csv_data = session.exec_ok(&copy_cmd).unwrap_or_default();

        // Convert CSV to INSERT statements in SQL.
        let mut lines = csv_data.lines();
        if let Some(header) = lines.next() {
            let columns: Vec<&str> = header.split(',').collect();
            sql_parts.push(format!("-- Table: {}", table));
            for row in lines {
                // Build INSERT — values already CSV-quoted.
                sql_parts.push(format!(
                    "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT DO NOTHING;",
                    table,
                    columns.join(", "),
                    row
                ));
            }
        }
    }

    sql_parts.push("COMMIT;".into());
    let full_sql = sql_parts.join("\n");

    // Write to a local temp file, compress, then save.
    progress("Compressing SQL…");
    let raw_path = local_path.with_extension(""); // strip .gz temporarily
    std::fs::write(&raw_path, &full_sql).context("write raw SQL")?;

    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    let out_file = std::fs::File::create(local_path).context("create gz")?;
    let mut gz = GzEncoder::new(out_file, Compression::best());
    gz.write_all(full_sql.as_bytes()).context("gz write")?;
    gz.finish().context("gz finish")?;

    let size = std::fs::metadata(local_path)?.len();
    let _ = std::fs::remove_file(&raw_path);

    // Clean up remote temp.
    let _ = session.exec(&format!("rm -rf {}", remote_dir));

    info!("SQL export v2 complete -> {:?} ({} bytes)", local_path, size);
    Ok(size)
}

/// Import a gzipped SQL file into PostgreSQL on the destination server.
pub fn import_domain_sql(
    session: &SshSession,
    local_sql_gz: &Path,
    progress: &mut dyn FnMut(&str),
) -> Result<()> {
    let remote_path = "/tmp/fpbx-import.sql.gz";

    progress("Uploading SQL dump to destination…");
    session
        .upload(local_sql_gz, Path::new(remote_path), 0o600)
        .context("upload SQL dump")?;

    progress("Importing SQL into PostgreSQL…");
    let import_cmd = format!(
        "zcat {} | sudo -u postgres psql -d fusionpbx 2>&1",
        remote_path
    );
    let (out, err, code) = session.exec(&import_cmd)?;
    if code != 0 {
        anyhow::bail!("psql import failed (exit {}):\n{}\n{}", code, out, err);
    }

    let _ = session.exec(&format!("rm -f {}", remote_path));
    info!("SQL import complete");
    Ok(())
}
