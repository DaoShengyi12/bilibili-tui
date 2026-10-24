mod article_detail;
mod bangumi;
mod bangumi_detail;
mod dynamic;
mod dynamic_detail;
mod favorites;
mod history;
mod home;
mod live;
mod live_detail;
mod login;
mod search;
mod settings;
mod sidebar;
pub mod theme;
mod up;
mod video_card;
mod video_detail;

pub use article_detail::ArticleDetailPage;
pub use bangumi::BangumiPage;
pub use bangumi_detail::BangumiDetailPage;
pub use dynamic::{DynamicPage, DynamicTab};
pub use dynamic_detail::DynamicDetailPage;
pub use favorites::FavoritesPage;
pub use history::HistoryPage;
pub use home::HomePage;
pub use live::LivePage;
pub use live_detail::LiveDetailPage;
pub use login::LoginPage;
pub use search::SearchPage;
pub use settings::SettingsPage;
pub use sidebar::{NavItem, Sidebar};
pub use theme::{DEFAULT_THEME_ID, Theme, ThemeChoice};
pub use up::UpPage;
pub use video_card::{VideoCard, VideoCardGrid};
pub use video_detail::VideoDetailPage;

use crate::application::AppAction;
use crate::storage::Keybindings;
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyModifiers, MouseEvent},
    prelude::{Color, Line, Modifier, Rect, Span, Style},
};

static IMAGE_PICKER: std::sync::OnceLock<std::sync::Arc<ratatui_image::picker::Picker>> =
    std::sync::OnceLock::new();

/// Query the terminal's image capabilities once, before the event loop reads
/// stdin. The query reads terminal replies from stdin on a helper thread; doing
/// it while crossterm is also reading lets crossterm consume the reply, so the
/// helper times out but stays blocked in `read()` and swallows the next key.
pub fn init_image_picker() {
    use ratatui_image::picker::{Picker, cap_parser::QueryStdioOptions};
    const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

    IMAGE_PICKER.get_or_init(|| {
        let started = std::time::Instant::now();
        let picker = Picker::from_query_stdio_with_options(QueryStdioOptions {
            timeout: QUERY_TIMEOUT,
            ..QueryStdioOptions::default()
        })
        .unwrap_or_else(|_| Picker::halfblocks());
        if started.elapsed() >= QUERY_TIMEOUT {
            release_stalled_query_reader();
        }
        std::sync::Arc::new(picker)
    });
}

/// A timed-out query (for example inside tmux without `allow-passthrough`,
/// which drops the wrapped queries) leaves its helper thread blocked reading
/// stdin. Ask for a plain Device Status Report, which every terminal and tmux
/// answer natively, so that thread reads the reply and exits instead of
/// consuming the user's first key.
fn release_stalled_query_reader() {
    use std::io::Write;
    let mut stdout = std::io::stdout();
    if stdout
        .write_all(b"\x1b[5n")
        .and_then(|()| stdout.flush())
        .is_ok()
    {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Shared image picker. Falls back to halfblocks without touching stdin when
/// [`init_image_picker`] has not run (for example in tests).
pub(crate) fn image_picker() -> std::sync::Arc<ratatui_image::picker::Picker> {
    IMAGE_PICKER
        .get()
        .cloned()
        .unwrap_or_else(|| std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks()))
}

/// Build the centered, bracketed shortcut footer used across list pages.
/// Each tuple is `(shortcut, label, color)`; shortcut text is emphasized while
/// labels and brackets use the secondary foreground.
pub fn shortcut_footer(
    theme: &Theme,
    items: impl IntoIterator<Item = (String, String, Color)>,
) -> Line<'static> {
    let muted = Style::default().fg(theme.fg_secondary);
    let mut spans = Vec::new();
    for (index, (shortcut, label, color)) in items.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ", muted));
        }
        spans.push(Span::styled("[", muted));
        spans.push(Span::styled(
            shortcut,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("] ", muted));
        spans.push(Span::styled(label, muted));
    }
    Line::from(spans)
}

/// UI Component trait
pub trait Component {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings);
    fn handle_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        let _ = (key, keys);
        None
    }
    fn handle_input_with_modifiers(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        keys: &Keybindings,
    ) -> Option<AppAction> {
        let _ = modifiers;
        self.handle_input(key, keys)
    }
    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Option<AppAction> {
        let _ = (event, area);
        None
    }
}

/// Application pages
pub enum Page {
    Login(LoginPage),
    Home(HomePage),
    Search(SearchPage),
    Dynamic(DynamicPage),
    DynamicDetail(Box<DynamicDetailPage>),
    ArticleDetail(Box<ArticleDetailPage>),
    VideoDetail(Box<VideoDetailPage>),
    Up(Box<UpPage>),
    History(HistoryPage),
    Favorites(FavoritesPage),
    Live(LivePage),
    LiveDetail(Box<LiveDetailPage>),
    Settings(Box<SettingsPage>),
    Bangumi(Box<BangumiPage>),
    BangumiDetail(Box<BangumiDetailPage>),
}
