use crossterm::event::{KeyCode, KeyEvent};
use fpbx_core::{new_worker, ssh::SshSession};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
};

use super::types::{App, AppScreen, GatewayMapping, OutboundRoute};
use super::workers::{build_mappings, extract_gateway_uuid, fetch_outbound_routes, run_transfer};

type RouteResult = Option<Result<Vec<OutboundRoute>, String>>;
type MappingResult = Option<Result<Vec<GatewayMapping>, String>>;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.screen.clone() {
            AppScreen::Source => self.handle_source_key(key),
            AppScreen::Dest => self.handle_dest_key(key),
            AppScreen::Routes => self.handle_routes_key(key),
            AppScreen::Gateways => self.handle_gateways_key(key),
            AppScreen::Confirm => self.handle_confirm_key(key),
            AppScreen::Progress => {}
            AppScreen::Done => {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Enter | KeyCode::Esc) {
                    self.should_quit = true;
                }
            }
            AppScreen::Error(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.screen = AppScreen::Source;
                }
            }
        }
    }

    // --- Source screen ---

    fn handle_source_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => self.src_active_field = 1 - self.src_active_field,
            KeyCode::Char(c) => {
                if self.src_active_field == 0 {
                    self.src_host_input.push(c);
                    self.apply_src_ssh_lookup();
                } else {
                    self.src_user_input.push(c);
                }
                self.src_verify_ok = false;
                self.src_verify_msg = None;
            }
            KeyCode::Backspace => {
                if self.src_active_field == 0 {
                    self.src_host_input.pop();
                    self.apply_src_ssh_lookup();
                } else {
                    self.src_user_input.pop();
                }
                self.src_verify_ok = false;
                self.src_verify_msg = None;
            }
            KeyCode::Enter => {
                if self.src_verifying {
                    return;
                }
                if self.src_verify_ok {
                    self.load_routes();
                } else {
                    self.start_src_verify();
                }
            }
            KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

    pub(super) fn start_src_verify(&mut self) {
        self.src_verifying = true;
        let host = self.resolved_src_host();
        let user = self.src_user_input.trim().to_string();
        let slot: Arc<Mutex<Option<Result<String, String>>>> = Arc::new(Mutex::new(None));
        let slot2 = slot.clone();
        let slot3 = slot.clone();
        thread::spawn(move || {
            let r = SshSession::connect(&host, &user)
                .and_then(|s: SshSession| s.verify_fusionpbx())
                .map(|v| v.summary())
                .map_err(|e| e.to_string());
            *slot2.lock().unwrap() = Some(r);
        });
        let wstate = new_worker();
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        thread::spawn(move || {
            loop {
                if let Some(r) = slot3.lock().unwrap().take() {
                    let mut w = wstate2.lock().unwrap();
                    match r {
                        Ok(msg) => {
                            w.log.push(msg);
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

    pub(super) fn load_routes(&mut self) {
        self.loading_routes = true;
        let host = self.resolved_src_host();
        let user = self.src_user_input.trim().to_string();
        let slot: Arc<Mutex<RouteResult>> = Arc::new(Mutex::new(None));
        let slot2 = slot.clone();
        thread::spawn(move || {
            let r = fetch_outbound_routes(&host, &user);
            *slot2.lock().unwrap() = Some(r);
        });
        loop {
            if let Some(r) = slot.lock().unwrap().take() {
                match r {
                    Ok(routes) => {
                        self.routes = routes;
                        self.loading_routes = false;
                        self.screen = AppScreen::Routes;
                    }
                    Err(e) => {
                        self.screen = AppScreen::Error(e);
                        self.loading_routes = false;
                    }
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // --- Dest screen ---

    fn handle_dest_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => self.dst_active_field = 1 - self.dst_active_field,
            KeyCode::Char(c) => {
                if self.dst_active_field == 0 {
                    self.dst_host_input.push(c);
                    self.apply_dst_ssh_lookup();
                } else {
                    self.dst_user_input.push(c);
                }
                self.dst_verify_ok = false;
                self.dst_verify_msg = None;
            }
            KeyCode::Backspace => {
                if self.dst_active_field == 0 {
                    self.dst_host_input.pop();
                    self.apply_dst_ssh_lookup();
                } else {
                    self.dst_user_input.pop();
                }
                self.dst_verify_ok = false;
                self.dst_verify_msg = None;
            }
            KeyCode::Enter => {
                if self.dst_verifying {
                    return;
                }
                if self.dst_verify_ok {
                    self.build_gateway_mappings();
                } else {
                    self.start_dst_verify();
                }
            }
            KeyCode::Esc => self.screen = AppScreen::Routes,
            _ => {}
        }
    }

    pub(super) fn start_dst_verify(&mut self) {
        self.dst_verifying = true;
        let host = self.resolved_dst_host();
        let user = self.dst_user_input.trim().to_string();
        let slot: Arc<Mutex<Option<Result<String, String>>>> = Arc::new(Mutex::new(None));
        let slot2 = slot.clone();
        let slot3 = slot.clone();
        thread::spawn(move || {
            let r = SshSession::connect(&host, &user)
                .and_then(|s: SshSession| s.verify_fusionpbx())
                .map(|v| v.summary())
                .map_err(|e| e.to_string());
            *slot2.lock().unwrap() = Some(r);
        });
        let wstate = new_worker();
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        thread::spawn(move || {
            loop {
                if let Some(r) = slot3.lock().unwrap().take() {
                    let mut w = wstate2.lock().unwrap();
                    match r {
                        Ok(msg) => {
                            w.log.push(msg);
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

    pub(super) fn build_gateway_mappings(&mut self) {
        let mut src_gateway_uuids: Vec<String> = Vec::new();
        for route in self.routes.iter().filter(|r| r.selected) {
            for detail in &route.details {
                if detail.dialplan_detail_type == "bridge"
                    && detail.dialplan_detail_data.contains("/gateway/")
                    && let Some(uuid) = extract_gateway_uuid(&detail.dialplan_detail_data)
                    && !src_gateway_uuids.contains(&uuid)
                {
                    src_gateway_uuids.push(uuid);
                }
            }
        }

        let src_host = self.resolved_src_host();
        let src_user = self.src_user_input.trim().to_string();
        let dst_host = self.resolved_dst_host();
        let dst_user = self.dst_user_input.trim().to_string();

        let slot: Arc<Mutex<MappingResult>> = Arc::new(Mutex::new(None));
        let slot2 = slot.clone();

        thread::spawn(move || {
            let r = build_mappings(
                &src_host,
                &src_user,
                &dst_host,
                &dst_user,
                &src_gateway_uuids,
            );
            *slot2.lock().unwrap() = Some(r);
        });

        loop {
            if let Some(r) = slot.lock().unwrap().take() {
                match r {
                    Ok(mappings) => {
                        self.gateway_mappings = mappings;
                        self.gateway_focus_idx = 0;
                        if self.gateway_mappings.is_empty() {
                            self.screen = AppScreen::Confirm;
                        } else {
                            self.screen = AppScreen::Gateways;
                        }
                    }
                    Err(e) => self.screen = AppScreen::Error(e),
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // --- Routes screen ---

    fn handle_routes_key(&mut self, key: KeyEvent) {
        let n = self.routes.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if n > 0 {
                    self.routes_list_idx = self.routes_list_idx.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if n > 0 {
                    self.routes_list_idx = (self.routes_list_idx + 1).min(n - 1);
                }
            }
            KeyCode::Char(' ') => {
                if let Some(r) = self.routes.get_mut(self.routes_list_idx) {
                    r.selected = !r.selected;
                }
            }
            KeyCode::Char('a') => {
                let all = self.routes.iter().all(|r| r.selected);
                for r in &mut self.routes {
                    r.selected = !all;
                }
            }
            KeyCode::Enter => {
                if self.routes.iter().any(|r| r.selected) {
                    self.screen = AppScreen::Dest;
                }
            }
            KeyCode::Esc => self.screen = AppScreen::Source,
            _ => {}
        }
    }

    // --- Gateways screen ---

    fn handle_gateways_key(&mut self, key: KeyEvent) {
        let mappings_len = self.gateway_mappings.len();
        if mappings_len == 0 {
            self.screen = AppScreen::Confirm;
            return;
        }
        let mapping = &mut self.gateway_mappings[self.gateway_focus_idx];
        let opts_len = mapping.dest_options.len();

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if opts_len > 0 {
                    mapping.list_state = mapping.list_state.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if opts_len > 0 {
                    mapping.list_state = (mapping.list_state + 1).min(opts_len - 1);
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if opts_len > 0 {
                    mapping.selected_idx = Some(mapping.list_state);
                }
                if self.gateway_focus_idx + 1 < mappings_len {
                    self.gateway_focus_idx += 1;
                } else {
                    self.screen = AppScreen::Confirm;
                }
            }
            KeyCode::Char('s') => {
                mapping.selected_idx = None;
                if self.gateway_focus_idx + 1 < mappings_len {
                    self.gateway_focus_idx += 1;
                } else {
                    self.screen = AppScreen::Confirm;
                }
            }
            KeyCode::Esc => self.screen = AppScreen::Routes,
            _ => {}
        }
    }

    // --- Confirm screen ---

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.start_transfer(),
            KeyCode::Char('n') | KeyCode::Esc => self.screen = AppScreen::Gateways,
            _ => {}
        }
    }

    // --- Transfer worker ---

    pub(super) fn start_transfer(&mut self) {
        let dst_host = self.resolved_dst_host();
        let dst_user = self.dst_user_input.trim().to_string();

        let mut uuid_remap: HashMap<String, String> = HashMap::new();
        for mapping in &self.gateway_mappings {
            if let Some(dest_uuid) = mapping.resolved_dest_uuid() {
                uuid_remap.insert(mapping.source.uuid.clone(), dest_uuid.to_string());
            }
        }

        let routes: Vec<OutboundRoute> =
            self.routes.iter().filter(|r| r.selected).cloned().collect();

        let wstate = new_worker();
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        self.screen = AppScreen::Progress;

        thread::spawn(move || {
            let result = run_transfer(&dst_host, &dst_user, &routes, &uuid_remap, &wstate2);
            let mut w = wstate2.lock().unwrap();
            match result {
                Ok(()) => {
                    w.log.push("✓ Transfer complete".to_string());
                    w.progress = 1.0;
                    w.done = true;
                }
                Err(e) => {
                    w.log.push(format!("✗ {}", e));
                    w.error = Some(e.to_string());
                    w.done = true;
                }
            }
        });
    }
}
