// Shared widget helpers for fpbx-backup TUI.
// Currently a placeholder; extract common widgets here as the TUI grows.

/// Format bytes as a human-readable string.
#[allow(dead_code)]
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut val = bytes as f64;
    let mut unit_idx = 0;
    while val >= 1024.0 && unit_idx < UNITS.len() - 1 {
        val /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", val, UNITS[unit_idx])
    }
}
