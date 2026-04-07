use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::info;

use crate::{domain::DOMAIN_TABLES, ssh::SshSession};

/// Describes a domain rename to apply during restore.
/// All occurrences of `src_uuid` and `src_name` in the SQL are replaced with
/// `dest_uuid` and `dest_name` respectively before import.
#[derive(Debug, Clone)]
pub struct DomainRename {
    pub src_uuid: String,
    pub src_name: String,
    pub dest_uuid: String,
    pub dest_name: String,
}

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

        // Get column list in ordinal order.
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

    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    let out_file = std::fs::File::create(local_path).context("create gz")?;
    let mut gz = GzEncoder::new(out_file, Compression::best());
    gz.write_all(full_sql.as_bytes()).context("gz write")?;
    gz.finish().context("gz finish")?;

    let size = std::fs::metadata(local_path)?.len();
    let _ = std::fs::remove_file(&raw_path);

    let _ = session.exec(&format!("rm -rf {}", remote_dir));

    info!("SQL export v2 complete -> {:?} ({} bytes)", local_path, size);
    Ok(size)
}

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
        progress(&format!("Renaming domain {} → {}…", r.src_name, r.dest_name));
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
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;
        let mut buf = Vec::new();
        let mut gz = GzEncoder::new(&mut buf, Compression::best());
        gz.write_all(adapted_sql.as_bytes()).context("gz adapted SQL")?;
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

/// Read a gzipped SQL file, strip COPY columns that don't exist on the destination,
/// and return the adapted plain SQL.
///
/// Works on the raw SQL string to correctly handle embedded newlines inside
/// quoted CSV fields (e.g. dialplan_xml).
fn adapt_sql_for_dest(
    local_sql_gz: &Path,
    session: &SshSession,
    progress: &mut dyn FnMut(&str),
) -> Result<String> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let f = std::fs::File::open(local_sql_gz).context("open sql gz")?;
    let mut gz = GzDecoder::new(f);
    let mut sql = String::new();
    gz.read_to_string(&mut sql).context("decompress sql")?;

    let mut dest_col_cache: HashMap<String, HashSet<String>> = HashMap::new();
    let mut out = String::with_capacity(sql.len());

    // Process the SQL character by character, identifying COPY blocks.
    // We scan line-by-line for headers and `\.` terminators, but treat the
    // data body as an opaque blob unless we need to strip columns.
    let mut pos = 0;
    let bytes = sql.as_bytes();
    let n = bytes.len();

    while pos < n {
        // Read one line.
        let line_start = pos;
        while pos < n && bytes[pos] != b'\n' {
            pos += 1;
        }
        let line = &sql[line_start..pos];
        if pos < n { pos += 1; } // consume '\n'

        if let Some((table, src_cols)) = parse_copy_header(line) {
            let dest_cols = dest_columns_for(session, &table, &mut dest_col_cache);

            if dest_cols.is_empty() {
                info!("adapt: skipping table {} (not on destination)", table);
                // Skip raw bytes until we see a line that is exactly `\.`
                pos = skip_copy_block(bytes, pos);
                continue;
            }

            let dropped: Vec<&str> = src_cols
                .iter()
                .filter(|c| !dest_cols.contains(c.as_str()))
                .map(|s| s.as_str())
                .collect();

            if dropped.is_empty() {
                // No columns need dropping — emit header as-is and pass data through raw.
                out.push_str(line);
                out.push('\n');
                let block_end = skip_copy_block(bytes, pos);
                out.push_str(&sql[pos..block_end]);
                pos = block_end;
            } else {
                progress(&format!(
                    "Schema adapt {}: dropping {} column(s): {}",
                    table, dropped.len(), dropped.join(", ")
                ));
                info!("adapt: table {} — dropping source-only columns: {:?}", table, dropped);

                let keep_indices: Vec<usize> = src_cols
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| dest_cols.contains(c.as_str()))
                    .map(|(i, _)| i)
                    .collect();
                let keep_cols: Vec<&str> = keep_indices
                    .iter()
                    .map(|&i| src_cols[i].as_str())
                    .collect();

                out.push_str(&format!(
                    "COPY {} ({}) FROM stdin WITH CSV;\n",
                    table, keep_cols.join(", ")
                ));

                // Extract the raw data block, then parse CSV records properly.
                let block_end = skip_copy_block(bytes, pos);
                let block = &sql[pos..block_end];
                // block ends with "\\.\n" — strip that before parsing rows.
                let data = if block.ends_with("\\.\n") {
                    &block[..block.len() - 3]
                } else {
                    block.trim_end_matches("\\.")
                };

                for record in iter_csv_records(data) {
                    let fields = split_csv_row_raw(record);
                    let kept: Vec<&str> = keep_indices
                        .iter()
                        .map(|&i| *fields.get(i).unwrap_or(&""))
                        .collect();
                    out.push_str(&kept.join(","));
                    out.push('\n');
                }
                out.push_str("\\.\n");
                pos = block_end;
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }

    Ok(out)
}

/// Advance `pos` past the current COPY data block (stops after the `\.\n` line).
/// Returns the new position (just after `\.\n`).
fn skip_copy_block(bytes: &[u8], mut pos: usize) -> usize {
    let n = bytes.len();
    loop {
        let line_start = pos;
        while pos < n && bytes[pos] != b'\n' { pos += 1; }
        let line = &bytes[line_start..pos];
        if pos < n { pos += 1; }
        if line == b"\\." {
            return pos;
        }
        if pos >= n { return pos; }
    }
}

/// Iterate over complete CSV records in a COPY data block.
/// Handles quoted fields containing embedded newlines correctly.
fn iter_csv_records(data: &str) -> impl Iterator<Item = &str> {
    CsvRecordIter { data, pos: 0 }
}

struct CsvRecordIter<'a> {
    data: &'a str,
    pos: usize,
}

impl<'a> Iterator for CsvRecordIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let bytes = self.data.as_bytes();
        let n = bytes.len();
        if self.pos >= n { return None; }

        let start = self.pos;
        let mut in_quotes = false;

        loop {
            if self.pos >= n {
                let record = &self.data[start..self.pos];
                return if record.is_empty() { None } else { Some(record) };
            }
            match bytes[self.pos] {
                b'"' if !in_quotes => { in_quotes = true; self.pos += 1; }
                b'"' if in_quotes => {
                    self.pos += 1;
                    if self.pos < n && bytes[self.pos] == b'"' {
                        self.pos += 1; // escaped ""
                    } else {
                        in_quotes = false;
                    }
                }
                b'\n' if !in_quotes => {
                    let record = &self.data[start..self.pos];
                    self.pos += 1;
                    if !record.is_empty() { return Some(record); }
                    // empty line — skip
                }
                _ => { self.pos += 1; }
            }
        }
    }
}

/// Parse a `COPY table (col, ...) FROM stdin WITH CSV;` line.
/// Returns (table_name, [col, ...]) or None if the line isn't a COPY header.
fn parse_copy_header(line: &str) -> Option<(String, Vec<String>)> {
    let upper = line.to_ascii_uppercase();
    if !upper.starts_with("COPY ") || !upper.contains("FROM STDIN WITH CSV") {
        return None;
    }
    let rest = &line[5..]; // skip "COPY "
    let paren_open = rest.find('(')?;
    let table = rest[..paren_open].trim().to_string();
    let paren_close = rest.rfind(')')?;
    let cols: Vec<String> = rest[paren_open + 1..paren_close]
        .split(',')
        .map(|c| c.trim().to_string())
        .collect();
    Some((table, cols))
}

/// Query the destination for all columns of `table`, using `cache` to avoid
/// repeated round-trips for the same table.
fn dest_columns_for<'a>(
    session: &SshSession,
    table: &str,
    cache: &'a mut HashMap<String, HashSet<String>>,
) -> &'a HashSet<String> {
    cache.entry(table.to_string()).or_insert_with(|| {
        let cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"SELECT column_name FROM information_schema.columns \
              WHERE table_name='{}'\" 2>/dev/null",
            table
        );
        session
            .exec_ok(&cmd)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    })
}

/// Split a PostgreSQL COPY CSV row into raw field slices.
///
/// Returns the raw text of each field (including surrounding `"` if quoted),
/// so that joining the kept fields with `,` produces a valid COPY CSV row.
/// Unquoted empty fields represent NULL; `""` represents an empty string.
fn split_csv_row_raw(row: &str) -> Vec<&str> {
    let bytes = row.as_bytes();
    let n = bytes.len();
    let mut fields = Vec::new();
    let mut pos = 0;

    loop {
        let field_start = pos;
        if pos < n && bytes[pos] == b'"' {
            // Quoted field — scan to the closing (unescaped) quote.
            pos += 1;
            while pos < n {
                if bytes[pos] == b'"' {
                    pos += 1;
                    if pos < n && bytes[pos] == b'"' {
                        pos += 1; // escaped "" — continue
                    } else {
                        break; // end of quoted field
                    }
                } else {
                    pos += 1;
                }
            }
        } else {
            // Unquoted field — scan to the next comma.
            while pos < n && bytes[pos] != b',' {
                pos += 1;
            }
        }
        fields.push(&row[field_start..pos]);
        if pos >= n {
            break;
        }
        // Skip the comma.
        pos += 1;
    }

    fields
}

/// Per-table UUID column metadata queried from the destination server's schema.
#[derive(Debug, Default, Clone)]
struct TableUuidSchema {
    /// Columns that are both UUID type and the table's primary key.
    pk_cols: HashSet<String>,
    /// All columns that are UUID type (superset of pk_cols).
    uuid_cols: HashSet<String>,
}

/// Query UUID column info (type + PK membership) for every domain table from
/// the destination server.  Uses a single psql round-trip.
fn query_uuid_schemas(session: &SshSession) -> HashMap<String, TableUuidSchema> {
    let tables_in = DOMAIN_TABLES
        .iter()
        .map(|t| format!("'{}'", t))
        .collect::<Vec<_>>()
        .join(",");

    // Returns rows: table_name | column_name | pk  (where pk is 'pk' or 'col')
    let sql = format!(
        "SELECT c.table_name, c.column_name, \
         CASE WHEN pk.column_name IS NOT NULL THEN 'pk' ELSE 'col' END \
         FROM information_schema.columns c \
         LEFT JOIN ( \
           SELECT kcu.table_name, kcu.column_name \
           FROM information_schema.table_constraints tc \
           JOIN information_schema.key_column_usage kcu \
             ON tc.constraint_name = kcu.constraint_name \
             AND tc.table_schema   = kcu.table_schema \
           WHERE tc.constraint_type = 'PRIMARY KEY' \
         ) pk ON c.table_name = pk.table_name AND c.column_name = pk.column_name \
         WHERE c.data_type    = 'uuid' \
           AND c.table_schema = 'public' \
           AND c.table_name   IN ({}) \
         ORDER BY c.table_name, c.ordinal_position",
        tables_in
    );

    let cmd = format!(
        "sudo -u postgres psql -d fusionpbx -t -A -F '|' -c \"{}\" 2>/dev/null",
        sql.replace('"', "\\\"")
    );

    let output = session.exec_ok(&cmd).unwrap_or_default();
    let mut schemas: HashMap<String, TableUuidSchema> = HashMap::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(3, '|').collect();
        if parts.len() != 3 { continue; }
        let (table, col, kind) = (parts[0].trim(), parts[1].trim(), parts[2].trim());
        if table.is_empty() || col.is_empty() { continue; }
        let schema = schemas.entry(table.to_string()).or_default();
        schema.uuid_cols.insert(col.to_string());
        if kind == "pk" {
            schema.pk_cols.insert(col.to_string());
        }
    }

    info!("UUID schema loaded for {} tables", schemas.len());
    schemas
}

/// Remap per-record PK UUIDs in the COPY blocks to fresh UUID v4 values, and
/// propagate the same mapping to FK columns within the dump.
///
/// Rules:
///   - PK columns (as reported by the schema):  always get a new UUID, except
///     for `preserve_uuid` (the destination domain UUID, already correct).
///   - Non-PK UUID columns: if the value was seen as a PK earlier in the dump,
///     apply the same mapping (preserving FK integrity).  If the value is not
///     in the mapping it references something outside the dump (e.g. a global
///     gateway or profile) and is left unchanged.
///   - Non-UUID columns and NULLs: always left unchanged.
///
/// Because DOMAIN_TABLES is ordered parents-before-children, parent PKs are
/// always in the mapping by the time we process child FK columns.
fn remap_domain_uuids(
    sql: &str,
    preserve_uuid: &str,
    schemas: &HashMap<String, TableUuidSchema>,
) -> String {
    let mut mapping: HashMap<String, String> = HashMap::new();
    let mut out = String::with_capacity(sql.len() + 4096);
    let bytes = sql.as_bytes();
    let n = bytes.len();
    let mut pos = 0;

    while pos < n {
        // Read one line.
        let line_start = pos;
        while pos < n && bytes[pos] != b'\n' { pos += 1; }
        let line = &sql[line_start..pos];
        if pos < n { pos += 1; }

        if let Some((table, src_cols)) = parse_copy_header(line) {
            // Re-emit the COPY header unchanged.
            out.push_str(line);
            out.push('\n');

            let block_start = pos;
            let block_end = skip_copy_block(bytes, pos);
            let block = &sql[block_start..block_end];
            pos = block_end;

            let Some(schema) = schemas.get(&table) else {
                // No schema info for this table — emit block as-is.
                out.push_str(block);
                continue;
            };

            // Pre-compute per-column flags for this COPY block.
            let col_is_pk: Vec<bool> = src_cols.iter()
                .map(|c| schema.pk_cols.contains(c.as_str()))
                .collect();
            let col_is_uuid: Vec<bool> = src_cols.iter()
                .map(|c| schema.uuid_cols.contains(c.as_str()))
                .collect();

            // Strip the trailing `\.\n` before iterating rows.
            let data = if block.ends_with("\\.\n") {
                &block[..block.len() - 3]
            } else {
                block.trim_end_matches("\\.")
            };

            for record in iter_csv_records(data) {
                let fields = split_csv_row_raw(record);
                let mut row_parts: Vec<String> = Vec::with_capacity(fields.len());

                for (i, &raw) in fields.iter().enumerate() {
                    let is_pk   = *col_is_pk  .get(i).unwrap_or(&false);
                    let is_uuid = *col_is_uuid.get(i).unwrap_or(&false);

                    // NULL (empty unquoted) or column we don't care about.
                    if raw.is_empty() || (!is_pk && !is_uuid) {
                        row_parts.push(raw.to_string());
                        continue;
                    }

                    // UUID values in PostgreSQL COPY CSV are always unquoted
                    // (hex + dashes — no special chars that need quoting).
                    if is_pk {
                        if raw.eq_ignore_ascii_case(preserve_uuid) {
                            // Domain UUID — already set correctly, do not remap.
                            row_parts.push(raw.to_string());
                        } else {
                            let new = mapping
                                .entry(raw.to_lowercase())
                                .or_insert_with(|| uuid::Uuid::new_v4().to_string())
                                .clone();
                            row_parts.push(new);
                        }
                    } else {
                        // Non-PK UUID: apply mapping if this value was a PK earlier
                        // (intra-dump FK), otherwise leave as-is (external reference).
                        match mapping.get(&raw.to_lowercase()) {
                            Some(mapped) => row_parts.push(mapped.clone()),
                            None => row_parts.push(raw.to_string()),
                        }
                    }
                }

                out.push_str(&row_parts.join(","));
                out.push('\n');
            }
            out.push_str("\\.\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }

    out
}

