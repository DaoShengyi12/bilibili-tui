use bilibili_tui::app::App;
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
};
use ratatui::DefaultTerminal;
use std::io;

#[tokio::main]
async fn main() -> io::Result<()> {
    // Initialize terminal; `ratatui::init` installs a panic hook that
    // restores the terminal, and ours disables mouse capture before it.
    let mut terminal = ratatui::init();
    install_panic_hook();

    let result = run(&mut terminal).await;

    // Disable mouse capture and bracketed paste before restoring, on
    // success and on error alike.
    let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::restore();

    result
}

async fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    terminal.clear()?;

    // Must run before anything else reads stdin; see `init_image_picker`.
    bilibili_tui::presentation::tui::init_image_picker();

    execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;

    App::new().run(terminal).await
}

fn install_panic_hook() {
    let restore_terminal = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
        restore_terminal(info);
    }));
}
