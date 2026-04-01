use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use fpbx_core::ssh::SshSession;
use std::{
    Collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    Source,
    Dest,
    Routes,
    Gateways,
    Confirm,
    Progress,
    Done,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct OutboundRoute {
    pub dialplan_uuid: String,
    pub dialplan_name: String,
    pub dialplan_description: String,
    pub dialplan_order: String,
    pub dialplan_enabled: String,
    pub details: Vec<RouteDetail>,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct RouteDetail {
    pub dialplan_detail_uuid: String,
    pub dialplan_detail_tag: String,
    pub dialplan_detail_type: String,
    pub dialplan_detail_data: String,
    pub dialplan_detail_break: String,
    pub dialplan_detail_inline: String,
    pub dialplan_detail_group: String,
    pub dialplan_detail_order: String,
    pub dialplan_detail_enabled: String,
}

#[derive(Debug, Clone)]
pub struct Gateway {
    pub uuid: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct GatewayMapping {
    pub source: Gateway,
    pub dest_options: Vec<Gateway>,
    pub selected_idx: Option<usize>,
    pub list_state: usize,
}

impl GatewayMapping {
    pub fn resolved_dest_uuid(&self) -> Option<&str> {
        self.selected_idx
        .and_then(|i| self.dest_options.get(i))
            .map(|g| g.uuid.as_str())
    }
}

#[derive(Debug, Default)]
pub struct WorkerState {
    pub log: Vec<String>
    pub progress: f64,
    pub current_task: String,
    pub done: bool,
    pub error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct SshHostEntry {
    pub hostname: String,
    pub user: String,
}

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,

    // SSH config.
    pub ssh_hosts: HashMap<String, SshHostEntry>,

    // Source screen.
    pub src_host_input: String,
    pub src_user_input: String,
    pub src_active_field: usize,
    pub src_verified: bool,
    pub src_verifying: bool,
    pub src_verify_msg: Option<String>,
    pub src_verify_ok: bool,

    // Dest screen.
    pub dst_host_input: String,
    pub dst_user_input: String,
    pub dst_active_field: usize,
    pub dst_verified: bool,
    pub dst_verifying: bool,
    pub dst_verify_msg: Option<String>,
    pub dst_verify_ok: bool,

    // Routes screen.
    pub routes: Vec<OutboundRoute>,
    pub routes_list_idx: usize,
    pub loading_routes: bool,

    // Gateways screen.
    pub gateway_mappings: Vec<GatewayMapping>,
    pub gateway_focus_idx: usize,

    // Progress.
    pub worker: Option<Arc<Mutex<WorkerState>>>,
}

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
            src_verified: false,
            src_verifying: false,
            src_verify_msg: None,
            src_verify_ok: false,
            dst_host_input: String::new(),
            dst_user_input: String::new(),
            dst_active_field: 0,
            dst_verified: false,
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
            && self.worker.as_ref()
                .map(|w| !w.lock().unwrap().done)
                .unwrap_or(false)
    }

    pub fn resolved_src_host(&self) -> String {
        let key = self.src_host_input.trim().to_lowercase();
        self.ssh_hosts.get(&key)
            .map(|e| e.hostname.clone())
            .unwrap_or_else(|| self.src_host_input.trim().to_string())
    }

    pub fn resolved_dst_host(&self) -> String {
        let key = self.dst_host_input.trim().to_lowercase();
        self.ssh_hosts.get(&key)
            .map(|e| e.hostname.clone())
            .unwrap_or_else(|| self.dst_host_input.trim().to_string())
    }

    fn apply_src_ssh_lookup(&mut self) {
        let key = self.src_host_input.trim().to_lowercase();
        if let Some(e) = self.ssh_hosts.get(&key) {
            self.src_user_input = e.user.clone();
        }
    }

    fn apply_dst_ssh_lookup(&mut self) {
        let key = self.dst_host_input.trim().to_lowercase();
        if let Some(e) = self.ssh_hosts.get(&key) {
            self.dst_user_input = e.user.clone();
        }
    }

    pub fn selected_routes(&self) -> Vec<&OutboundRoute> {
        self.routes.iter().filter(|r| r.selected).collect()
    }

    pub fn tick(&mut self) {
        // Poll verify workers.
        if self.screen == AppScreen::Source && self.src_verifying {
            if let Some(w) = &self.worker {
                let state = w.lock().unwrap();
                if state.done {
                    if let Some(ref err) = state.error {
                        self.src_verify_msg = Some(format!("✗ {}", err));
                        self.src_verify_ok = false;
                    } else if let Some(msg) = state.log.last() {
                        self.src_verify_msg = Some(format!("✓ {}", msg));
                        self.src_verify_ok = true;
                    }
                    self.src_verifying = false;
                    self.worker = None;
                }
            }
        }
        if self.screen == AppScreen::Dest && self.dst_verifying {
            if let Some(w) = &self.worker {
                let state = w.lock().unwrap();
                if state.done {
                    if let Some(ref err) = state.error {
                        self.dst_verify_msg = Some(format!("✗ {}", err));
                        self.dst_verify_ok = false;
                    } else if let Some(msg) = state.log.last() {
                        self.dst_verify_msg = Some(format!("✓ {}", msg));
                        self.dst_verify_ok = true;
                    }
                    self.dst_verifying = false;
                    self.worker = None;
                }
            }
        }
        // Poll progress worker.
        if self.screen == AppScreen::Progress {
            if let Some(w) = &self.worker {
                let state = w.lock().unwrap();
                if state.done {
                    if let Some(ref err) = state.error {
                        let err = err.clone();
                        drop(state);
                        self.screen = AppScreen::Error(err);
                    } else {
                        drop(state);
                        self.screen = AppScreen::Done;
                    }
                }
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.screen.clone() {
            AppScreen::Source   => self.handle_source_key(key),
            AppScreen::Dest     => self.handle_dest_key(key),
            AppScreen::Routes   => self.handle_routes_key(key),
            AppScreen::Gateways => self.handle_gateways_key(key),
            AppScreen::Confirm  => self.handle_confirm_key(key),
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
                if self.src_verifying { return; }
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

    fn start_src_verify(&mut self) {
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
        let wstate = Arc::new(Mutex::new(WorkerState::default()));
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        thread::spawn(move || {
            loop {
                if let Some(r) = slot3.lock().unwrap().take() {
                    let mut w = wstate2.lock().unwrap();
                    match r {
                        Ok(msg) => { w.log.push(msg); w.done = true; }
                        Err(e)  => { w.error = Some(e); w.done = true; }
                    }
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }

    fn load_routes(&mut self) {
        self.loading_routes = true;
        let host = self.resolved_src_host();
        let user = self.src_user_input.trim().to_string();
        let slot: Arc<Mutex<Option<Result<Vec<OutboundRoute>, String>>>> =
            Arc::new(Mutex::new(None));
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
                if self.dst_verifying { return; }
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

    fn start_dst_verify(&mut self) {
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
        let wstate = Arc::new(Mutex::new(WorkerState::default()));
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        thread::spawn(move || {
            loop {
                if let Some(r) = slot3.lock().unwrap().take() {
                    let mut w = wstate2.lock().unwrap();
                    match r {
                        Ok(msg) => { w.log.push(msg); w.done = true; }
                        Err(e)  => { w.error = Some(e); w.done = true; }
                    }
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }

    fn build_gateway_mappings(&mut self) {
        // Collect all gateway UUIDs referenced in selected routes.
        let mut src_gateway_uuids: Vec<String> = Vec::new();
        for route in self.routes.iter().filter(|r| r.selected) {
            for detail in &route.details {
                if detail.dialplan_detail_type == "bridge"
                    && detail.dialplan_detail_data.contains("/gateway/")
                {
                    // Extract UUID from sofia/gateway/<uuid>/$1
                    if let Some(uuid) = extract_gateway_uuid(&detail.dialplan_detail_data) {
                        if !src_gateway_uuids.contains(&uuid) {
                            src_gateway_uuids.push(uuid);
                        }
                    }
                }
            }
        }

        let src_host = self.resolved_src_host();
        let src_user = self.src_user_input.trim().to_string();
        let dst_host = self.resolved_dst_host();
        let dst_user = self.dst_user_input.trim().to_string();

        let slot: Arc<Mutex<Option<Result<Vec<GatewayMapping>, String>>>> =
            Arc::new(Mutex::new(None));
        let slot2 = slot.clone();

        thread::spawn(move || {
            let r = build_mappings(&src_host, &src_user, &dst_host, &dst_user, &src_gateway_uuids);
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
                if n > 0 { self.routes_list_idx = self.routes_list_idx.saturating_sub(1); }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if n > 0 { self.routes_list_idx = (self.routes_list_idx + 1).min(n - 1); }
            }
            KeyCode::Char(' ') => {
                if let Some(r) = self.routes.get_mut(self.routes_list_idx) {
                    r.selected = !r.selected;
                }
            }
            KeyCode::Char('a') => {
                let all = self.routes.iter().all(|r| r.selected);
                for r in &mut self.routes { r.selected = !all; }
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
                // Advance to next unresolved mapping or confirm.
                if self.gateway_focus_idx + 1 < mappings_len {
                    self.gateway_focus_idx += 1;
                } else {
                    self.screen = AppScreen::Confirm;
                }
            }
            KeyCode::Char('s') => {
                // Skip this gateway mapping.
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

    fn start_transfer(&mut self) {
        let dst_host = self.resolved_dst_host();
        let dst_user = self.dst_user_input.trim().to_string();

        // Build UUID remap map.
        let mut uuid_remap: HashMap<String, String> = HashMap::new();
        for mapping in &self.gateway_mappings {
            if let Some(dest_uuid) = mapping.resolved_dest_uuid() {
                uuid_remap.insert(mapping.source.uuid.clone(), dest_uuid.to_string());
            }
        }

        let routes: Vec<OutboundRoute> = self.routes.iter()
            .filter(|r| r.selected)
            .cloned()
            .collect();

        let wstate = Arc::new(Mutex::new(WorkerState::default()));
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

// --- Background functions ---

fn fetch_outbound_routes(host: &str, user: &str) -> Result<Vec<OutboundRoute>, String> {
    let session = SshSession::connect(host, user).map_err(|e| e.to_string())?;

    // Fetch dialplans.
    let sql = "SELECT dialplan_uuid, dialplan_name, COALESCE(dialplan_description,''), \
               dialplan_order, dialplan_enabled \
               FROM v_dialplans dp \
               WHERE dp.dialplan_context = 'global' \
               AND dp.domain_uuid IS NULL \
               AND EXISTS ( \
                 SELECT 1 FROM v_dialplan_details dd \
                 WHERE dd.dialplan_uuid = dp.dialplan_uuid \
                 AND dd.dialplan_detail_type = 'bridge' \
                 AND dd.dialplan_detail_data LIKE '%/gateway/%' \
               ) \
               ORDER BY dialplan_name";

    let cmd = format!(
        "sudo -u postgres psql -d fusionpbx -t -A -F'|' -P pager=off -c \"{}\"",
        sql
    );
    let out = session.exec_ok(&cmd).map_err(|e| e.to_string())?;

    let mut routes = Vec::new();
    for line in out.lines() {
        let p: Vec<&str> = line.splitn(5, '|').collect();
        if p.len() < 5 { continue; }
        let uuid = p[0].trim().to_string();

        // Fetch details for this dialplan.
        let detail_sql = format!(
            "SELECT dialplan_detail_uuid, dialplan_detail_tag, dialplan_detail_type, \
             dialplan_detail_data, COALESCE(dialplan_detail_break,''), \
             COALESCE(dialplan_detail_inline,''), COALESCE(dialplan_detail_group,'0'), \
             dialplan_detail_order, COALESCE(dialplan_detail_enabled,'true') \
             FROM v_dialplan_details \
             WHERE dialplan_uuid = '{}' \
             ORDER BY dialplan_detail_order",
            uuid
        );
        let detail_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -F'|' -P pager=off -c \"{}\"",
            detail_sql
        );
        let detail_out = session.exec_ok(&detail_cmd).unwrap_or_default();
        let mut details = Vec::new();
        for dline in detail_out.lines() {
            let dp: Vec<&str> = dline.splitn(9, '|').collect();
            if dp.len() < 9 { continue; }
            details.push(RouteDetail {
                dialplan_detail_uuid:    dp[0].trim().to_string(),
                dialplan_detail_tag:     dp[1].trim().to_string(),
                dialplan_detail_type:    dp[2].trim().to_string(),
                dialplan_detail_data:    dp[3].trim().to_string(),
                dialplan_detail_break:   dp[4].trim().to_string(),
                dialplan_detail_inline:  dp[5].trim().to_string(),
                dialplan_detail_group:   dp[6].trim().to_string(),
                dialplan_detail_order:   dp[7].trim().to_string(),
                dialplan_detail_enabled: dp[8].trim().to_string(),
            });
        }

        routes.push(OutboundRoute {
            dialplan_uuid:        uuid,
            dialplan_name:        p[1].trim().to_string(),
            dialplan_description: p[2].trim().to_string(),
            dialplan_order:       p[3].trim().to_string(),
            dialplan_enabled:     p[4].trim().to_string(),
            details,
            selected: true, // default all selected
        });
    }
    Ok(routes)
}

fn fetch_gateways(host: &str, user: &str) -> Result<Vec<Gateway>, String> {
    let session = SshSession::connect(host, user).map_err(|e| e.to_string())?;
    let cmd = "sudo -u postgres psql -d fusionpbx -t -A -F'|' -P pager=off -c \
               \"SELECT gateway_uuid, gateway FROM v_gateways ORDER BY gateway\"";
    let out = session.exec_ok(cmd).map_err(|e| e.to_string())?;
    let mut gateways = Vec::new();
    for line in out.lines() {
        let p: Vec<&str> = line.splitn(2, '|').collect();
        if p.len() < 2 { continue; }
        gateways.push(Gateway {
            uuid: p[0].trim().to_string(),
            name: p[1].trim().to_string(),
        });
    }
    Ok(gateways)
}

fn build_mappings(
    src_host: &str, src_user: &str,
    dst_host: &str, dst_user: &str,
    src_uuids: &[String],
) -> Result<Vec<GatewayMapping>, String> {
    let src_gws = fetch_gateways(src_host, src_user)?;
    let dst_gws = fetch_gateways(dst_host, dst_user)?;

    let mut mappings = Vec::new();
    for uuid in src_uuids {
        let src_gw = match src_gws.iter().find(|g| &g.uuid == uuid) {
            Some(g) => g.clone(),
            None => continue,
        };
        // Try to auto-match by name.
        let auto_match = dst_gws.iter().position(|g| g.name == src_gw.name);
        let selected_idx = auto_match;
        let list_state = auto_match.unwrap_or(0);
        mappings.push(GatewayMapping {
            source: src_gw,
            dest_options: dst_gws.clone(),
            selected_idx,
            list_state,
        });
    }
    Ok(mappings)
}

fn extract_gateway_uuid(bridge_data: &str) -> Option<String> {
    // Format: sofia/gateway/<uuid>/... or sofia/gateway/<uuid>
    let parts: Vec<&str> = bridge_data.split('/').collect();
    parts.iter().position(|&p| p == "gateway")
        .and_then(|i| parts.get(i + 1))
        .map(|s| s.to_string())
}

fn run_transfer(
    dst_host: &str,
    dst_user: &str,
    routes: &[OutboundRoute],
    uuid_remap: &HashMap<String, String>,
    wstate: &Arc<Mutex<WorkerState>>,
) -> Result<()> {
    let log = |msg: &str, progress: f64| {
        let mut w = wstate.lock().unwrap();
        w.log.push(msg.to_string());
        w.current_task = msg.to_string();
        w.progress = progress;
    };

    log("Connecting to destination server…", 0.05);
    let session = SshSession::connect(dst_host, dst_user)?;

    let total = routes.len() as f64;
    for (i, route) in routes.iter().enumerate() {
        let progress = 0.1 + (i as f64 / total) * 0.8;
        log(&format!("Transferring {}…", route.dialplan_name), progress);

        // Check if route already exists by name — delete first to avoid conflicts.
        let del_sql = format!(
            "DELETE FROM v_dialplans WHERE dialplan_name = '{}' \
             AND dialplan_context = 'global' AND domain_uuid IS NULL",
            route.dialplan_name.replace('\'', "''")
        );
        let del_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -P pager=off -c \"{}\"",
            del_sql
        );
        let _ = session.exec(&del_cmd);

        // Insert dialplan.
        let insert_sql = format!(
            "INSERT INTO v_dialplans \
             (domain_uuid, dialplan_uuid, dialplan_context, dialplan_name, \
              dialplan_order, dialplan_enabled, dialplan_description) \
             VALUES (NULL, '{}', 'global', '{}', {}, {}, '{}')",
            route.dialplan_uuid,
            route.dialplan_name.replace('\'', "''"),
            route.dialplan_order,
            route.dialplan_enabled,
            route.dialplan_description.replace('\'', "''"),
        );
        let insert_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -P pager=off -c \"{}\"",
            insert_sql
        );
        session.exec_ok(&insert_cmd)
            .map_err(|e| anyhow::anyhow!("insert dialplan {}: {}", route.dialplan_name, e))?;

        // Insert details with UUID remapping on bridge actions.
        for detail in &route.details {
            let data = if detail.dialplan_detail_type == "bridge"
                && detail.dialplan_detail_data.contains("/gateway/")
            {
                remap_bridge_uuid(&detail.dialplan_detail_data, uuid_remap)
            } else {
                detail.dialplan_detail_data.clone()
            };

            let detail_sql = format!(
                "INSERT INTO v_dialplan_details \
                 (domain_uuid, dialplan_uuid, dialplan_detail_uuid, dialplan_detail_tag, \
                  dialplan_detail_type, dialplan_detail_data, dialplan_detail_break, \
                  dialplan_detail_inline, dialplan_detail_group, dialplan_detail_order, \
                  dialplan_detail_enabled) \
                 VALUES (NULL, '{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, {}, {})",
                route.dialplan_uuid,
                detail.dialplan_detail_uuid,
                detail.dialplan_detail_tag.replace('\'', "''"),
                detail.dialplan_detail_type.replace('\'', "''"),
                data.replace('\'', "''"),
                detail.dialplan_detail_break.replace('\'', "''"),
                detail.dialplan_detail_inline.replace('\'', "''"),
                detail.dialplan_detail_group,
                detail.dialplan_detail_order,
                detail.dialplan_detail_enabled,
            );
            let detail_cmd = format!(
                "sudo -u postgres psql -d fusionpbx -t -A -P pager=off -c \"{}\"",
                detail_sql
            );
            session.exec_ok(&detail_cmd)
                .map_err(|e| anyhow::anyhow!("insert detail: {}", e))?;
        }
    }

    log("Reloading FusionPBX XML on destination…", 0.95);
    let reload_cmd = "fs_cli -x 'reloadxml' 2>/dev/null || true";
    let _ = session.exec(reload_cmd);

    Ok(())
}

fn remap_bridge_uuid(bridge_data: &str, uuid_remap: &HashMap<String, String>) -> String {
    // sofia/gateway/<src_uuid>/$1 → sofia/gateway/<dst_uuid>/$1
    let parts: Vec<&str> = bridge_data.split('/').collect();
    if let Some(gw_pos) = parts.iter().position(|&p| p == "gateway") {
        if let Some(uuid) = parts.get(gw_pos + 1) {
            if let Some(new_uuid) = uuid_remap.get(*uuid) {
                let mut new_parts = parts.clone();
                new_parts[gw_pos + 1] = new_uuid.as_str();
                return new_parts.join("/");
            }
        }
    }
    bridge_data.to_string()
}

fn parse_ssh_config() -> HashMap<String, SshHostEntry> {
    let mut map = HashMap::new();
    let config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".ssh")
        .join("config");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return map;
    };
    let mut current_alias: Option<String> = None;
    let mut current_hostname: Option<String> = None;
    let mut current_user: Option<String> = None;

    let mut flush = |map: &mut HashMap<String, SshHostEntry>,
                     alias: &mut Option<String>,
                     hostname: &mut Option<String>,
                     user: &mut Option<String>| {
        if let (Some(a), Some(h), Some(u)) = (alias.take(), hostname.take(), user.take()) {
            map.insert(a.to_lowercase(), SshHostEntry { hostname: h, user: u });
        }
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let (key, val) = match line.split_once(|c: char| c.is_whitespace()) {
            Some(pair) => (pair.0.to_lowercase(), pair.1.trim().to_string()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                flush(&mut map, &mut current_alias, &mut current_hostname, &mut current_user);
                if !val.contains('*') { current_alias = Some(val); }
            }
            "hostname" => { current_hostname = Some(val); }
            "user"     => { current_user = Some(val); }
            _ => {}
        }
    }
    flush(&mut map, &mut current_alias, &mut current_hostname, &mut current_user);
    map
}
