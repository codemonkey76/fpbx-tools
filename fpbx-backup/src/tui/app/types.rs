use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use fpbx_core::{SshHostEntry, WorkerSlot, domain::FpbxDomain, ssh::VerifyResult};

/// Which screen the TUI is showing.
#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    Server,        // Enter host + user, verify SSH + FusionPBX
    Domains,       // Filterable list of domains
    OutputPath,    // Confirm/edit output path
    Progress,      // Export + bundle progress with log
    Done,          // Summary + bundle location
    Error(String), // Error overlay
}

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,

    // SSH config aliases.
    pub ssh_hosts: HashMap<String, SshHostEntry>,

    // Server screen.
    pub host_input: String,
    pub user_input: String,
    pub active_field: usize, // 0=host, 1=user
    pub verify_result: Option<Result<VerifyResult, String>>,
    pub verifying: bool,

    // Domain screen.
    pub domains: Vec<FpbxDomain>,
    pub domain_filter: String,
    pub domain_list_state: ratatui::widgets::ListState,
    pub filter_active: bool,
    pub loading_domains: bool,

    // Output path screen.
    pub output_path_input: String,

    // Domain multi-selection.
    pub selected_domain_uuids: HashSet<String>,

    // Progress screen.
    pub worker: Option<WorkerSlot>,

    // Done.
    pub bundle_paths: Vec<PathBuf>,
}
