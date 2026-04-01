mod tui;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io,
    time::{Duration, Instant},
};
use tracing_subscriber::EnvFilter;

use tui::app::App;

fn main() -> Result<()> {
    // File-only logging so it doesn't interfere with TUI.
    let log_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".fpbx")
        .join("backup.log");
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

    let mut app = App::new();
    let tick = Duration::from_millis(100);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| tui::ui::draw(f, &mut app))?;

        let timeout = tick.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Global quit.
                if key.code == KeyCode::Char('q')
                    && key.modifiers == KeyModifiers::NONE
                    && !app.is_running_task()
                {
                    break;
                }
                app.handle_key(key);
            }
        }

        if last_tick.elapsed() >= tick {
            app.tick();
            last_tick = Instant::now();
        }

        if app.should_quit {
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

    // Print bundle path if successful.
    if let Some(path) = app.completed_bundle_path() {
        println!("\nBackup complete: {}", path.display());
    }

    Ok(())
}
