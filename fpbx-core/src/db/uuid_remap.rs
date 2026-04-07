use std::collections::{HashMap, HashSet};
use tracing::info;

use crate::{domain::DOMAIN_TABLES, ssh::SshSession};

use super::adapt::{iter_csv_records, parse_copy_header, skip_copy_block, split_csv_row_raw};

/// Per-table UUID column metadata queried from the destination server's schema.
#[derive(Debug, Default, Clone)]
pub(super) struct TableUuidSchema {
    /// Columns that are both UUID type and the table's primary key.
    pub pk_cols: HashSet<String>,
    /// All columns that are UUID type (superset of pk_cols).
    pub uuid_cols: HashSet<String>,
}

/// Query UUID column info (type + PK membership) for every domain table from
/// the destination server.  Uses a single psql round-trip.
pub(super) fn query_uuid_schemas(session: &SshSession) -> HashMap<String, TableUuidSchema> {
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
        if parts.len() != 3 {
            continue;
        }
        let (table, col, kind) = (parts[0].trim(), parts[1].trim(), parts[2].trim());
        if table.is_empty() || col.is_empty() {
            continue;
        }
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
pub(super) fn remap_domain_uuids(
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
        while pos < n && bytes[pos] != b'\n' {
            pos += 1;
        }
        let line = &sql[line_start..pos];
        if pos < n {
            pos += 1;
        }

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
            let col_is_pk: Vec<bool> = src_cols
                .iter()
                .map(|c| schema.pk_cols.contains(c.as_str()))
                .collect();
            let col_is_uuid: Vec<bool> = src_cols
                .iter()
                .map(|c| schema.uuid_cols.contains(c.as_str()))
                .collect();

            // Strip the trailing `\.\n` before iterating rows.
            let data = block
                .strip_suffix("\\.\n")
                .unwrap_or_else(|| block.trim_end_matches("\\."));

            for record in iter_csv_records(data) {
                let fields = split_csv_row_raw(record);
                let mut row_parts: Vec<String> = Vec::with_capacity(fields.len());

                for (i, &raw) in fields.iter().enumerate() {
                    let is_pk = *col_is_pk.get(i).unwrap_or(&false);
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
