use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ssh::SshSession;

/// Tables used to compute the schema fingerprint.
const FINGERPRINT_TABLES: &[&str] = &[
    "v_extensions",
    "v_domains",
    "v_voicemails",
    "v_ring_groups",
    "v_dialplans",
];

/// Version identity for a FusionPBX deployment.
/// Since FusionPBX has no standard version file, we use:
///   - FreeSwitch version string (proxy for install generation)
///   - Schema fingerprint (hash of key table column sets — changes with schema migrations)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FpbxVersion {
    pub freeswitch: String,
    pub schema_fingerprint: String,
}

impl FpbxVersion {
    /// Short human-readable label.
    pub fn label(&self) -> String {
        let fs = self.freeswitch_short();
        format!("FS {} (schema {})", fs, &self.schema_fingerprint[..8])
    }

    /// e.g. "1.10.12" extracted from the full version string.
    pub fn freeswitch_short(&self) -> &str {
        // "FreeSWITCH version: 1.10.12-release+..." → "1.10.12"
        let s = self
            .freeswitch
            .split_whitespace()
            .find(|t| {
                t.chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
            })
            .unwrap_or(self.freeswitch.as_str());
        // strip anything after first non-version char (-, +, ~)
        let end = s
            .find(|c: char| ['-', '+', '~'].contains(&c))
            .unwrap_or(s.len());
        &s[..end]
    }

    /// Parse major.minor from the freeswitch version string.
    fn major_minor(&self) -> Option<(u32, u32)> {
        let s = self.freeswitch_short();
        let mut parts = s.split('.');
        let major: u32 = parts.next()?.parse().ok()?;
        let minor: u32 = parts.next()?.parse().ok()?;
        Some((major, minor))
    }
}

/// Result of a version compatibility check between source and destination.
#[derive(Debug, Clone, PartialEq)]
pub enum VersionCompat {
    /// Identical schema — no column adaptation needed.
    Identical,
    /// Schema differs but within a supported range — column intersection will be applied.
    Compatible { src: FpbxVersion, dst: FpbxVersion },
    /// Versions are too far apart or clearly incompatible — restore blocked.
    Unsupported { reason: String },
}

impl VersionCompat {
    pub fn is_ok(&self) -> bool {
        !matches!(self, VersionCompat::Unsupported { .. })
    }

    /// True when the import SQL must be adapted (columns filtered).
    pub fn needs_adaptation(&self) -> bool {
        matches!(self, VersionCompat::Compatible { .. })
    }

    /// Human-readable status line for the UI.
    pub fn status_line(&self) -> String {
        match self {
            VersionCompat::Identical => "✓ Schema identical — no adaptation needed".into(),
            VersionCompat::Compatible { src, dst } => format!(
                "⚠ Schema differs ({} → {}) — column intersection will be applied",
                src.freeswitch_short(),
                dst.freeswitch_short()
            ),
            VersionCompat::Unsupported { reason } => format!("✗ {}", reason),
        }
    }
}

/// Check whether restoring from `src` to `dst` is supported.
pub fn check_compat(src: &FpbxVersion, dst: &FpbxVersion) -> VersionCompat {
    if src.schema_fingerprint == dst.schema_fingerprint {
        return VersionCompat::Identical;
    }

    // Block if FreeSwitch major version differs (1.x → 2.x would be a huge jump).
    if let (Some((sm, _)), Some((dm, _))) = (src.major_minor(), dst.major_minor())
        && sm != dm
    {
        return VersionCompat::Unsupported {
            reason: format!(
                "FreeSwitch major version mismatch ({} → {}); cross-generation restore not supported",
                src.freeswitch_short(),
                dst.freeswitch_short()
            ),
        };
    }

    // Same major (or unknown) — allow with column intersection.
    VersionCompat::Compatible {
        src: src.clone(),
        dst: dst.clone(),
    }
}

/// Detect the FusionPBX deployment version on a connected server.
pub fn detect_version(session: &SshSession) -> Result<FpbxVersion> {
    let fs_raw = session
        .exec_ok("freeswitch -version 2>/dev/null || echo unknown")
        .unwrap_or_else(|_| "unknown".into());

    let freeswitch = fs_raw
        .lines()
        .find(|l| {
            let ll = l.to_lowercase();
            ll.contains("freeswitch") || ll.starts_with('1') || ll.starts_with('2')
        })
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| fs_raw.trim().to_string());

    let schema_fingerprint =
        compute_schema_fingerprint(session).unwrap_or_else(|_| "unknown".into());

    Ok(FpbxVersion {
        freeswitch,
        schema_fingerprint,
    })
}

fn compute_schema_fingerprint(session: &SshSession) -> Result<String> {
    let mut all_cols = String::new();

    for table in FINGERPRINT_TABLES {
        let cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -c \
            \"SELECT string_agg(column_name, ',' ORDER BY column_name) \
              FROM information_schema.columns WHERE table_name='{}'\" 2>/dev/null",
            table
        );
        let cols = session.exec_ok(&cmd).unwrap_or_default();
        all_cols.push_str(table);
        all_cols.push(':');
        all_cols.push_str(cols.trim());
        all_cols.push(';');
    }

    let mut hasher = Sha256::new();
    hasher.update(all_cols.as_bytes());
    let hash = hex::encode(hasher.finalize());
    Ok(hash[..16].to_string())
}
