mod handlers;
mod ssh_config;
mod workers;

pub mod types;

pub use types::{App, AppScreen};

use anyhow::Result;
use fpbx_core::{bundle::default_backup_dir, domain::FpbxDomain};
use ssh_config::parse_ssh_config;

/// Convenience alias used by advance_to_domains and the slot pattern.
pub(super) type DomainResult = Option<Result<Vec<FpbxDomain>, String>>;

impl App {
    pub fn new() -> Self {
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(0));
        let ssh_hosts = parse_ssh_config();
        Self {
            screen: AppScreen::Server,
            should_quit: false,
            ssh_hosts,
            host_input: String::new(),
            user_input: String::new(),
            active_field: 0,
            verify_result: None,
            verifying: false,
            domains: Vec::new(),
            domain_filter: String::new(),
            domain_list_state: list_state,
            filter_active: false,
            loading_domains: false,
            output_path_input: default_backup_dir().to_string_lossy().to_string(),
            selected_domain_uuids: std::collections::HashSet::new(),
            worker: None,
            bundle_paths: Vec::new(),
        }
    }

    pub fn is_running_task(&self) -> bool {
        self.screen == AppScreen::Progress
            && self
                .worker
                .as_ref()
                .map(|w| !w.lock().unwrap().done)
                .unwrap_or(false)
    }

    pub fn is_typing(&self) -> bool {
        matches!(self.screen, AppScreen::Server | AppScreen::OutputPath)
            || (self.screen == AppScreen::Domains && self.filter_active)
    }

    pub fn bundle_paths(&self) -> &[std::path::PathBuf] {
        &self.bundle_paths
    }

    pub fn filtered_domains(&self) -> Vec<&FpbxDomain> {
        let q = self.domain_filter.to_lowercase();
        self.domains
            .iter()
            .filter(|d| q.is_empty() || d.label().to_lowercase().contains(&q))
            .collect()
    }

    pub fn selected_domains(&self) -> Vec<&FpbxDomain> {
        self.domains
            .iter()
            .filter(|d| self.selected_domain_uuids.contains(&d.domain_uuid))
            .collect()
    }

    /// Resolved hostname — uses HostName from ssh config if available, else raw input.
    pub fn resolved_host(&self) -> String {
        let key = self.host_input.trim().to_lowercase();
        self.ssh_hosts
            .get(&key)
            .map(|e| e.hostname.clone())
            .unwrap_or_else(|| self.host_input.trim().to_string())
    }

    /// Called every ~100ms tick.
    pub fn tick(&mut self) {
        // Poll verify worker on Server screen.
        if self.screen == AppScreen::Server
            && self.verifying
            && let Some(w) = &self.worker
        {
            let state = w.lock().unwrap();
            if state.done {
                if let Some(ref v) = state.verify_result {
                    self.verify_result = Some(Ok(v.clone()));
                    self.verifying = false;
                } else if let Some(ref err) = state.error {
                    self.verify_result = Some(Err(err.clone()));
                    self.verifying = false;
                }
            }
        }

        // Poll worker for completion on Progress screen.
        if self.screen == AppScreen::Progress
            && let Some(w) = &self.worker
        {
            let state = w.lock().unwrap();
            if state.done {
                if let Some(ref err) = state.error {
                    self.screen = AppScreen::Error(err.clone());
                } else {
                    self.bundle_paths = state.bundle_paths.clone();
                    self.screen = AppScreen::Done;
                }
            }
        }
    }
}
