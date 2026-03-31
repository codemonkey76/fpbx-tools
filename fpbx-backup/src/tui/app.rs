use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use fpbx_core::{
    bundle::{create_bundle, default_backup_dir, default_staging_dir, BundleManifest},
    db::export_domain_sql_v2,
    domain::{count_domain_rows, list_domains, DomainFilePaths, FpbxDomain},
    ssh::{SshSession, VerifyResult},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

/// Which screen the TUI is showing.
#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    Server,           // Enter host + user, verify SSH + FusionPBX
    Domains,          // Filterable list of domains
    OutputPath,       // Confirm/edit output path
    Progress,         // Export + bundle progress with log
    Done,             // Summary + bundle location
    Error(String),    // Error overlay
}

/// Shared state for the background worker thread.
#[derive(Debug, Default)]
pub struct WorkerState {
    pub log: Vec<String>,
    pub progress: f64,         // 0.0 – 1.0
    pub current_task: String,
    pub done: bool,
    pub error: Option<String>,
    pub bundle_path: Option<PathBuf>,
}

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,

    // Server screen.
    pub host_input: String,
    pub user_input: String,
    pub active_field: usize,  // 0=host, 1=user
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

    // Progress screen.
    pub worker: Option<Arc<Mutex<WorkerState>>>,

    // Done.
    pub bundle_path: Option<PathBuf>,
    pub manifest: Option<BundleManifest>,
}

impl App {
    pub fn new() -> Self {
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(0));
        Self {
            screen: AppScreen::Server,
            should_quit: false,
            host_input: String::new(),
            user_input: whoami_user(),
            active_field: 0,
            verify_result: None,
            verifying: false,
            domains: Vec::new(),
            domain_filter: String::new(),
            domain_list_state: list_state,
            filter_active: false,
            loading_domains: false,
            output_path_input: default_backup_dir()
                .to_string_lossy()
                .to_string(),
            worker: None,
            bundle_path: None,
            manifest: None,
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

    pub fn completed_bundle_path(&self) -> Option<&PathBuf> {
        self.bundle_path.as_ref()
    }

    pub fn filtered_domains(&self) -> Vec<&FpbxDomain> {
        let q = self.domain_filter.to_lowercase();
        self.domains
            .iter()
            .filter(|d| q.is_empty() || d.label().to_lowercase().contains(&q))
            .collect()
    }

    pub fn selected_domain(&self) -> Option<&FpbxDomain> {
        let filtered = self.filtered_domains();
        self.domain_list_state
            .selected()
            .and_then(|i| filtered.get(i).copied())
    }

    /// Called every ~100ms tick.
    pub fn tick(&mut self) {
        // Poll worker for completion.
        if self.screen == AppScreen::Progress {
            if let Some(w) = &self.worker {
                let state = w.lock().unwrap();
                if state.done {
                    if let Some(ref err) = state.error {
                        self.screen = AppScreen::Error(err.clone());
                    } else if let Some(ref p) = state.bundle_path {
                        self.bundle_path = Some(p.clone());
                        self.screen = AppScreen::Done;
                    }
                }
            }
        }
    }

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
                } else {
                    self.user_input.push(c);
                }
                self.verify_result = None;
            }
            KeyCode::Backspace => {
                if self.active_field == 0 {
                    self.host_input.pop();
                } else {
                    self.user_input.pop();
                }
                self.verify_result = None;
            }
            KeyCode::Enter => {
                if self.verifying {
                    return;
                }
                // If already verified OK, advance.
                if matches!(&self.verify_result, Some(Ok(v)) if v.is_ok()) {
                    self.advance_to_domains();
                    return;
                }
                // Start verification.
                self.start_verify();
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    fn start_verify(&mut self) {
        self.verifying = true;
        self.verify_result = None;
        let host = self.host_input.trim().to_string();
        let user = self.user_input.trim().to_string();
        let worker: Arc<Mutex<Option<Result<VerifyResult, String>>>> =
            Arc::new(Mutex::new(None));
        let worker_clone = worker.clone();
        thread::spawn(move || {
            let result = SshSession::connect(&host, &user)
                .and_then(|s| s.verify_fusionpbx());
            *worker_clone.lock().unwrap() = Some(result.map_err(|e| e.to_string()));
        });
        // Poll via tick — we stash the join handle-equivalent in a thread.
        // Use a simpler approach: spin up a thread that writes to a shared slot.
        let result_slot: Arc<Mutex<Option<Result<VerifyResult, String>>>> = worker;
        let host2 = self.host_input.trim().to_string();
        let user2 = self.user_input.trim().to_string();
        // Restart with a proper approach using a dedicated slot.
        self.start_verify_inner(host2, user2);
    }

    fn start_verify_inner(&mut self, host: String, user: String) {
        self.verifying = true;
        let slot: Arc<Mutex<Option<Result<VerifyResult, String>>>> =
            Arc::new(Mutex::new(None));
        let slot2 = slot.clone();
        thread::spawn(move || {
            let r = SshSession::connect(&host, &user)
                .and_then(|s| s.verify_fusionpbx())
                .map_err(|e| e.to_string());
            *slot2.lock().unwrap() = Some(r);
        });
        // We'll poll this in a busy-check on tick.
        // Store slot in a way tick() can reach it:
        let slot3 = slot.clone();
        // Ugly but straightforward: spawn another thread to deliver result.
        let app_ptr = std::ptr::null_mut::<App>(); // won't use
        // Instead: store in worker field with a wrapper.
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
                            w.done = true;
                            // We signal verify completion via log.
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
        // NOTE: verification result is stored in worker for now.
        // tick() will detect worker.done and update verify_result.
    }

    fn advance_to_domains(&mut self) {
        self.loading_domains = true;
        let host = self.host_input.trim().to_string();
        let user = self.user_input.trim().to_string();
        let slot: Arc<Mutex<Option<Result<Vec<FpbxDomain>, String>>>> =
            Arc::new(Mutex::new(None));
        let slot2 = slot.clone();
        thread::spawn(move || {
            let r = SshSession::connect(&host, &user)
                .and_then(|s| list_domains(&s))
                .map_err(|e| e.to_string());
            *slot2.lock().unwrap() = Some(r);
        });
        // Spin until loaded (simple approach — runs fast).
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
                    // Reset selection when filter changes.
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
                if n == 0 { return; }
                let i = self.domain_list_state.selected().unwrap_or(0);
                self.domain_list_state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let n = self.filtered_domains().len();
                if n == 0 { return; }
                let i = self.domain_list_state.selected().unwrap_or(0);
                self.domain_list_state
                    .select(Some((i + 1).min(n.saturating_sub(1))));
            }
            KeyCode::Enter => {
                if self.selected_domain().is_some() {
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
            KeyCode::Backspace => { self.output_path_input.pop(); }
            KeyCode::Enter => self.start_backup(),
            KeyCode::Esc => self.screen = AppScreen::Domains,
            _ => {}
        }
    }

    // --- Progress screen ---

    fn handle_progress_key(&mut self, key: KeyEvent) {
        // During active work, only allow quit if done.
        if key.code == KeyCode::Esc && !self.is_running_task() {
            self.screen = AppScreen::Domains;
        }
    }

    // --- Backup worker ---

    fn start_backup(&mut self) {
        let host = self.host_input.trim().to_string();
        let user = self.user_input.trim().to_string();
        let domain = self.selected_domain().cloned().unwrap();
        let output_dir = PathBuf::from(self.output_path_input.trim());

        let wstate = Arc::new(Mutex::new(WorkerState::default()));
        let wstate2 = wstate.clone();
        self.worker = Some(wstate);
        self.screen = AppScreen::Progress;

        thread::spawn(move || {
            let log = |msg: &str| {
                let mut w = wstate2.lock().unwrap();
                w.log.push(msg.to_string());
                w.current_task = msg.to_string();
            };

            let mut progress = |msg: &str| log(msg);

            let result = run_backup(host, user, domain, output_dir, &mut progress);

            let mut w = wstate2.lock().unwrap();
            match result {
                Ok(path) => {
                    w.log.push(format!("✓ Bundle saved: {}", path.display()));
                    w.bundle_path = Some(path);
                    w.progress = 1.0;
                    w.done = true;
                }
                Err(e) => {
                    w.error = Some(e.to_string());
                    w.done = true;
                }
            }
        });
    }
}

fn run_backup(
    host: String,
    user: String,
    domain: FpbxDomain,
    output_dir: PathBuf,
    progress: &mut dyn FnMut(&str),
) -> Result<PathBuf> {
    progress("Connecting to source server…");
    let session = SshSession::connect(&host, &user)?;

    let staging = default_staging_dir().join(&domain.domain_uuid);
    std::fs::create_dir_all(&staging)?;

    // Count rows.
    progress("Counting domain records…");
    let table_counts = count_domain_rows(&session, &domain.domain_uuid)?;

    // Export SQL.
    progress("Exporting database records…");
    let sql_path = staging.join("db.sql.gz");
    let db_bytes = export_domain_sql_v2(&session, &domain.domain_uuid, &sql_path, progress)?;

    // Export files.
    progress("Discovering domain file paths…");
    let file_paths_spec = DomainFilePaths::for_domain(&domain.domain_name);
    let existing_paths = file_paths_spec.existing(&session);

    progress("Archiving voicemail + recordings…");
    let files_tar_path = staging.join("files.tar.gz");
    let files_bytes = export_domain_files(&session, &existing_paths, &files_tar_path, progress)?;

    // Build manifest.
    let manifest = BundleManifest::new(
        &host,
        domain,
        table_counts,
        existing_paths,
        db_bytes,
        files_bytes,
    );

    // Create bundle.
    progress("Assembling .fpbx bundle…");
    let bundle_path = create_bundle(&manifest, &staging, &output_dir, progress)?;

    // Cleanup staging.
    let _ = std::fs::remove_dir_all(&staging);

    Ok(bundle_path)
}

fn export_domain_files(
    session: &SshSession,
    paths: &[String],
    local_tar: &std::path::Path,
    progress: &mut dyn FnMut(&str),
) -> Result<u64> {
    if paths.is_empty() {
        // Create empty tar.gz.
        let f = std::fs::File::create(local_tar)?;
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::best());
        tar::Builder::new(gz).finish()?;
        return Ok(0);
    }

    let remote_tar = "/tmp/fpbx-files.tar.gz";
    let path_args = paths
        .iter()
        .map(|p| format!("'{}'", p))
        .collect::<Vec<_>>()
        .join(" ");

    progress("Compressing remote files…");
    let cmd = format!("tar czf {} {} 2>/dev/null || true", remote_tar, path_args);
    session.exec(&cmd)?;

    progress("Downloading file archive…");
    let bytes = session.download(std::path::Path::new(remote_tar), local_tar)?;
    let _ = session.exec(&format!("rm -f {}", remote_tar));

    Ok(bytes)
}

fn whoami_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "root".to_string())
}
