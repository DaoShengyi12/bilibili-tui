use bilibili_tui::app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};
use std::io;

#[tokio::main]
async fn main() -> io::Result<()> {
    // Initialize terminal
    let mut terminal = ratatui::init();
    terminal.clear()?;

    // Must run before anything else reads stdin; see `init_image_picker`.
    bilibili_tui::presentation::tui::init_image_picker();

    // Enable mouse capture
    execute!(std::io::stdout(), EnableMouseCapture)?;

    // Run the application
    let app = App::new();
    let result = app.run(&mut terminal).await;

    // Disable mouse capture before restoring
    let _ = execute!(std::io::stdout(), DisableMouseCapture);

    // Restore terminal
    ratatui::restore();

    result
}
