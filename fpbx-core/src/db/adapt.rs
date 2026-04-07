use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::info;

use crate::ssh::SshSession;

/// Read a gzipped SQL file, strip COPY columns that don't exist on the destination,
/// and return the adapted plain SQL.
///
/// Works on the raw SQL string to correctly handle embedded newlines inside
/// quoted CSV fields (e.g. dialplan_xml).
pub(super) fn adapt_sql_for_dest(
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
        if pos < n {
            pos += 1;
        } // consume '\n'

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
                    table,
                    dropped.len(),
                    dropped.join(", ")
                ));
                info!(
                    "adapt: table {} — dropping source-only columns: {:?}",
                    table, dropped
                );

                let keep_indices: Vec<usize> = src_cols
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| dest_cols.contains(c.as_str()))
                    .map(|(i, _)| i)
                    .collect();
                let keep_cols: Vec<&str> =
                    keep_indices.iter().map(|&i| src_cols[i].as_str()).collect();

                out.push_str(&format!(
                    "COPY {} ({}) FROM stdin WITH CSV;\n",
                    table,
                    keep_cols.join(", ")
                ));

                // Extract the raw data block, then parse CSV records properly.
                let block_end = skip_copy_block(bytes, pos);
                let block = &sql[pos..block_end];
                // block ends with "\\.\n" — strip that before parsing rows.
                let data = block
                    .strip_suffix("\\.\n")
                    .unwrap_or_else(|| block.trim_end_matches("\\."));

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
pub(super) fn skip_copy_block(bytes: &[u8], mut pos: usize) -> usize {
    let n = bytes.len();
    loop {
        let line_start = pos;
        while pos < n && bytes[pos] != b'\n' {
            pos += 1;
        }
        let line = &bytes[line_start..pos];
        if pos < n {
            pos += 1;
        }
        if line == b"\\." {
            return pos;
        }
        if pos >= n {
            return pos;
        }
    }
}

/// Iterate over complete CSV records in a COPY data block.
/// Handles quoted fields containing embedded newlines correctly.
pub(super) fn iter_csv_records(data: &str) -> impl Iterator<Item = &str> {
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
        if self.pos >= n {
            return None;
        }

        let start = self.pos;
        let mut in_quotes = false;

        loop {
            if self.pos >= n {
                let record = &self.data[start..self.pos];
                return if record.is_empty() {
                    None
                } else {
                    Some(record)
                };
            }
            match bytes[self.pos] {
                b'"' if !in_quotes => {
                    in_quotes = true;
                    self.pos += 1;
                }
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
                    if !record.is_empty() {
                        return Some(record);
                    }
                    // empty line — skip
                }
                _ => {
                    self.pos += 1;
                }
            }
        }
    }
}

/// Parse a `COPY table (col, ...) FROM stdin WITH CSV;` line.
/// Returns (table_name, [col, ...]) or None if the line isn't a COPY header.
pub(super) fn parse_copy_header(line: &str) -> Option<(String, Vec<String>)> {
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
pub(super) fn split_csv_row_raw(row: &str) -> Vec<&str> {
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
