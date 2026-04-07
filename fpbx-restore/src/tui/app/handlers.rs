use crossterm::event::{KeyCode, KeyEvent};
use fpbx_core::{
    db::DomainRename,
    ssh::{SshSession, VerifyResult},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

use super::types::{App, AppScreen, WorkerState};
use super::workers::{build_rename, run_restore_worker};

type VerifySlot = Option<Result<VerifyResult, String>>;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.screen.clone() {
            AppScreen::BundlePicker => self.handle_picker_key(key),
            AppScreen::Preview => self.handle_preview_key(key),
            AppScreen::Server => self.handle_server_key(key),
            AppScreen::Confirm => self.handle_confirm_key(key),
            AppScreen::Progress => {}
            AppScreen::Done => {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Enter | KeyCode::Esc) {
                    self.should_quit = true;
                }
            }
            AppScreen::Error(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.screen = AppScreen::BundlePicker;
                }
            }
        }
    }

    // --- Bundle picker screen ---

    fn handle_picker_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let n = self.bundles.len();
                if n == 0 {
                    return;
                }
                let i = self.bundle_list_state.selected().unwrap_or(0);
                self.bundle_list_state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let n = self.bundles.len();
                if n == 0 {
                    return;
                }
                let i = self.bundle_list_state.selected().unwrap_or(0);
                self.bundle_list_state
                    .select(Some((i + 1).min(n.saturating_sub(1))));
            }
            KeyCode::Char(' ') => {
                let n = self.bundles.len();
                if n == 0 {
                    return;
                }
                let i = self.bundle_list_state.selected().unwrap_or(0);
                if let Some((path, _)) = self.bundles.get(i) {
                    let path = path.clone();
                    if self.selected_bundle_paths.contains(&path) {
                        self.selected_bundle_paths.remove(&path);
                    } else {
                        self.selected_bundle_paths.insert(path);
                    }
                }
            }
            KeyCode::Char('a') => {
                let all_paths: Vec<PathBuf> = self.bundles.iter().map(|(p, _)| p.clone()).collect();
                let all_selected = all_paths
                    .iter()
                    .all(|p| self.selected_bundle_paths.contains(p));
                if all_selected {
                    for p in &all_paths {
                        self.selected_bundle_paths.remove(p);
                    }
                } else {
                    for p in all_paths {
                        self.selected_bundle_paths.insert(p);
                    }
                }
            }
            KeyCode::Enter => {
                if !self.selected_bundle_paths.is_empty() {
                    self.active_field = 0;
                    self.verify_result = None;
                    self.screen = AppScreen::Server;
                } else if let Some(i) = self.bundle_list_state.selected()
                    && let Some((path, manifest)) = self.bundles.get(i)
                {
                    self.selected_bundle_path = Some(path.clone());
                    self.selected_manifest = Some(manifest.clone());
                    self.screen = AppScreen::Preview;
                }
            }
            KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

    // --- Preview screen ---

    fn handle_preview_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                self.active_field = 0;
                self.verify_result = None;
                self.screen = AppScreen::Server;
            }
            KeyCode::Esc => self.screen = AppScreen::BundlePicker,
            _ => {}
        }
    }

    // --- Server screen ---

    fn handle_server_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => self.active_field = 1 - self.active_field,
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
                    let src_name = self
                        .selected_bundles()
                        .first()
                        .map(|(_, m)| m.domain.domain_name.clone())
                        .or_else(|| {
                            self.selected_manifest
                                .as_ref()
                                .map(|m| m.domain.domain_name.clone())
                        })
                        .unwrap_or_default();
                    self.dest_domain_input = src_name;
                    self.confirm_field = 0;
                    self.screen = AppScreen::Confirm;
                    return;
                }
                self.start_verify();
            }
            KeyCode::Esc => self.screen = AppScreen::BundlePicker,
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

        let slot: Arc<Mutex<VerifySlot>> = Arc::new(Mutex::new(None));
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

    // --- Confirm screen ---

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        if self.confirm_field == 0 {
            // Editing destination domain name.
            match key.code {
                KeyCode::Char(c) => {
                    self.dest_domain_input.push(c);
                }
                KeyCode::Backspace => {
                    self.dest_domain_input.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.confirm_field = 1;
                }
                KeyCode::Esc => self.screen = AppScreen::Server,
                _ => {}
            }
        } else {
            // Ready to confirm.
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.start_restore_worker();
                }
                KeyCode::Tab | KeyCode::Char('e') => {
                    self.confirm_field = 0;
                }
                KeyCode::Char('n') | KeyCode::Esc => self.screen = AppScreen::Server,
                _ => {}
            }
        }
    }

    // --- Restore worker ---

    pub(super) fn start_restore_worker(&mut self) {
        let host = self.resolved_host();
        let user = self.user_input.trim().to_string();
        let dest_version = self.dest_version.clone();

        let bundles_with_rename: Vec<(PathBuf, Option<DomainRename>)> = {
            let selected = self.selected_bundles();
            if selected.is_empty() {
                let path = self.selected_bundle_path.clone().unwrap();
                let manifest = self.selected_manifest.as_ref().unwrap();
                let rename = build_rename(manifest, &self.dest_domain_input);
                vec![(path, rename)]
            } else if selected.len() == 1 {
                let (path, manifest) = selected[0];
                let rename = build_rename(manifest, &self.dest_domain_input);
                vec![(path.clone(), rename)]
            } else {
                // Multi-bundle: no renaming.
                selected.iter().map(|(p, _)| ((*p).clone(), None)).collect()
            }
        };

        let wstate = Arc::new(Mutex::new(WorkerState::default()));
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        self.screen = AppScreen::Progress;

        thread::spawn(move || {
            run_restore_worker(host, user, dest_version, bundles_with_rename, wstate2);
        });
    }
}
