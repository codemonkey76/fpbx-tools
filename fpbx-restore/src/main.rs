mod tui;

use anyhow::Result;
use fpbx_tui_shared::run_tui;

use tui::app::App;

fn main() -> Result<()> {
    let app = run_tui("restore.log", App::new(), tui::ui::draw)?;
    if app.restore_succeeded {
        println!("\nRestore complete.");
    }
    Ok(())
}
