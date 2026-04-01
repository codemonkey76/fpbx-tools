use crossterm::event::{KeyCode, KeyEvent};
use fpbx_core::bundle::{default_backup_dir, list_bundles, BundleManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
}

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,
    pub restore_succeeded: bool,

    // Bundle picker.
    pub bundle_dir: PathBuf,
    pub bundles: Vec<(PathBuf, BundleManifest)>,
    pub bundle_list_state: ratatui::widgets::ListState,

    // Selected bundle.
    pub selected_manifest: Option<BundleManifest>,
    pub selected_bundle_path: Option<PathBuf>,

    // Server screen.
    pub host_input: String,
    pub user_input: String,
    pub active_field: usize,

    // Progress.
    pub worker: Option<Arc<Mutex<WorkerState>>>,
}

impl App {
    pub fn new() -> Self {
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(0));
        let bundle_dir = default_backup_dir();
        let bundles = list_bundles(&bundle_dir).unwrap_or_default();
        Self {
            screen: AppScreen::BundlePicker,
            should_quit: false,
            restore_succeeded: false,
            bundle_dir,
            bundles,
            bundle_list_state: list_state,
            selected_manifest: None,
            selected_bundle_path: None,
            host_input: String::new(),
            user_input: whoami_user(),
            active_field: 0,
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

    pub fn tick(&mut self) {
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
            KeyCode::Enter => {
                if let Some(i) = self.bundle_list_state.selected() {
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
            KeyCode::Enter => self.screen = AppScreen::Server,
            KeyCode::Esc => self.screen = AppScreen::BundlePicker,
            _ => {}
        }
    }

    fn handle_server_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => self.active_field = 1 - self.active_field,
            KeyCode::Char(c) => {
                if self.active_field == 0 { self.host_input.push(c); }
                else { self.user_input.push(c); }
            }
            KeyCode::Backspace => {
                if self.active_field == 0 { self.host_input.pop(); }
                else { self.user_input.pop(); }
            }
            KeyCode::Enter => self.screen = AppScreen::Confirm,
            KeyCode::Esc => self.screen = AppScreen::Preview,
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                // TODO: start restore worker
                self.screen = AppScreen::Progress;
            }
            KeyCode::Char('n') | KeyCode::Esc => self.screen = AppScreen::Server,
            _ => {}
        }
    }
}

fn whoami_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "root".to_string())
}
