use crossterm::event::{KeyCode, KeyEvent};
use fpbx_core::bundle::{default_backup_dir, default_staging_dir, list_bundles, open_bundle, BundleManifest, DB_DUMP_NAME, FILES_TAR_NAME};
use fpbx_core::db::{import_domain_sql, DomainRename};
use fpbx_core::ssh::{SshSession, VerifyResult};
use fpbx_core::version::{check_compat, FpbxVersion};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

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

#[derive(Debug, Default)]
pub struct WorkerState {
    pub log: Vec<String>,
    pub progress: f64,
    pub current_task: String,
    pub done: bool,
    pub error: Option<String>,
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
    pub worker: Option<Arc<Mutex<WorkerState>>>,
}

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
            selected_bundle_paths: HashSet::new(),
            selected_manifest: None,
            selected_bundle_path: None,
            host_input: String::new(),
            user_input: whoami_user(),
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

    /// Resolved hostname — uses HostName from ssh config if available, else raw input.
    pub fn resolved_host(&self) -> String {
        let key = self.host_input.trim().to_lowercase();
        self.ssh_hosts
            .get(&key)
            .map(|e| e.hostname.clone())
            .unwrap_or_else(|| self.host_input.trim().to_string())
    }

    pub fn tick(&mut self) {
            // Poll verify worker on Server screen.
        if self.screen == AppScreen::Server && self.verifying {
            if let Some(w) = &self.worker {
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
        }

        // Poll restore worker for completion.
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
                        self.restore_succeeded = true;
                        self.screen = AppScreen::Done;
                    }
                }
            }
        }
    }

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

    fn handle_picker_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let n = self.bundles.len();
                if n == 0 { return; }
                let i = self.bundle_list_state.selected().unwrap_or(0);
                self.bundle_list_state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let n = self.bundles.len();
                if n == 0 { return; }
                let i = self.bundle_list_state.selected().unwrap_or(0);
                self.bundle_list_state.select(Some((i + 1).min(n.saturating_sub(1))));
            }
            KeyCode::Char(' ') => {
                let n = self.bundles.len();
                if n == 0 { return; }
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
                let all_selected = all_paths.iter().all(|p| self.selected_bundle_paths.contains(p));
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
                } else if let Some(i) = self.bundle_list_state.selected() {
                    if let Some((path, manifest)) = self.bundles.get(i) {
                        self.selected_bundle_path = Some(path.clone());
                        self.selected_manifest = Some(manifest.clone());
                        self.screen = AppScreen::Preview;
                    }
                }
            }
            KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

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
                // Already verified OK — advance to Confirm.
                if matches!(&self.verify_result, Some(Ok(v)) if v.is_ok()) {
                    // Pre-populate dest domain from the single selected bundle (if any).
                    let src_name = self.selected_bundles().first()
                        .map(|(_, m)| m.domain.domain_name.clone())
                        .or_else(|| self.selected_manifest.as_ref().map(|m| m.domain.domain_name.clone()))
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

    fn apply_ssh_config_lookup(&mut self) {
        let key = self.host_input.trim().to_lowercase();
        if let Some(entry) = self.ssh_hosts.get(&key) {
            self.user_input = entry.user.clone();
        }
    }

    fn start_verify(&mut self) {
        self.verifying = true;
        self.verify_result = None;
        let host = self.resolved_host();
        let user = self.user_input.trim().to_string();

        let slot: Arc<Mutex<Option<Result<VerifyResult, String>>>> =
            Arc::new(Mutex::new(None));
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

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        if self.confirm_field == 0 {
            // Editing destination domain name.
            match key.code {
                KeyCode::Char(c) => { self.dest_domain_input.push(c); }
                KeyCode::Backspace => { self.dest_domain_input.pop(); }
                KeyCode::Enter | KeyCode::Tab => { self.confirm_field = 1; }
                KeyCode::Esc => self.screen = AppScreen::Server,
                _ => {}
            }
        } else {
            // Ready to confirm.
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.start_restore_worker();
                }
                KeyCode::Tab | KeyCode::Char('e') => { self.confirm_field = 0; }
                KeyCode::Char('n') | KeyCode::Esc => self.screen = AppScreen::Server,
                _ => {}
            }
        }
    }

    fn start_restore_worker(&mut self) {
        let host = self.resolved_host();
        let user = self.user_input.trim().to_string();
        let dest_version = self.dest_version.clone();

        // Build list of (bundle_path, optional DomainRename).
        // Rename is only applied for single-bundle restores.
        let bundles_with_rename: Vec<(std::path::PathBuf, Option<DomainRename>)> = {
            let selected = self.selected_bundles();
            if selected.is_empty() {
                // Single bundle from Preview flow.
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
            let n = bundles_with_rename.len();
            for (idx, (bundle_path, rename)) in bundles_with_rename.into_iter().enumerate() {
                {
                    let mut w = wstate2.lock().unwrap();
                    w.progress = idx as f64 / n as f64;
                    w.log.push(format!(
                        "--- Bundle {}/{}: {} ---",
                        idx + 1,
                        n,
                        bundle_path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    w.current_task = format!("Restoring bundle {}/{}…", idx + 1, n);
                }

                let ws = wstate2.clone();
                let mut progress = move |msg: &str| {
                    let mut w = ws.lock().unwrap();
                    w.log.push(msg.to_string());
                    w.current_task = msg.to_string();
                };

                match run_restore(host.clone(), user.clone(), bundle_path, dest_version.clone(), rename.as_ref(), &mut progress) {
                    Ok(()) => {
                        let mut w = wstate2.lock().unwrap();
                        w.log.push("✓ Restore complete".to_string());
                    }
                    Err(e) => {
                        let mut w = wstate2.lock().unwrap();
                        w.error = Some(e.to_string());
                        w.done = true;
                        return;
                    }
                }
            }

            let mut w = wstate2.lock().unwrap();
            w.progress = 1.0;
            w.done = true;
        });
    }
}

fn build_rename(manifest: &fpbx_core::bundle::BundleManifest, dest_name: &str) -> Option<DomainRename> {
    let dest_name = dest_name.trim();
    if dest_name.is_empty() || dest_name == manifest.domain.domain_name {
        return None;
    }
    let new_uuid = uuid::Uuid::new_v4().to_string();
    Some(DomainRename {
        src_uuid: manifest.domain.domain_uuid.clone(),
        src_name: manifest.domain.domain_name.clone(),
        dest_uuid: new_uuid,
        dest_name: dest_name.to_string(),
    })
}

fn run_restore(
    host: String,
    user: String,
    bundle_path: std::path::PathBuf,
    dest_version: Option<FpbxVersion>,
    rename: Option<&DomainRename>,
    progress: &mut dyn FnMut(&str),
) -> anyhow::Result<()> {
    let stem = bundle_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bundle");
    let staging = default_staging_dir().join(stem);
    std::fs::create_dir_all(&staging)?;

    progress("Opening and verifying bundle…");
    let manifest = open_bundle(&bundle_path, &staging)?;

    // Version compatibility check.
    if let (Some(src_v), Some(dst_v)) = (&manifest.source_version, &dest_version) {
        let compat = check_compat(src_v, dst_v);
        if !compat.is_ok() {
            anyhow::bail!("{}", compat.status_line());
        }
        progress(&compat.status_line());
    } else {
        progress("Version info unavailable — proceeding with column intersection");
    }

    progress("Connecting to destination server…");
    let session = SshSession::connect(&host, &user)?;

    let sql_path = staging.join(DB_DUMP_NAME);
    import_domain_sql(&session, &sql_path, rename, progress)?;

    let files_tar = staging.join(FILES_TAR_NAME);
    if files_tar.exists() && files_tar.metadata().map(|m| m.len()).unwrap_or(0) > 100 {
        progress("Uploading file archive to destination…");
        let remote_tar = "/tmp/fpbx-restore-files.tar.gz";
        session.upload(&files_tar, std::path::Path::new(remote_tar), 0o600)?;
        progress("Creating destination directories…");
        for dir in &[
            "/var/lib/freeswitch/storage/voicemail/default",
            "/var/lib/freeswitch/recordings",
            "/etc/freeswitch/dialplan",
            "/etc/freeswitch/directory",
        ] {
            let _ = session.exec(&format!("sudo mkdir -p {}", dir));
        }
        progress("Extracting files on destination server…");
        session.exec_ok(&format!("sudo tar xzf {} -C /", remote_tar))?;
        let _ = session.exec(&format!("rm -f {}", remote_tar));
    }

    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

fn whoami_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "root".to_string())
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

    let flush = |map: &mut HashMap<String, SshHostEntry>,
                  alias: &mut Option<String>,
                  hostname: &mut Option<String>,
                  user: &mut Option<String>| {
        if let (Some(a), Some(h), Some(u)) = (alias.take(), hostname.take(), user.take()) {
            map.insert(a.to_lowercase(), SshHostEntry { hostname: h, user: u });
        }
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = match line.split_once(|c: char| c.is_whitespace()) {
            Some(pair) => (pair.0.to_lowercase(), pair.1.trim().to_string()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                flush(&mut map, &mut current_alias, &mut current_hostname, &mut current_user);
                if !val.contains('*') {
                    current_alias = Some(val);
                }
            }
            "hostname" => { current_hostname = Some(val); }
            "user" => { current_user = Some(val); }
            _ => {}
        }
    }
    flush(&mut map, &mut current_alias, &mut current_hostname, &mut current_user);
    map
}
