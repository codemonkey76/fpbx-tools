mod tui;

use anyhow::Result;
use fpbx_tui_shared::run_tui;

use tui::app::App;

fn main() -> Result<()> {
    let app = run_tui("backup.log", App::new(), tui::ui::draw)?;
    for path in app.bundle_paths() {
        println!("\nBackup complete: {}", path.display());
    }
    Ok(())
}
