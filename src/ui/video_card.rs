//! Shared video card components for grid display across pages

use super::Theme;
use image::DynamicImage;
use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Message for a finished cover download. `url` identifies the card it was
/// started for, so a result arriving after the list changed is dropped.
pub struct CoverResult {
    pub index: usize,
    pub url: String,
    /// `None` when the download or decode failed.
    pub protocol: Option<StatefulProtocol>,
}

/// A video card with cover image
pub struct VideoCard {
    pub bvid: Option<String>,
    pub aid: Option<i64>,
    /// Bilibili member ID of the uploader. This is not the video's aid.
    pub uploader_mid: Option<i64>,
    pub title: String,
    pub author: String,
    pub views: String,
    pub duration: String,
    pub pic_url: Option<String>,
    pub cover: Option<StatefulProtocol>,
    /// The cover download failed; it is not retried for this card.
    pub cover_failed: bool,
    /// Uploader followers, when the source API reports them.
    pub follower: Option<String>,
    /// Danmaku and reply counts. Set for videos so their stats line matches
    /// the home feed; other cards (articles, live rooms) leave it unset.
    pub counts: Option<(String, String)>,
}

impl VideoCard {
    pub fn new(
        bvid: Option<String>,
        aid: Option<i64>,
        title: String,
        author: String,
        views: String,
        duration: String,
        pic_url: Option<String>,
    ) -> Self {
        Self {
            bvid,
            aid,
            uploader_mid: None,
            title,
            author,
            views,
            duration,
            pic_url,
            cover: None,
            cover_failed: false,
            follower: None,
            counts: None,
        }
    }

    /// Show the home feed's danmaku and reply counts, "-" when unknown.
    pub fn with_counts(mut self, danmaku: Option<String>, replies: Option<String>) -> Self {
        let dash = || "-".to_string();
        self.counts = Some((danmaku.unwrap_or_else(dash), replies.unwrap_or_else(dash)));
        self
    }

    /// `with_counts` for counts the API sends as integers.
    pub fn with_count_values(self, danmaku: Option<i64>, replies: Option<i64>) -> Self {
        self.with_counts(danmaku.map(format_count), replies.map(format_count))
    }

    pub fn with_uploader_mid(mut self, uploader_mid: Option<i64>) -> Self {
        self.uploader_mid = uploader_mid;
        self
    }

    /// Render a single video card
    pub fn render(&mut self, frame: &mut Frame, area: Rect, is_selected: bool, theme: &Theme) {
        // Enhanced border styling - use Bilibili pink for selection
        let (border_style, border_type) = if is_selected {
            (
                Style::default()
                    .fg(theme.bilibili_pink)
                    .add_modifier(Modifier::BOLD),
                BorderType::Rounded,
            )
        } else {
            (
                Style::default().fg(theme.border_subtle),
                BorderType::Rounded,
            )
        };

        // Card title shows selection indicator
        let title_span = if is_selected {
            Span::styled(
                " ▶ ",
                Style::default()
                    .fg(theme.bilibili_pink)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(border_type)
            .border_style(border_style)
            .title(title_span);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let card_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),    // Cover
                Constraint::Length(3), // Info (3 lines: title, author, metadata)
            ])
            .split(inner);

        // Cover area - center the image horizontally
        let cover_area = card_chunks[0];

        // Calculate centered cover area (assuming 16:9 aspect ratio for video covers)
        let target_width = cover_area.width.saturating_sub(2);
        let centered_cover = Rect {
            x: cover_area.x + (cover_area.width.saturating_sub(target_width)) / 2,
            y: cover_area.y,
            width: target_width,
            height: cover_area.height,
        };

        if let Some(ref mut cover) = self.cover {
            let image_widget = StatefulImage::new();
            frame.render_stateful_widget(image_widget, centered_cover, cover);
        } else {
            // Modern placeholder with subtle styling
            let placeholder = Paragraph::new(if self.cover_failed { "无封面" } else { "" })
                .style(Style::default().fg(theme.fg_muted))
                .alignment(Alignment::Center);
            frame.render_widget(placeholder, cover_area);
        }

        // Video info with improved hierarchy
        let info_area = card_chunks[1];
        let max_title_width = (info_area.width as usize).saturating_sub(2);
        let display_title = super::text::truncate_to_width(&self.title, max_title_width, "…");

        // Title styling - selected items get primary color and bold
        let title_style = if is_selected {
            Style::default()
                .fg(theme.fg_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_secondary)
        };

        let info_text = Text::from(vec![
            Line::from(Span::styled(&display_title, title_style)),
            Line::from(Span::styled(
                &self.author,
                Style::default().fg(theme.bilibili_cyan),
            )),
            Line::from(vec![
                Span::styled(&self.views, Style::default().fg(theme.fg_muted)),
                Span::styled(" · ", Style::default().fg(theme.fg_muted)),
                Span::styled(&self.duration, Style::default().fg(theme.success)),
            ]),
        ]);

        let info = Paragraph::new(info_text)
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center);
        frame.render_widget(info, info_area);
    }

    fn render_list(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        is_selected: bool,
        pending: bool,
        theme: &Theme,
    ) {
        let placeholder = if pending {
            "…"
        } else if self.cover_failed {
            "无封面"
        } else {
            ""
        };
        let content = CardContent {
            title: &self.title,
            author: &self.author,
            follower: self.follower.as_deref(),
            views: &self.views,
            counts: self
                .counts
                .as_ref()
                .map(|(danmaku, replies)| (danmaku.as_str(), replies.as_str())),
            duration: &self.duration,
        };
        render_wide_card(
            frame,
            area,
            self.cover.as_mut(),
            placeholder,
            &content,
            is_selected,
            theme,
        );
    }
}

/// Count with the 万 unit used across the client, e.g. "12.3万".
pub fn format_count(value: i64) -> String {
    if value >= 10_000 {
        format!("{:.1}万", value as f64 / 10_000.0)
    } else {
        value.to_string()
    }
}

/// Text of a wide video card, laid out like the home feed.
pub struct CardContent<'a> {
    pub title: &'a str,
    pub author: &'a str,
    /// Shown as "N 粉丝" after the author when known.
    pub follower: Option<&'a str>,
    /// Play count for videos; a label such as "图文" for other cards.
    pub views: &'a str,
    /// Danmaku and reply counts. With them the stats line carries icons
    /// like the home feed; without them it shows `views` as a plain label.
    pub counts: Option<(&'a str, &'a str)>,
    pub duration: &'a str,
}

/// The home feed's card: cover on the left, then the title, the author and
/// a stats line. Every wide video list draws its cards with this.
pub fn render_wide_card(
    frame: &mut Frame,
    area: Rect,
    cover: Option<&mut StatefulProtocol>,
    placeholder: &str,
    content: &CardContent,
    is_selected: bool,
    theme: &Theme,
) {
    let border_style = if is_selected {
        Style::default()
            .fg(theme.border_focused)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.border_unfocused)
    };
    let title_span = if is_selected {
        Span::styled(
            " ▶ ",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title_span);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Covers are 16:9 and terminal cells about 1:2, so a cover as tall
    // as the card is about 3.5 cells wide per row.
    let cover_width = (inner.height * 7 / 2).min(inner.width / 2);
    let [cover_area, _, info_area] = Layout::horizontal([
        Constraint::Length(cover_width),
        Constraint::Length(1),
        Constraint::Min(20),
    ])
    .areas(inner);
    if let Some(cover) = cover {
        frame.render_stateful_widget(StatefulImage::new(), cover_area, cover);
    } else {
        frame.render_widget(
            Paragraph::new(placeholder)
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center),
            cover_area,
        );
    }

    let title_style = if is_selected {
        Style::default()
            .fg(theme.fg_primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_secondary)
    };
    let meta_style = Style::default().fg(theme.fg_secondary);
    let title_lines = usize::from(info_area.height.saturating_sub(2)).max(1);
    let mut lines: Vec<Line> =
        clamp_lines(content.title, usize::from(info_area.width), title_lines)
            .into_iter()
            .map(|line| Line::styled(line, title_style))
            .collect();
    lines.resize(title_lines, Line::raw(""));

    let mut author = vec![Span::styled(
        content.author,
        Style::default().fg(theme.bilibili_cyan),
    )];
    if let Some(follower) = content.follower {
        author.push(Span::styled(format!("  {follower} 粉丝"), meta_style));
    }
    lines.push(Line::from(author));

    let mut stats = Vec::new();
    match content.counts {
        Some((danmaku, replies)) => {
            stats.push(Span::styled(
                format!("{} {}", super::icons::PLAY, content.views),
                meta_style,
            ));
            stats.push(Span::styled(
                format!("  {} {danmaku}", super::icons::DANMAKU),
                meta_style,
            ));
            stats.push(Span::styled(
                format!("  {} {replies}", super::icons::COMMENT),
                meta_style,
            ));
        }
        None => stats.push(Span::styled(content.views, meta_style)),
    }
    if !content.duration.is_empty() {
        stats.push(Span::styled(
            format!("  {}", content.duration),
            Style::default().fg(theme.success),
        ));
    }
    lines.push(Line::from(stats));
    frame.render_widget(Paragraph::new(lines), info_area);
}

/// `text` wrapped to `width` in at most `max_lines` lines, the last one
/// ending in "…" when text was cut.
pub fn clamp_lines(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut lines = super::text::wrap_to_width(text, width);
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            *last = super::text::truncate_to_width(&format!("{last}…"), width, "…");
        }
    }
    lines
}

/// Narrowest list card (cover left, text right). The home feed and every
/// other list of videos share it so their cards have the same size.
pub const LIST_CARD_MIN_WIDTH: u16 = 64;
/// Height of a list card, shared by the home feed and other video lists.
pub const LIST_CARD_HEIGHT: u16 = 8;
/// Width of the menu pane left of the home feed and similar list pages.
pub const MENU_PANE_WIDTH: u16 = 32;
/// Poster cards (bangumi covers) are narrow and tall.
pub const POSTER_CARD_MIN_WIDTH: u16 = 26;
const POSTER_CARD_HEIGHT: u16 = 16;
/// More columns than this makes rows of wide cards hard to scan.
const MAX_COLUMNS: usize = 4;
const MAX_POSTER_COLUMNS: usize = 8;

/// How many cards of at least `min_width` fit side by side in `width`.
pub fn columns_for_width(width: u16, min_width: u16) -> usize {
    let max = if min_width <= POSTER_CARD_MIN_WIDTH {
        MAX_POSTER_COLUMNS
    } else {
        MAX_COLUMNS
    };
    usize::from(width / min_width.max(1)).clamp(1, max)
}

/// Width of one list card when a list page `page_width` wide (menu pane
/// included) lays its cards out in as many columns as fit.
pub fn list_card_width(page_width: u16) -> u16 {
    let feed_width = page_width.saturating_sub(MENU_PANE_WIDTH);
    let columns = columns_for_width(feed_width, LIST_CARD_MIN_WIDTH);
    feed_width / u16::try_from(columns).unwrap_or(1).max(1)
}

/// Video card grid manager for async cover loading
pub struct VideoCardGrid {
    pub cards: Vec<VideoCard>,
    pub selected_index: usize,
    pub scroll_row: usize,
    pub columns: usize,
    pub card_height: u16,
    pub picker: Arc<Picker>,
    pub cover_tx: mpsc::Sender<CoverResult>,
    pub cover_rx: mpsc::Receiver<CoverResult>,
    pub pending_downloads: HashSet<usize>,
    pub cached_visible_rows: usize,
    pub list_layout: bool,
    /// When set, `columns` follows the drawn width: as many cards as fit
    /// at this width, at least one.
    pub min_card_width: Option<u16>,
    /// Where each visible card was drawn last frame, for mouse hit-testing.
    /// Kept so hit-testing matches exactly what was drawn.
    card_areas: Vec<(usize, Rect)>,
}

impl VideoCardGrid {
    pub fn new() -> Self {
        let picker = super::image_picker();
        let (cover_tx, cover_rx) = mpsc::channel(32);

        Self {
            cards: Vec::new(),
            selected_index: 0,
            scroll_row: 0,
            columns: 3,
            card_height: POSTER_CARD_HEIGHT,
            picker,
            cover_tx,
            cover_rx,
            pending_downloads: HashSet::new(),
            cached_visible_rows: 3,
            list_layout: false,
            min_card_width: Some(POSTER_CARD_MIN_WIDTH),
            card_areas: Vec::new(),
        }
    }

    /// Wide list cards (cover left, text right) that fill a wide pane with
    /// several columns.
    pub fn new_list() -> Self {
        let mut grid = Self::new();
        grid.columns = 1;
        grid.card_height = LIST_CARD_HEIGHT;
        grid.list_layout = true;
        grid.min_card_width = Some(LIST_CARD_MIN_WIDTH);
        grid
    }

    /// Switch between wide list cards and poster cards (bangumi covers),
    /// keeping the cards.
    pub fn set_poster_layout(&mut self, poster: bool) {
        if self.list_layout != poster {
            return;
        }
        self.list_layout = !poster;
        if poster {
            self.card_height = POSTER_CARD_HEIGHT;
            self.min_card_width = Some(POSTER_CARD_MIN_WIDTH);
        } else {
            self.card_height = LIST_CARD_HEIGHT;
            self.min_card_width = Some(LIST_CARD_MIN_WIDTH);
        }
        self.scroll_row = self.selected_row();
    }

    /// Fit the column count to `width`, keeping the selected card.
    pub fn fit_columns(&mut self, width: u16) {
        let Some(min_width) = self.min_card_width else {
            return;
        };
        let columns = columns_for_width(width, min_width);
        if columns != self.columns {
            self.columns = columns;
            self.scroll_row = self.selected_row();
        }
    }

    pub fn clear(&mut self) {
        self.cards.clear();
        self.selected_index = 0;
        self.scroll_row = 0;
        self.pending_downloads.clear();
        self.card_areas.clear();
    }

    pub fn add_card(&mut self, card: VideoCard) {
        self.cards.push(card);
    }

    pub fn visible_rows(&self, height: u16) -> usize {
        let available_height = height.saturating_sub(1);
        (available_height / self.card_height).max(1) as usize
    }

    pub fn selected_row(&self) -> usize {
        self.selected_index / self.columns
    }

    pub fn total_rows(&self) -> usize {
        self.cards.len().div_ceil(self.columns)
    }

    pub fn update_scroll(&mut self, visible_rows: usize) {
        let current_row = self.selected_row();
        if current_row < self.scroll_row {
            self.scroll_row = current_row;
        } else if current_row >= self.scroll_row + visible_rows {
            self.scroll_row = current_row - visible_rows + 1;
        }
    }

    /// Move one row down. From the row above a short last row, the
    /// selection lands on the last card instead of staying put.
    pub fn move_down(&mut self) -> bool {
        let Some(last_index) = self.cards.len().checked_sub(1) else {
            return false;
        };
        let columns = self.columns.max(1);
        if self.selected_index / columns >= last_index / columns {
            return false;
        }
        self.selected_index = (self.selected_index + columns).min(last_index);
        self.update_scroll(self.cached_visible_rows);
        true
    }

    /// Whether the selection is in the leftmost column, where Left hands
    /// the focus to a menu beside the grid.
    pub fn at_first_column(&self) -> bool {
        self.selected_index.is_multiple_of(self.columns.max(1))
    }

    /// Select the first card.
    pub fn move_first(&mut self) -> bool {
        if self.cards.is_empty() || self.selected_index == 0 {
            return false;
        }
        self.selected_index = 0;
        self.update_scroll(self.cached_visible_rows);
        true
    }

    /// Select the last loaded card.
    pub fn move_last(&mut self) -> bool {
        let Some(last_index) = self.cards.len().checked_sub(1) else {
            return false;
        };
        if self.selected_index == last_index {
            return false;
        }
        self.selected_index = last_index;
        self.update_scroll(self.cached_visible_rows);
        true
    }

    pub fn move_up(&mut self) -> bool {
        if !self.cards.is_empty() && self.selected_index >= self.columns {
            self.selected_index -= self.columns;
            self.update_scroll(self.cached_visible_rows);
            return true;
        }
        false
    }

    pub fn move_right(&mut self) -> bool {
        if !self.cards.is_empty() && self.selected_index + 1 < self.cards.len() {
            self.selected_index += 1;
            self.update_scroll(self.cached_visible_rows);
            return true;
        }
        false
    }

    pub fn move_left(&mut self) -> bool {
        if !self.cards.is_empty() && self.selected_index > 0 {
            self.selected_index -= 1;
            self.update_scroll(self.cached_visible_rows);
            return true;
        }
        false
    }

    /// Move the selection by one visible viewport.
    pub fn move_page_down(&mut self) -> bool {
        self.move_page(true)
    }

    /// Move the selection backwards by one visible viewport.
    pub fn move_page_up(&mut self) -> bool {
        self.move_page(false)
    }

    fn move_page(&mut self, down: bool) -> bool {
        let Some(last_index) = self.cards.len().checked_sub(1) else {
            return false;
        };
        let page_size = self
            .cached_visible_rows
            .max(1)
            .saturating_mul(self.columns.max(1));
        let old_index = self.selected_index;
        self.selected_index = if down {
            old_index.saturating_add(page_size).min(last_index)
        } else {
            old_index.saturating_sub(page_size)
        };
        if self.selected_index == old_index {
            return false;
        }
        self.update_scroll(self.cached_visible_rows.max(1));
        true
    }

    /// Check if near bottom for pagination
    /// Whether the selection is within the last rows of what is loaded. A
    /// list that fits on one screen counts too, so it still loads more.
    pub fn is_near_bottom(&self) -> bool {
        if self.cards.is_empty() {
            return false;
        }
        let current_row = self.selected_row();
        let total = self.total_rows();
        current_row + 2 >= total.saturating_sub(1)
    }

    /// Start background downloads for visible covers
    pub fn start_cover_downloads(&mut self) {
        if self.cards.is_empty() {
            return;
        }

        let start = self.scroll_row * self.columns;
        // Prefetch all visible rows plus 2 extra rows for smooth scrolling
        let prefetch_rows = self.cached_visible_rows + 2;
        let end = (start + self.columns * prefetch_rows).min(self.cards.len());

        for idx in start..end {
            let card = &self.cards[idx];
            if card.cover.is_some() || card.cover_failed || self.pending_downloads.contains(&idx) {
                continue;
            }

            if let Some(pic_url) = card.pic_url.clone() {
                self.pending_downloads.insert(idx);
                let tx = self.cover_tx.clone();
                let picker = Arc::clone(&self.picker);

                tokio::spawn(async move {
                    let protocol = download_image(&pic_url)
                        .await
                        .map(|img| picker.new_resize_protocol(img));
                    let _ = tx
                        .send(CoverResult {
                            index: idx,
                            url: pic_url,
                            protocol,
                        })
                        .await;
                });
            }
        }
    }

    /// Poll for completed cover downloads
    pub fn poll_cover_results(&mut self) {
        while let Ok(result) = self.cover_rx.try_recv() {
            self.apply_cover_result(result);
        }
    }

    /// Store a finished cover on the card it was requested for. Results for
    /// a card that has since been replaced (new search, folder, tab) are
    /// dropped instead of landing on whatever card now has that index.
    fn apply_cover_result(&mut self, result: CoverResult) {
        let Some(card) = self
            .cards
            .get_mut(result.index)
            .filter(|card| card.pic_url.as_deref() == Some(result.url.as_str()))
        else {
            return;
        };
        self.pending_downloads.remove(&result.index);
        match result.protocol {
            Some(protocol) => card.cover = Some(protocol),
            None => card.cover_failed = true,
        }
    }

    /// Render the grid
    pub fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.fit_columns(area.width);
        let visible_rows = self.visible_rows(area.height);
        self.cached_visible_rows = visible_rows;
        self.update_scroll(visible_rows);

        let row_constraints: Vec<Constraint> = (0..visible_rows)
            .map(|_| Constraint::Length(self.card_height))
            .collect();

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints)
            .split(area);

        let mut card_areas = std::mem::take(&mut self.card_areas);
        card_areas.clear();

        for (row_offset, row_area) in rows.iter().enumerate() {
            let actual_row = self.scroll_row + row_offset;
            let start_idx = actual_row * self.columns;

            if start_idx >= self.cards.len() {
                break;
            }

            let col_constraints: Vec<Constraint> = (0..self.columns)
                .map(|_| Constraint::Ratio(1, self.columns as u32))
                .collect();

            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(col_constraints)
                .split(*row_area);

            for (col_idx, col_area) in cols.iter().enumerate() {
                let video_idx = start_idx + col_idx;
                if video_idx >= self.cards.len() {
                    break;
                }
                card_areas.push((video_idx, *col_area));
            }
        }

        for &(video_idx, col_area) in &card_areas {
            let is_selected = video_idx == self.selected_index;
            if self.list_layout {
                let pending = self.pending_downloads.contains(&video_idx);
                self.cards[video_idx].render_list(frame, col_area, is_selected, pending, theme);
            } else {
                self.cards[video_idx].render(frame, col_area, is_selected, theme);
            }
        }
        self.card_areas = card_areas;
    }

    /// The card drawn at a screen position in the last frame.
    pub fn index_at(&self, column: u16, row: u16) -> Option<usize> {
        let position = Position::new(column, row);
        self.card_areas
            .iter()
            .find(|(_, area)| area.contains(position))
            .map(|(index, _)| *index)
            .filter(|index| *index < self.cards.len())
    }

    /// Select the card in the clicked row of a single-column list.
    pub fn select_at(&mut self, row: u16, area: Rect) -> bool {
        let Some(index) = self.index_at(area.x, row) else {
            return false;
        };
        self.selected_index = index;
        self.update_scroll(self.cached_visible_rows);
        true
    }

    pub fn selected_card(&self) -> Option<&VideoCard> {
        self.cards.get(self.selected_index)
    }
}

impl Default for VideoCardGrid {
    fn default() -> Self {
        Self::new()
    }
}

async fn download_image(url: &str) -> Option<DynamicImage> {
    let response = reqwest::get(url).await.ok()?;
    let bytes = response.bytes().await.ok()?;
    image::load_from_memory(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw_list_card(card: VideoCard) -> String {
        let mut grid = VideoCardGrid::new_list();
        grid.add_card(card);
        let theme = Theme::default();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(80, 8)).unwrap();
        terminal
            .draw(|frame| grid.render(frame, Rect::new(0, 0, 80, 8), &theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..8 {
            let mut x = 0;
            while x < 80 {
                let symbol = buffer[(x, y)].symbol();
                text.push_str(symbol);
                // A wide character also covers the next cell.
                x += u16::try_from(super::super::text::display_width(symbol).max(1)).unwrap();
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn video_cards_show_the_home_feed_stats_line() {
        let card = card_with_cover("https://i0.test/a.jpg").with_count_values(Some(12_345), None);
        let text = draw_list_card(card);
        assert!(text.contains(super::super::icons::PLAY.to_string().as_str()));
        assert!(text.contains("1.2万"), "{text}");
        assert!(text.contains('-'), "unknown replies show a dash");
        assert!(
            text.contains('▶'),
            "the selected card is marked like the home feed"
        );
    }

    #[test]
    fn non_video_cards_keep_their_label_without_count_icons() {
        let mut card = card_with_cover("https://i0.test/a.jpg");
        card.views = "专栏".to_string();
        let text = draw_list_card(card);
        assert!(text.contains("专栏"));
        assert!(!text.contains(super::super::icons::DANMAKU.to_string().as_str()));
    }

    #[test]
    fn list_grids_add_columns_as_the_pane_widens() {
        assert_eq!(columns_for_width(80, LIST_CARD_MIN_WIDTH), 1);
        assert_eq!(columns_for_width(120, LIST_CARD_MIN_WIDTH), 1);
        assert_eq!(columns_for_width(130, LIST_CARD_MIN_WIDTH), 2);
        assert_eq!(columns_for_width(400, LIST_CARD_MIN_WIDTH), MAX_COLUMNS);
        assert_eq!(columns_for_width(10, LIST_CARD_MIN_WIDTH), 1);

        let mut grid = VideoCardGrid::new_list();
        for index in 0..9 {
            grid.add_card(card_with_cover(&format!("https://i0.test/{index}.jpg")));
        }
        grid.selected_index = 5;
        grid.fit_columns(200);
        assert_eq!(grid.columns, 3);
        assert_eq!(grid.selected_row(), 1, "the selected card stays selected");
        assert_eq!(grid.selected_index, 5);
    }

    #[test]
    fn list_cards_keep_the_home_card_size_whatever_the_pane_height() {
        let theme = Theme::default();
        for height in [20u16, 27, 35] {
            let mut grid = VideoCardGrid::new_list();
            for index in 0..12 {
                grid.add_card(card_with_cover(&format!("https://i0.test/{index}.jpg")));
            }
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(140, height)).unwrap();
            terminal
                .draw(|frame| grid.render(frame, Rect::new(0, 0, 140, height), &theme))
                .unwrap();
            assert!(
                grid.card_areas
                    .iter()
                    .all(|(_, area)| area.height == LIST_CARD_HEIGHT),
                "pane height {height}"
            );
        }
    }

    #[test]
    fn long_titles_are_clamped_with_an_ellipsis() {
        let lines = clamp_lines("一二三四五六七八九十", 6, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with('…'));
        assert_eq!(clamp_lines("短", 6, 2), ["短"]);
    }

    fn card_with_cover(url: &str) -> VideoCard {
        VideoCard::new(
            None,
            None,
            url.to_string(),
            "up".to_string(),
            "0".to_string(),
            "00:00".to_string(),
            Some(url.to_string()),
        )
    }

    #[test]
    fn clicks_hit_the_card_drawn_there_after_scrolling() {
        let mut grid = VideoCardGrid::new();
        grid.columns = 2;
        grid.card_height = 8;
        for index in 0..10 {
            grid.add_card(card_with_cover(&format!("https://i0.test/{index}.jpg")));
        }
        grid.scroll_row = 2;
        // The selection is on screen, so drawing keeps the scroll position.
        grid.selected_index = 6;
        let theme = Theme::default();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(80, 30)).unwrap();
        // 29 usable rows hold three 8-row cards; the rows below stay empty.
        let area = Rect::new(10, 1, 60, 29);
        terminal
            .draw(|frame| grid.render(frame, area, &theme))
            .unwrap();

        assert_eq!(
            grid.index_at(11, 1),
            Some(4),
            "first visible row after scrolling"
        );
        assert_eq!(grid.index_at(69, 1), Some(5));
        assert_eq!(grid.index_at(11, 24), Some(8), "last visible row");
        assert_eq!(grid.index_at(11, 28), None, "cards do not stretch");
        assert_eq!(grid.index_at(5, 5), None, "left of the grid (sidebar)");

        grid.clear();
        assert_eq!(grid.index_at(11, 1), None);
    }

    #[test]
    fn cover_from_a_previous_list_does_not_land_on_the_new_list() {
        let mut grid = VideoCardGrid::new();
        grid.add_card(card_with_cover("https://i0.test/old.jpg"));
        grid.pending_downloads.insert(0);
        grid.clear();
        grid.add_card(card_with_cover("https://i0.test/new.jpg"));
        grid.pending_downloads.insert(0);

        grid.apply_cover_result(CoverResult {
            index: 0,
            url: "https://i0.test/old.jpg".to_string(),
            protocol: None,
        });
        assert!(!grid.cards[0].cover_failed);
        assert!(grid.pending_downloads.contains(&0));

        grid.apply_cover_result(CoverResult {
            index: 0,
            url: "https://i0.test/new.jpg".to_string(),
            protocol: None,
        });
        assert!(
            grid.cards[0].cover_failed,
            "a failed download is not retried"
        );
        assert!(!grid.pending_downloads.contains(&0));
    }

    #[test]
    fn page_navigation_moves_by_one_viewport() {
        let mut grid = VideoCardGrid::new_list();
        for index in 0..8 {
            grid.add_card(VideoCard::new(
                None,
                None,
                format!("video-{index}"),
                "up".to_string(),
                "0".to_string(),
                "00:00".to_string(),
                None,
            ));
        }
        grid.cached_visible_rows = 3;

        assert!(grid.move_page_down());
        assert_eq!(grid.selected_index, 3);
        assert!(grid.move_page_down());
        assert_eq!(grid.selected_index, 6);
        assert!(grid.move_page_up());
        assert_eq!(grid.selected_index, 3);
    }
}
