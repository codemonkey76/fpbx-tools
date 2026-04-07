mod handlers;
mod ssh_config;
mod workers;

pub mod types;

pub use types::{App, AppScreen, OutboundRoute};

use ssh_config::parse_ssh_config;

impl App {
    pub fn new() -> Self {
        let ssh_hosts = parse_ssh_config();
        Self {
            screen: AppScreen::Source,
            should_quit: false,
            ssh_hosts,
            src_host_input: String::new(),
            src_user_input: String::new(),
            src_active_field: 0,
            src_verifying: false,
            src_verify_msg: None,
            src_verify_ok: false,
            dst_host_input: String::new(),
            dst_user_input: String::new(),
            dst_active_field: 0,
            dst_verifying: false,
            dst_verify_msg: None,
            dst_verify_ok: false,
            routes: Vec::new(),
            routes_list_idx: 0,
            loading_routes: false,
            gateway_mappings: Vec::new(),
            gateway_focus_idx: 0,
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
        matches!(self.screen, AppScreen::Source | AppScreen::Dest)
    }

    pub fn resolved_src_host(&self) -> String {
        let key = self.src_host_input.trim().to_lowercase();
        self.ssh_hosts
            .get(&key)
            .map(|e| e.hostname.clone())
            .unwrap_or_else(|| self.src_host_input.trim().to_string())
    }

    pub fn resolved_dst_host(&self) -> String {
        let key = self.dst_host_input.trim().to_lowercase();
        self.ssh_hosts
            .get(&key)
            .map(|e| e.hostname.clone())
            .unwrap_or_else(|| self.dst_host_input.trim().to_string())
    }

    pub(super) fn apply_src_ssh_lookup(&mut self) {
        let key = self.src_host_input.trim().to_lowercase();
        if let Some(e) = self.ssh_hosts.get(&key) {
            self.src_user_input = e.user.clone();
        }
    }

    pub(super) fn apply_dst_ssh_lookup(&mut self) {
        let key = self.dst_host_input.trim().to_lowercase();
        if let Some(e) = self.ssh_hosts.get(&key) {
            self.dst_user_input = e.user.clone();
        }
    }

    #[allow(dead_code)]
    pub fn selected_routes(&self) -> Vec<&OutboundRoute> {
        self.routes.iter().filter(|r| r.selected).collect()
    }

    pub fn tick(&mut self) {
        // Poll source verify worker.
        if self.screen == AppScreen::Source && self.src_verifying {
            let result = if let Some(w) = &self.worker {
                let state = w.lock().unwrap();
                if state.done {
                    Some((state.error.clone(), state.log.last().cloned()))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((err, msg)) = result {
                if let Some(e) = err {
                    self.src_verify_msg = Some(format!("✗ {}", e));
                    self.src_verify_ok = false;
                } else if let Some(m) = msg {
                    self.src_verify_msg = Some(format!("✓ {}", m));
                    self.src_verify_ok = true;
                }
                self.src_verifying = false;
                self.worker = None;
            }
        }

        // Poll dest verify worker.
        if self.screen == AppScreen::Dest && self.dst_verifying {
            let result = if let Some(w) = &self.worker {
                let state = w.lock().unwrap();
                if state.done {
                    Some((state.error.clone(), state.log.last().cloned()))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((err, msg)) = result {
                if let Some(e) = err {
                    self.dst_verify_msg = Some(format!("✗ {}", e));
                    self.dst_verify_ok = false;
                } else if let Some(m) = msg {
                    self.dst_verify_msg = Some(format!("✓ {}", m));
                    self.dst_verify_ok = true;
                }
                self.dst_verifying = false;
                self.worker = None;
            }
        }

        // Poll progress worker.
        if self.screen == AppScreen::Progress {
            let result = if let Some(w) = &self.worker {
                let state = w.lock().unwrap();
                if state.done {
                    Some(state.error.clone())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(err) = result {
                if let Some(e) = err {
                    self.screen = AppScreen::Error(e);
                } else {
                    self.screen = AppScreen::Done;
                }
            }
        }
    }
}
