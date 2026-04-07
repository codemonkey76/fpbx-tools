use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use std::{
    io,
    time::{Duration, Instant},
};
use tracing_subscriber::EnvFilter;

/// Contract that every TUI application must satisfy to run inside [`run_tui`].
pub trait TuiApp {
    fn handle_key(&mut self, key: crossterm::event::KeyEvent);
    fn tick(&mut self);
    fn is_running_task(&self) -> bool;
    fn is_typing(&self) -> bool;
    fn should_quit(&self) -> bool;
}

/// Set up logging, initialise the terminal, run the event loop, then tear down.
///
/// `log_name` is the filename placed under `~/.fpbx/` (e.g. `"backup.log"`).
/// Returns the app after the loop so callers can inspect final state.
pub fn run_tui<A, D>(log_name: &str, mut app: A, mut draw_fn: D) -> Result<A>
where
    A: TuiApp,
    D: FnMut(&mut Frame, &mut A),
{
    // File-only logging so it never interferes with the TUI.
    let log_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".fpbx")
        .join(log_name);
    std::fs::create_dir_all(log_path.parent().unwrap()).ok();
    let log_file = std::fs::File::create(&log_path)?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .with_writer(log_file)
        .init();

    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let tick = Duration::from_millis(100);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| draw_fn(f, &mut app))?;

        let timeout = tick.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
        {
            if key.code == KeyCode::Char('q')
                && key.modifiers == KeyModifiers::NONE
                && !app.is_running_task()
                && !app.is_typing()
            {
                break;
            }
            app.handle_key(key);
        }

        if last_tick.elapsed() >= tick {
            app.tick();
            last_tick = Instant::now();
        }

        if app.should_quit() {
            break;
        }
    }

    // Restore terminal.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(app)
}
