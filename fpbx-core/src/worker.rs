use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::ssh::VerifyResult;

/// State shared between the UI thread and a background worker thread.
/// The worker writes into this struct; the UI thread polls it every tick.
#[derive(Debug, Default)]
pub struct WorkerState {
    pub log: Vec<String>,
    pub progress: f64,
    pub current_task: String,
    pub done: bool,
    pub error: Option<String>,
    /// Populated by verify workers in backup and restore.
    pub verify_result: Option<VerifyResult>,
    /// Populated by the backup worker once a bundle is written.
    pub bundle_paths: Vec<PathBuf>,
}

/// A shared, mutex-guarded handle to a [`WorkerState`].
pub type WorkerSlot = Arc<Mutex<WorkerState>>;

/// Create a fresh, empty [`WorkerSlot`].
pub fn new_worker() -> WorkerSlot {
    Arc::new(Mutex::new(WorkerState::default()))
}
