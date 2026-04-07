use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

use crate::{domain::DOMAIN_TABLES, ssh::SshSession};

/// Export all domain-scoped tables as filtered COPY FROM stdin blocks,
/// wrapped in `BEGIN` / `COMMIT` and compressed with gzip.
/// Returns the number of compressed bytes written to `local_path`.
pub fn export_domain_sql(
    session: &SshSession,
    domain_uuid: &str,
    local_path: &Path,
    progress: &mut dyn FnMut(&str),
) -> Result<u64> {
    let remote_dir = format!("/tmp/fpbx-{}", &domain_uuid[..8]);
    session.exec_ok(&format!("mkdir -p {}", remote_dir))?;

    let mut sql_parts: Vec<String> =
        vec!["SET client_min_messages = warning;".into(), "BEGIN;".into()];

    for table in DOMAIN_TABLES {
        progress(&format!("Exporting {}…", table));

        // Check if table exists first.
        let exists_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name='{}')\" 2>/dev/null",
            table
        );
        let exists = session.exec_ok(&exists_cmd).unwrap_or_else(|_| "f".into());
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

        // Get column list in ordinal order.
        let cols_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"SELECT string_agg(column_name, ',' ORDER BY ordinal_position) \
              FROM information_schema.columns WHERE table_name='{}'\" 2>/dev/null",
            table
        );
        let cols = session.exec_ok(&cols_cmd).unwrap_or_default();
        let cols = cols.trim();
        if cols.is_empty() {
            continue;
        }

        // Export data as COPY FROM stdin blocks (handles all types and NULLs correctly).
        let copy_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"COPY (SELECT {} FROM {}{}) TO STDOUT WITH CSV\" 2>/dev/null",
            cols, table, where_clause
        );
        let csv_data = session.exec_ok(&copy_cmd).unwrap_or_default();

        let data_lines: Vec<&str> = csv_data.lines().collect();
        if !data_lines.is_empty() {
            sql_parts.push(format!("-- Table: {}", table));
            sql_parts.push(format!("COPY {} ({}) FROM stdin WITH CSV;", table, cols));
            for row in &data_lines {
                sql_parts.push(row.to_string());
            }
            sql_parts.push("\\.".into());
        }
    }

    sql_parts.push("COMMIT;".into());
    let full_sql = sql_parts.join("\n");

    // Compress and save locally — no remote temp files needed.
    progress("Compressing SQL…");
    let raw_path = local_path.with_extension(""); // strip .gz temporarily
    std::fs::write(&raw_path, &full_sql).context("write raw SQL")?;

    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;
    let out_file = std::fs::File::create(local_path).context("create gz")?;
    let mut gz = GzEncoder::new(out_file, Compression::best());
    gz.write_all(full_sql.as_bytes()).context("gz write")?;
    gz.finish().context("gz finish")?;

    let size = std::fs::metadata(local_path)?.len();
    let _ = std::fs::remove_file(&raw_path);

    let _ = session.exec(&format!("rm -rf {}", remote_dir));

    info!(
        "SQL export v2 complete -> {:?} ({} bytes)",
        local_path, size
    );
    Ok(size)
}
