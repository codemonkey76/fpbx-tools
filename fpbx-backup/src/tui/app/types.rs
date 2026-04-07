use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use fpbx_core::{domain::FpbxDomain, ssh::VerifyResult};

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

/// Shared state for the background worker thread.
#[derive(Debug, Default)]
pub struct WorkerState {
    pub log: Vec<String>,
    pub progress: f64, // 0.0 – 1.0
    pub current_task: String,
    pub done: bool,
    pub error: Option<String>,
    pub bundle_paths: Vec<PathBuf>,
    pub verify_result: Option<VerifyResult>,
}

#[derive(Debug, Clone)]
pub struct SshHostEntry {
    pub hostname: String,
    pub user: String,
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
    pub worker: Option<Arc<Mutex<WorkerState>>>,

    // Done.
    pub bundle_paths: Vec<PathBuf>,
}
