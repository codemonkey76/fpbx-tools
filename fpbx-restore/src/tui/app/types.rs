use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use fpbx_core::{SshHostEntry, WorkerSlot, bundle::BundleManifest, ssh::VerifyResult, version::FpbxVersion};

#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    BundlePicker,
    Preview,
    Server,
    Confirm,
    Progress,
    Done,
    Error(String),
}

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,
    pub restore_succeeded: bool,

    // SSH config aliases.
    pub ssh_hosts: HashMap<String, SshHostEntry>,

    // Bundle picker.
    pub bundle_dir: PathBuf,
    pub bundles: Vec<(PathBuf, BundleManifest)>,
    pub bundle_list_state: ratatui::widgets::ListState,
    pub selected_bundle_paths: HashSet<PathBuf>,

    // Selected bundle (single, for Preview screen).
    pub selected_manifest: Option<BundleManifest>,
    pub selected_bundle_path: Option<PathBuf>,

    // Server screen.
    pub host_input: String,
    pub user_input: String,
    pub active_field: usize,
    pub verify_result: Option<Result<VerifyResult, String>>,
    pub verifying: bool,

    // Detected destination version (populated after successful verify).
    pub dest_version: Option<FpbxVersion>,

    // Confirm screen — destination domain name (editable, single-bundle only).
    pub dest_domain_input: String,
    pub confirm_field: usize, // 0 = editing dest domain, 1 = ready to confirm

    // Progress.
    pub worker: Option<WorkerSlot>,
}
