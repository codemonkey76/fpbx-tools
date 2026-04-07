use crossterm::event::{KeyCode, KeyEvent};
use fpbx_core::{
    domain::list_domains,
    ssh::{SshSession, VerifyResult},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

use super::types::{App, AppScreen, WorkerState};
use super::workers::run_backup_worker;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        match &self.screen {
            AppScreen::Server => self.handle_server_key(key),
            AppScreen::Domains => self.handle_domains_key(key),
            AppScreen::OutputPath => self.handle_output_key(key),
            AppScreen::Progress => self.handle_progress_key(key),
            AppScreen::Done => {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Enter | KeyCode::Esc) {
                    self.should_quit = true;
                }
            }
            AppScreen::Error(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    self.screen = AppScreen::Server;
                    self.verify_result = None;
                }
            }
        }
    }

    // --- Server screen ---

    fn handle_server_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => {
                self.active_field = 1 - self.active_field;
            }
            KeyCode::Char(c) => {
                if self.active_field == 0 {
                    self.host_input.push(c);
                    self.apply_ssh_config_lookup();
                } else {
                    self.user_input.push(c);
                }
                self.verify_result = None;
            }
            KeyCode::Backspace => {
                if self.active_field == 0 {
                    self.host_input.pop();
                    self.apply_ssh_config_lookup();
                } else {
                    self.user_input.pop();
                }
                self.verify_result = None;
            }
            KeyCode::Enter => {
                if self.verifying {
                    return;
                }
                if matches!(&self.verify_result, Some(Ok(v)) if v.is_ok()) {
                    self.advance_to_domains();
                    return;
                }
                self.start_verify();
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    pub(super) fn apply_ssh_config_lookup(&mut self) {
        let key = self.host_input.trim().to_lowercase();
        if let Some(entry) = self.ssh_hosts.get(&key) {
            self.user_input = entry.user.clone();
        }
    }

    pub(super) fn start_verify(&mut self) {
        self.verifying = true;
        self.verify_result = None;
        let host = self.resolved_host();
        let user = self.user_input.trim().to_string();
        let slot: Arc<Mutex<Option<Result<VerifyResult, String>>>> = Arc::new(Mutex::new(None));
        let slot2 = slot.clone();
        let slot3 = slot.clone();
        thread::spawn(move || {
            let r = SshSession::connect(&host, &user)
                .and_then(|s: SshSession| s.verify_fusionpbx())
                .map_err(|e: anyhow::Error| e.to_string());
            *slot2.lock().unwrap() = Some(r);
        });
        let wstate = Arc::new(Mutex::new(WorkerState::default()));
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        thread::spawn(move || {
            loop {
                if let Some(r) = slot3.lock().unwrap().take() {
                    let mut w = wstate2.lock().unwrap();
                    match r {
                        Ok(v) => {
                            w.log.push(v.summary());
                            w.verify_result = Some(v);
                            w.done = true;
                        }
                        Err(e) => {
                            w.error = Some(e);
                            w.done = true;
                        }
                    }
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }

    pub(super) fn advance_to_domains(&mut self) {
        self.loading_domains = true;
        let host = self.resolved_host();
        let user = self.user_input.trim().to_string();
        let slot: Arc<Mutex<super::DomainResult>> = Arc::new(Mutex::new(None));
        let slot2 = slot.clone();
        thread::spawn(move || {
            let r = SshSession::connect(&host, &user)
                .and_then(|s| list_domains(&s))
                .map_err(|e| e.to_string());
            *slot2.lock().unwrap() = Some(r);
        });
        loop {
            if let Some(r) = slot.lock().unwrap().take() {
                match r {
                    Ok(domains) => {
                        self.domains = domains;
                        self.loading_domains = false;
                        self.screen = AppScreen::Domains;
                    }
                    Err(e) => {
                        self.screen = AppScreen::Error(e);
                        self.loading_domains = false;
                    }
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // --- Domains screen ---

    fn handle_domains_key(&mut self, key: KeyEvent) {
        if self.filter_active {
            match key.code {
                KeyCode::Esc => {
                    self.filter_active = false;
                    self.domain_filter.clear();
                }
                KeyCode::Enter => {
                    self.filter_active = false;
                }
                KeyCode::Backspace => {
                    self.domain_filter.pop();
                }
                KeyCode::Char(c) => {
                    self.domain_filter.push(c);
                    self.domain_list_state.select(Some(0));
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('/') => {
                self.filter_active = true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let n = self.filtered_domains().len();
                if n == 0 {
                    return;
                }
                let i = self.domain_list_state.selected().unwrap_or(0);
                self.domain_list_state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let n = self.filtered_domains().len();
                if n == 0 {
                    return;
                }
                let i = self.domain_list_state.selected().unwrap_or(0);
                self.domain_list_state
                    .select(Some((i + 1).min(n.saturating_sub(1))));
            }
            KeyCode::Char(' ') => {
                let i = self.domain_list_state.selected().unwrap_or(0);
                if let Some(d) = self.filtered_domains().get(i).copied() {
                    let uuid = d.domain_uuid.clone();
                    if self.selected_domain_uuids.contains(&uuid) {
                        self.selected_domain_uuids.remove(&uuid);
                    } else {
                        self.selected_domain_uuids.insert(uuid);
                    }
                }
            }
            KeyCode::Char('a') => {
                let uuids: Vec<String> = self
                    .filtered_domains()
                    .iter()
                    .map(|d| d.domain_uuid.clone())
                    .collect();
                let all_selected = uuids.iter().all(|u| self.selected_domain_uuids.contains(u));
                if all_selected {
                    for u in &uuids {
                        self.selected_domain_uuids.remove(u);
                    }
                } else {
                    for u in uuids {
                        self.selected_domain_uuids.insert(u);
                    }
                }
            }
            KeyCode::Enter => {
                if !self.selected_domain_uuids.is_empty() {
                    self.screen = AppScreen::OutputPath;
                }
            }
            KeyCode::Esc => {
                self.screen = AppScreen::Server;
            }
            _ => {}
        }
    }

    // --- Output path screen ---

    fn handle_output_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => self.output_path_input.push(c),
            KeyCode::Backspace => {
                self.output_path_input.pop();
            }
            KeyCode::Enter => self.start_backup(),
            KeyCode::Esc => self.screen = AppScreen::Domains,
            _ => {}
        }
    }

    // --- Progress screen ---

    fn handle_progress_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc && !self.is_running_task() {
            self.screen = AppScreen::Domains;
        }
    }

    // --- Backup worker ---

    pub(super) fn start_backup(&mut self) {
        let host = self.resolved_host();
        let user = self.user_input.trim().to_string();
        let domains = self
            .selected_domains()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let output_dir = PathBuf::from(self.output_path_input.trim());

        let wstate = Arc::new(Mutex::new(WorkerState::default()));
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        self.screen = AppScreen::Progress;

        thread::spawn(move || {
            run_backup_worker(host, user, domains, output_dir, wstate2);
        });
    }
}
