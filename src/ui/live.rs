//! Live streaming recommendations page with grid layout

use super::{Component, Theme, shortcut_footer};
use crate::api::live::LiveRoom;
use crate::application::AppAction;
use crate::storage::Keybindings;
use image::DynamicImage;
use ratatui::{
    crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

/// Live card with cached cover image
struct LiveCard {
    pub room: LiveRoom,
    pub cover_image: Option<StatefulProtocol>,
}

/// Message for completed cover download
struct CoverResult {
    room_id: i64,
    protocol: Option<StatefulProtocol>,
}

pub struct LivePage {
    rooms: Vec<LiveCard>,
    selected_index: usize,
    columns: usize,
    scroll_offset: usize,
    loading: bool,
    error: Option<String>,
    cached_visible_rows: usize,

    // Image loading state
    picker: Arc<Picker>,
    cover_tx: mpsc::Sender<CoverResult>,
    cover_rx: mpsc::Receiver<CoverResult>,
    pending_downloads: HashSet<i64>,
    /// Where each visible card was drawn last frame, for mouse hit-testing.
    card_areas: Vec<(usize, Rect)>,
    last_load_time: Option<Instant>,
    loading_more: bool,
}

impl LivePage {
    /// 默认列数
    const DEFAULT_COLUMNS: usize = 3;
    /// 卡片高度
    const CARD_HEIGHT: u16 = 10;
    /// 预取缓冲行数（可见区域之外额外下载）
    const PREFETCH_BUFFER_ROWS: usize = 4;
    /// 初始可见行数回退值（首次渲染前使用）
    const INITIAL_VISIBLE_ROWS: usize = 3;

    pub fn new() -> Self {
        let picker = super::image_picker();
        let (tx, rx) = mpsc::channel(50);
        Self {
            rooms: Vec::new(),
            selected_index: 0,
            columns: Self::DEFAULT_COLUMNS,
            scroll_offset: 0,
            loading: false,
            error: None,
            cached_visible_rows: Self::INITIAL_VISIBLE_ROWS,
            picker,
            cover_tx: tx,
            cover_rx: rx,
            pending_downloads: HashSet::new(),
            card_areas: Vec::new(),
            last_load_time: None,
            loading_more: false,
        }
    }

    pub fn begin_loading(&mut self) {
        self.loading = true;
        self.loading_more = false;
        self.error = None;
    }

    pub fn apply_live_init(&mut self, rooms: Vec<LiveRoom>) {
        self.rooms = rooms
            .into_iter()
            .map(|room| LiveCard {
                room,
                cover_image: None,
            })
            .collect();
        self.loading = false;
        self.error = None;
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.last_load_time = Some(Instant::now());
    }

    pub fn apply_live_init_error(&mut self, msg: String) {
        self.error = Some(msg);
        self.loading = false;
    }

    pub fn begin_load_more(&mut self) -> bool {
        if self.loading || self.loading_more {
            return false;
        }
        self.loading_more = true;
        true
    }

    pub fn apply_live_more(&mut self, rooms: Vec<LiveRoom>) {
        let existing = self
            .rooms
            .iter()
            .map(|card| card.room.roomid)
            .collect::<HashSet<_>>();
        for room in rooms {
            if existing.contains(&room.roomid) {
                continue;
            }
            self.rooms.push(LiveCard {
                room,
                cover_image: None,
            });
        }
        self.loading_more = false;
    }

    pub fn apply_live_more_error(&mut self) {
        self.loading_more = false;
    }

    pub fn is_near_bottom(&self, visible_rows: usize) -> bool {
        if self.rooms.is_empty() {
            return false;
        }
        let current_row = self.selected_row();
        let total_rows = self.total_rows();
        let last_row = total_rows.saturating_sub(1);

        if total_rows <= visible_rows {
            // When all currently loaded rows fit in viewport, trigger load-more at the real bottom.
            current_row >= last_row
        } else {
            // Keep preloading behavior when content is taller than viewport.
            current_row + 2 >= last_row
        }
    }

    /// Start background downloads for visible covers. Called from the app
    /// tick, never from `draw`.
    pub fn start_cover_downloads(&mut self) {
        let prefetch_rows = self.cached_visible_rows + Self::PREFETCH_BUFFER_ROWS;
        let start_idx = self.scroll_offset * self.columns;
        let end_idx = std::cmp::min(start_idx + prefetch_rows * self.columns, self.rooms.len());

        for idx in start_idx..end_idx {
            let room = &self.rooms[idx];
            if room.cover_image.is_some() || self.pending_downloads.contains(&room.room.roomid) {
                continue;
            }

            let cover_url = if !room.room.cover.is_empty() {
                room.room.cover.clone()
            } else if !room.room.keyframe.is_empty() {
                room.room.keyframe.clone()
            } else {
                continue;
            };

            self.pending_downloads.insert(room.room.roomid);
            let room_id = room.room.roomid;
            let tx = self.cover_tx.clone();
            let picker = Arc::clone(&self.picker);

            tokio::spawn(async move {
                let protocol = Self::download_image(&cover_url)
                    .await
                    .map(|img| picker.new_resize_protocol(img));
                let _ = tx.send(CoverResult { room_id, protocol }).await;
            });
        }
    }

    /// Poll for completed cover downloads
    pub fn poll_cover_results(&mut self) {
        while let Ok(result) = self.cover_rx.try_recv() {
            // Handle cover download result
            self.pending_downloads.remove(&result.room_id);
            if let Some(protocol) = result.protocol
                && let Some(card) = self
                    .rooms
                    .iter_mut()
                    .find(|c| c.room.roomid == result.room_id)
            {
                card.cover_image = Some(protocol);
            }
        }
    }

    async fn download_image(url: &str) -> Option<DynamicImage> {
        let response = reqwest::get(url).await.ok()?;
        let bytes = response.bytes().await.ok()?;
        image::load_from_memory(&bytes).ok()
    }

    fn visible_rows(&self, height: u16) -> usize {
        (height as usize / Self::CARD_HEIGHT as usize).max(1)
    }

    fn selected_row(&self) -> usize {
        self.selected_index / self.columns
    }

    fn update_scroll(&mut self, visible_rows: usize) {
        let selected_row = self.selected_row();
        if selected_row < self.scroll_offset {
            self.scroll_offset = selected_row;
        } else if selected_row >= self.scroll_offset + visible_rows {
            self.scroll_offset = selected_row.saturating_sub(visible_rows - 1);
        }
    }

    fn move_page(&mut self, down: bool) -> bool {
        let Some(last_index) = self.rooms.len().checked_sub(1) else {
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
        if old_index == self.selected_index {
            return false;
        }
        self.update_scroll(self.cached_visible_rows.max(1));
        true
    }

    fn total_rows(&self) -> usize {
        self.rooms.len().div_ceil(self.columns)
    }
}

impl Default for LivePage {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for LivePage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        // Split area into header/content/footer like home page
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(2),
            ])
            .split(area);

        // Header
        let header = Paragraph::new("📺 关注直播优先 · B站推荐")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_subtle))
                    .title(Span::styled(
                        " 直播 ",
                        Style::default()
                            .fg(theme.fg_accent)
                            .add_modifier(Modifier::BOLD),
                    )),
            )
            .style(
                Style::default()
                    .fg(theme.bilibili_pink)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center);
        frame.render_widget(header, chunks[0]);

        // Content area
        if self.loading && self.rooms.is_empty() {
            let loading = Paragraph::new("⏳ 加载中...")
                .style(
                    Style::default()
                        .fg(theme.warning)
                        .add_modifier(Modifier::ITALIC),
                )
                .alignment(Alignment::Center);
            frame.render_widget(loading, chunks[1]);
            return;
        }

        if let Some(ref err) = self.error {
            let error = Paragraph::new(format!("❌ 加载失败: {}", err))
                .style(Style::default().fg(theme.error))
                .alignment(Alignment::Center);
            frame.render_widget(error, chunks[1]);
            return;
        }

        if self.rooms.is_empty() {
            let empty = Paragraph::new("📭 暂无直播推荐")
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center);
            frame.render_widget(empty, chunks[1]);
            return;
        }

        // Render grid
        self.render_grid(frame, chunks[1], theme);

        // Footer with hints
        let hints = Paragraph::new(shortcut_footer(
            theme,
            [
                (
                    format!("{}/↑↓←→", keys.get_nav_keys_display()),
                    "导航".into(),
                    theme.fg_accent,
                ),
                (
                    format!("{}/{}", keys.page_up, keys.page_down),
                    "翻页".into(),
                    theme.fg_accent,
                ),
                (
                    format!("{}/{}", keys.confirm, keys.play),
                    "进入".into(),
                    theme.success,
                ),
                (keys.refresh.clone(), "刷新".into(), theme.info),
                (keys.next_theme.clone(), "切换主题".into(), theme.info),
            ],
        ))
        .alignment(Alignment::Center);
        frame.render_widget(hints, chunks[2]);
    }

    fn handle_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        // Handle global keybindings first
        if keys.matches_quit(key) {
            return Some(AppAction::Quit);
        }
        if keys.matches_nav_next(key) {
            return Some(AppAction::NavNext);
        }
        if keys.matches_nav_prev(key) {
            return Some(AppAction::NavPrev);
        }
        if keys.matches_next_theme(key) {
            return Some(AppAction::NextTheme);
        }
        if keys.matches_open_settings(key) {
            return Some(AppAction::SwitchToSettings);
        }
        if keys.matches_page_down(key) {
            self.move_page(true);
            if self.is_near_bottom(self.cached_visible_rows) && !self.loading_more {
                return Some(AppAction::LoadMoreLive);
            }
            return Some(AppAction::None);
        }
        if keys.matches_page_up(key) {
            self.move_page(false);
            return Some(AppAction::None);
        }
        if keys.matches_refresh(key) {
            return Some(AppAction::RefreshLive);
        }

        if self.rooms.is_empty() {
            return Some(AppAction::None);
        }

        if keys.matches_up(key) {
            if self.selected_index >= self.columns {
                self.selected_index -= self.columns;
                self.update_scroll(self.cached_visible_rows);
            }
            return Some(AppAction::None);
        }
        if keys.matches_down(key) {
            let new_idx = self.selected_index + self.columns;
            if new_idx < self.rooms.len() {
                self.selected_index = new_idx;
                self.update_scroll(self.cached_visible_rows);
            }
            // Check for pagination
            if self.is_near_bottom(self.cached_visible_rows) && !self.loading_more {
                return Some(AppAction::LoadMoreLive);
            }
            return Some(AppAction::None);
        }
        if keys.matches_left(key) {
            if self.selected_index > 0 {
                self.selected_index -= 1;
                self.update_scroll(self.cached_visible_rows);
            }
            return Some(AppAction::None);
        }
        if keys.matches_right(key) {
            if self.selected_index + 1 < self.rooms.len() {
                self.selected_index += 1;
                self.update_scroll(self.cached_visible_rows);
            }
            return Some(AppAction::None);
        }
        if (keys.matches_confirm(key) || keys.matches_play(key))
            && let Some(card) = self.rooms.get(self.selected_index)
        {
            return Some(AppAction::OpenLiveDetail(card.room.roomid));
        }
        Some(AppAction::None)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> Option<AppAction> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Hit-test against where the cards were drawn: the header,
                // scroll offset and narrow widths are already accounted for.
                let position = Position::new(event.column, event.row);
                if let Some(&(idx, _)) = self
                    .card_areas
                    .iter()
                    .find(|(idx, card)| *idx < self.rooms.len() && card.contains(position))
                {
                    self.selected_index = idx;
                }
                None
            }
            MouseEventKind::ScrollDown => {
                let new_idx = self.selected_index + self.columns;
                if new_idx < self.rooms.len() {
                    self.selected_index = new_idx;
                    // Check for pagination
                    if self.is_near_bottom(self.cached_visible_rows) && !self.loading_more {
                        return Some(AppAction::LoadMoreLive);
                    }
                }
                None
            }
            MouseEventKind::ScrollUp => {
                if self.selected_index >= self.columns {
                    self.selected_index -= self.columns;
                }
                None
            }
            _ => None,
        }
    }
}

impl LivePage {
    fn render_grid(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let visible_rows = self.visible_rows(area.height);
        self.cached_visible_rows = visible_rows;
        self.update_scroll(visible_rows);

        let row_constraints: Vec<Constraint> = (0..visible_rows)
            .map(|_| Constraint::Min(Self::CARD_HEIGHT))
            .collect();

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints)
            .split(area);

        // Collect all card areas first; they are kept for mouse hit-testing.
        let mut card_areas = std::mem::take(&mut self.card_areas);
        card_areas.clear();

        for (row_offset, row_area) in rows.iter().enumerate() {
            let actual_row = self.scroll_offset + row_offset;
            let start_idx = actual_row * self.columns;

            if start_idx >= self.rooms.len() {
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
                let room_idx = start_idx + col_idx;
                if room_idx >= self.rooms.len() {
                    break;
                }
                card_areas.push((room_idx, *col_area));
            }
        }

        // Now render each card with mutable access
        for &(room_idx, col_area) in &card_areas {
            let is_selected = room_idx == self.selected_index;
            self.render_live_card(frame, col_area, room_idx, is_selected, theme);
        }
        self.card_areas = card_areas;
    }

    fn render_live_card(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        idx: usize,
        is_selected: bool,
        theme: &Theme,
    ) {
        let card = &mut self.rooms[idx];
        let room = &card.room;

        // Enhanced border styling (consistent with home page)
        let (border_style, border_type) = if is_selected {
            (
                Style::default()
                    .fg(theme.border_focused)
                    .add_modifier(Modifier::BOLD),
                BorderType::Rounded,
            )
        } else {
            (
                Style::default().fg(theme.border_unfocused),
                BorderType::Rounded,
            )
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(border_type)
            .border_style(border_style);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Split into cover and info
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // Cover
                Constraint::Min(2),    // Info
            ])
            .split(inner);

        // Render cover image or placeholder
        if let Some(ref mut protocol) = card.cover_image {
            let image = StatefulImage::new();
            frame.render_stateful_widget(image, chunks[0], protocol);
        } else {
            let placeholder = Paragraph::new("🎬")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_muted));
            frame.render_widget(placeholder, chunks[0]);
        }

        // Info section
        let title = room.title.chars().take(20).collect::<String>();
        let title_style = if is_selected {
            Style::default()
                .fg(theme.bilibili_pink)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_primary)
        };

        // Format online count
        let online_text = if room.online >= 10000 {
            format!("👁 {:.1}万", room.online as f64 / 10000.0)
        } else {
            format!("👁 {}", room.online)
        };

        let info_lines = vec![
            Line::from(Span::styled(title, title_style)),
            Line::from(vec![Span::styled(
                &room.uname,
                Style::default().fg(theme.fg_secondary),
            )]),
            Line::from(vec![
                Span::styled(
                    format!("{} | ", room.area_v2_name),
                    Style::default().fg(theme.fg_muted),
                ),
                Span::styled(online_text, Style::default().fg(theme.fg_accent)),
            ]),
        ];

        let info = Paragraph::new(info_lines).wrap(Wrap { trim: true });
        frame.render_widget(info, chunks[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_ends_load_more_and_blocks_new_pages_until_loaded() {
        let mut page = LivePage::new();
        page.apply_live_init(Vec::new());
        assert!(page.begin_load_more());
        page.begin_loading();
        assert!(!page.loading_more);
        assert!(!page.begin_load_more());
        page.apply_live_init(Vec::new());
        assert!(page.begin_load_more());
    }
}
