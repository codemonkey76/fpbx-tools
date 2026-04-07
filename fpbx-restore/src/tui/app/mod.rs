mod handlers;
mod workers;

pub mod types;

pub use types::{App, AppScreen};

use fpbx_core::bundle::{BundleManifest, default_backup_dir, list_bundles};
use fpbx_core::{parse_ssh_config, resolve_host, whoami_current_user};
use fpbx_tui_shared::TuiApp;
use std::path::PathBuf;

impl App {
    pub fn new() -> Self {
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(0));
        let bundle_dir = default_backup_dir();
        let bundles = list_bundles(&bundle_dir).unwrap_or_default();
        let ssh_hosts = parse_ssh_config();
        Self {
            screen: AppScreen::BundlePicker,
            should_quit: false,
            restore_succeeded: false,
            ssh_hosts,
            bundle_dir,
            bundles,
            bundle_list_state: list_state,
            selected_bundle_paths: std::collections::HashSet::new(),
            selected_manifest: None,
            selected_bundle_path: None,
            host_input: String::new(),
            user_input: whoami_current_user(),
            active_field: 0,
            verify_result: None,
            verifying: false,
            dest_version: None,
            dest_domain_input: String::new(),
            confirm_field: 0,
            worker: None,
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
        self.screen == AppScreen::Server
            || (self.screen == AppScreen::Confirm && self.confirm_field == 0)
    }

    pub fn selected_bundles(&self) -> Vec<&(PathBuf, BundleManifest)> {
        self.bundles
            .iter()
            .filter(|(p, _)| self.selected_bundle_paths.contains(p))
            .collect()
    }

    pub fn resolved_host(&self) -> String {
        resolve_host(&self.host_input, &self.ssh_hosts)
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
                    self.dest_version = v.fpbx_version.clone();
                    self.verify_result = Some(Ok(v.clone()));
                    self.verifying = false;
                } else if let Some(ref err) = state.error {
                    self.verify_result = Some(Err(err.clone()));
                    self.verifying = false;
                }
            }
        }

        // Poll restore worker for completion.
        if self.screen == AppScreen::Progress
            && let Some(w) = &self.worker
        {
            let state = w.lock().unwrap();
            if state.done {
                if let Some(ref err) = state.error {
                    let err = err.clone();
                    drop(state);
                    self.screen = AppScreen::Error(err);
                } else {
                    drop(state);
                    self.restore_succeeded = true;
                    self.screen = AppScreen::Done;
                }
            }
        }
    }
}

impl TuiApp for App {
    fn handle_key(&mut self, key: crossterm::event::KeyEvent) { self.handle_key(key); }
    fn tick(&mut self) { self.tick(); }
    fn is_running_task(&self) -> bool { self.is_running_task() }
    fn is_typing(&self) -> bool { self.is_typing() }
    fn should_quit(&self) -> bool { self.should_quit }
}
