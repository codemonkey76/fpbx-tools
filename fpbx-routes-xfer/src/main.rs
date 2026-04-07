mod tui;

use anyhow::Result;
use fpbx_tui_shared::run_tui;

use tui::app::App;

fn main() -> Result<()> {
    run_tui("routes-xfer.log", App::new(), tui::ui::draw)?;
    Ok(())
}
