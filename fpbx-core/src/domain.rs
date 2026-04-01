use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::SshSession;

/// All FusionPBX tables that carry a domain_uuid FK.
/// Order matters for restore (parents before children).
pub const DOMAIN_TABLES: &[&str] = &[
    "v_domains",
    "v_domain_settings",
    "v_users",
    "v_groups",
    "v_user_groups",
    "v_extensions",
    "v_extension_users",
    "v_gateways",
    "v_dialplans",
    "v_dialplan_details",
    "v_ring_groups",
    "v_ring_group_destinations",
    "v_ivr_menus",
    "v_ivr_menu_options",
    "v_time_conditions",
    "v_time_condition_periods",
    "v_voicemails",
    "v_voicemail_messages",
    "v_call_center_queues",
    "v_call_center_agents",
    "v_call_center_tiers",
    "v_recordings",
    "v_contacts",
    "v_contact_phones",
    "v_contact_emails",
    "v_contact_urls",
    "v_contact_addresses",
    "v_fax",
    "v_fax_files",
];

/// A single FusionPBX domain as returned by the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FpbxDomain {
    pub domain_uuid: String,
    pub domain_name: String,
    pub domain_description: Option<String>,
    pub domain_enabled: bool,
}

impl FpbxDomain {
    /// Display label used in the TUI list.
    pub fn label(&self) -> String {
        match &self.domain_description {
            Some(d) if !d.is_empty() => format!("{} — {}", self.domain_name, d),
            _ => self.domain_name.clone(),
        }
    }
}

/// Row counts per table for a given domain.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DomainTableCounts(pub Vec<(String, u64)>);

impl DomainTableCounts {
    pub fn total_rows(&self) -> u64 {
        self.0.iter().map(|(_, n)| n).sum()
    }
}

/// List all domains on the remote server by querying PostgreSQL.
pub fn list_domains(session: &SshSession) -> Result<Vec<FpbxDomain>> {
    info!("Listing domains on {}", session.host());

    let sql = r#"
        SELECT domain_uuid,
               domain_name,
               COALESCE(domain_description, '') as domain_description,
               COALESCE(domain_enabled::text, 'true') as domain_enabled
        FROM   v_domains
        ORDER  BY domain_name;
    "#;

    let out = psql_query(session, sql)?;
    let mut domains = Vec::new();

    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 {
            continue;
        }
        domains.push(FpbxDomain {
            domain_uuid: parts[0].trim().to_string(),
            domain_name: parts[1].trim().to_string(),
            domain_description: {
                let d = parts[2].trim().to_string();
                if d.is_empty() { None } else { Some(d) }
            },
            domain_enabled: parts[3].trim() != "false",
        });
    }

    info!("Found {} domains", domains.len());
    Ok(domains)
}

/// Count rows per table for a domain UUID.
pub fn count_domain_rows(session: &SshSession, domain_uuid: &str) -> Result<DomainTableCounts> {
    let mut counts = Vec::new();
    for table in DOMAIN_TABLES {
        // Skip tables that might not exist on all versions.
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE domain_uuid = '{}'",
            table, domain_uuid
        );
        let cmd = psql_cmd(&sql);
        let result = session.exec_ok(&cmd).unwrap_or_else(|_| "0".into());
        let n: u64 = result.trim().parse().unwrap_or(0);
        if n > 0 {
            counts.push((table.to_string(), n));
        }
    }
    Ok(DomainTableCounts(counts))
}

/// Run a psql command as the postgres user, return stdout.
pub fn psql_query(session: &SshSession, sql: &str) -> Result<String> {
    let escaped = sql.replace('\'', "'\\''");
    let cmd = format!(
        "sudo -u postgres psql -d fusionpbx -t -A -F'|' -c '{}'",
        escaped
    );
    session.exec_ok(&cmd).context("psql query failed")
}

fn psql_cmd(sql: &str) -> String {
    let escaped = sql.replace('\'', "'\\''");
    format!(
        "sudo -u postgres psql -d fusionpbx -t -A -F'|' -c '{}' 2>/dev/null || echo 0",
        escaped
    )
}

/// Resolve file paths on the remote server for a given domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainFilePaths {
    pub voicemail_dir: String,
    pub recordings_dir: String,
    pub dialplan_xml_dir: String,
    pub directory_xml_dir: String,
}

impl DomainFilePaths {
    pub fn for_domain(domain_name: &str) -> Self {
        Self {
            voicemail_dir: format!("/var/lib/freeswitch/storage/voicemail/default/{}", domain_name),
            recordings_dir: format!("/var/lib/freeswitch/recordings/{}", domain_name),
            dialplan_xml_dir: format!("/etc/freeswitch/dialplan/{}", domain_name),
            directory_xml_dir: format!("/etc/freeswitch/directory/{}", domain_name),
        }
    }

    /// Return only paths that actually exist on the remote.
    pub fn existing(&self, session: &SshSession) -> Vec<String> {
        let paths = [
            &self.voicemail_dir,
            &self.recordings_dir,
            &self.dialplan_xml_dir,
            &self.directory_xml_dir,
        ];
        paths
            .iter()
            .filter(|p| {
                session
                    .exec_ok(&format!("test -d '{}' && echo yes || echo no", p))
                    .map(|r| r.trim() == "yes")
                    .unwrap_or(false)
            })
            .map(|p| p.to_string())
            .collect()
    }
}
