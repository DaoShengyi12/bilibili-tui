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
        }
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
            let placeholder = Paragraph::new("📺")
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

    fn render_list(&mut self, frame: &mut Frame, area: Rect, is_selected: bool, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if is_selected {
                theme.bilibili_pink
            } else {
                theme.border_subtle
            }));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Min(24)])
            .split(inner);
        if let Some(cover) = self.cover.as_mut() {
            frame.render_stateful_widget(StatefulImage::new(), chunks[0], cover);
        } else {
            frame.render_widget(Paragraph::new("📺").alignment(Alignment::Center), chunks[0]);
        }
        let style = if is_selected {
            Style::default()
                .fg(theme.fg_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_secondary)
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(self.title.clone(), style),
                Line::styled(
                    self.author.clone(),
                    Style::default().fg(theme.bilibili_cyan),
                ),
                Line::styled(
                    format!("▶ {}   {}", self.views, self.duration),
                    Style::default().fg(theme.fg_muted),
                ),
            ])
            .wrap(Wrap { trim: true }),
            chunks[1],
        );
    }
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
    /// Where each visible card was drawn last frame, for mouse hit-testing.
    /// Rows stretch beyond `card_height`, so positions cannot be computed.
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
            card_height: 12,
            picker,
            cover_tx,
            cover_rx,
            pending_downloads: HashSet::new(),
            cached_visible_rows: 3,
            list_layout: false,
            card_areas: Vec::new(),
        }
    }

    pub fn new_list() -> Self {
        let mut grid = Self::new();
        grid.columns = 1;
        grid.card_height = 7;
        grid.list_layout = true;
        grid
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

    pub fn move_down(&mut self) -> bool {
        if !self.cards.is_empty() {
            let new_idx = self.selected_index + self.columns;
            if new_idx < self.cards.len() {
                self.selected_index = new_idx;
                self.update_scroll(self.cached_visible_rows);
                return true;
            }
        }
        false
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
    pub fn is_near_bottom(&self, visible_rows: usize) -> bool {
        if self.cards.is_empty() {
            return false;
        }
        let current_row = self.selected_row();
        let total = self.total_rows();
        current_row + 2 >= total.saturating_sub(1) && total > visible_rows
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
        let visible_rows = self.visible_rows(area.height);
        self.cached_visible_rows = visible_rows;

        let row_constraints: Vec<Constraint> = (0..visible_rows)
            .map(|_| Constraint::Min(self.card_height))
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
                self.cards[video_idx].render_list(frame, col_area, is_selected, theme);
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
    fn clicks_hit_the_card_drawn_there_including_stretched_rows_and_scroll() {
        let mut grid = VideoCardGrid::new();
        grid.columns = 2;
        grid.card_height = 8;
        for index in 0..10 {
            grid.add_card(card_with_cover(&format!("https://i0.test/{index}.jpg")));
        }
        grid.scroll_row = 2;
        let theme = Theme::default();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(80, 30)).unwrap();
        // 29 usable rows hold three 8-row cards; layout stretches them to fill.
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
        assert_eq!(grid.index_at(11, 28), Some(8), "stretched last row");
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
