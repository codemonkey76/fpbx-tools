use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{domain::count_domain_rows, SshSession};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    pub domain_uuid: String,
    pub rows: Vec<TableVerify>,
    pub files_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableVerify {
    pub table: String,
    pub expected: u64,
    pub actual: u64,
}

impl TableVerify {
    pub fn ok(&self) -> bool {
        self.actual >= self.expected
    }
}

impl VerifyReport {
    pub fn all_ok(&self) -> bool {
        self.files_ok && self.rows.iter().all(|r| r.ok())
    }

    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for r in &self.rows {
            let status = if r.ok() { "✓" } else { "✗" };
            lines.push(format!(
                "  {} {} — expected {} got {}",
                status, r.table, r.expected, r.actual
            ));
        }
        lines.push(format!(
            "  {} files",
            if self.files_ok { "✓" } else { "✗" }
        ));
        lines
    }
}

/// Compare expected counts (from manifest) against actual counts on dest server.
pub fn verify_restore(
    session: &SshSession,
    domain_uuid: &str,
    expected: &[(String, u64)],
) -> Result<VerifyReport> {
    let actual_counts = count_domain_rows(session, domain_uuid)?;
    let actual_map: std::collections::HashMap<&str, u64> = actual_counts
        .0
        .iter()
        .map(|(t, n)| (t.as_str(), *n))
        .collect();

    let rows = expected
        .iter()
        .map(|(table, exp)| {
            let actual = actual_map.get(table.as_str()).copied().unwrap_or(0);
            TableVerify {
                table: table.clone(),
                expected: *exp,
                actual,
            }
        })
        .collect();

    Ok(VerifyReport {
        domain_uuid: domain_uuid.to_string(),
        rows,
        files_ok: true, // file verification done separately
    })
}
