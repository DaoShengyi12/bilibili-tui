//! History page with watch history display in a grid layout with cover images

use super::{Component, Theme, shortcut_footer};
use crate::api::client::ApiClient;
use crate::api::history::{HistoryCursor, HistoryItem, HistoryKey};
use crate::application::AppAction;
use crate::storage::Keybindings;
use image::DynamicImage;
use ratatui::{
    crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

/// History card with cached cover image
struct HistoryCard {
    item: HistoryItem,
    cover_protocol: Option<StatefulProtocol>,
}

/// Message for completed cover download
struct CoverResult {
    index: usize,
    generation: u64,
    protocol: StatefulProtocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoryMode {
    Browse,
    Selecting,
    ConfirmDelete,
    Deleting,
}

pub struct HistoryPage {
    items: Vec<HistoryCard>,
    selected: usize,
    scroll_offset: usize,
    loading: bool,
    error: Option<String>,
    picker: Arc<Picker>,
    cursor: Option<HistoryCursor>,
    has_more: bool,

    pending_downloads: HashSet<usize>,
    cover_rx: mpsc::Receiver<CoverResult>,
    cover_tx: mpsc::Sender<CoverResult>,
    cached_visible_rows: usize,
    selected_keys: HashSet<HistoryKey>,
    /// Set when `d` selected the hovered item on its own, so cancelling the
    /// confirmation returns to browsing instead of leaving it selected.
    implicit_delete_selection: bool,
    mode: HistoryMode,
    notice: Option<String>,
    generation: u64,

    last_click_time: Option<Instant>,
    last_click_index: Option<usize>,
}

impl HistoryPage {
    const COLUMNS: usize = 4;
    const CARD_HEIGHT: u16 = 12;
    const PREFETCH_BUFFER_ROWS: usize = 2;
    const INITIAL_VISIBLE_ROWS: usize = 3;

    pub fn new() -> Self {
        let picker = super::image_picker();
        let (tx, rx) = mpsc::channel(32);

        Self {
            items: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            loading: false,
            error: None,
            picker,
            cursor: None,
            has_more: true,
            pending_downloads: HashSet::new(),
            cover_rx: rx,
            cover_tx: tx,
            cached_visible_rows: Self::INITIAL_VISIBLE_ROWS,
            selected_keys: HashSet::new(),
            implicit_delete_selection: false,
            mode: HistoryMode::Browse,
            notice: None,
            generation: 0,
            last_click_time: None,
            last_click_index: None,
        }
    }

    pub async fn load_history(&mut self, api_client: &ApiClient) {
        self.loading = true;
        self.error = None;

        match api_client.get_history(None, None, None).await {
            Ok(data) => {
                self.items = data
                    .list
                    .into_iter()
                    .map(|item| HistoryCard {
                        item,
                        cover_protocol: None,
                    })
                    .collect();
                self.cursor = Some(data.cursor);
                self.has_more = !self.items.is_empty();
                self.loading = false;
                self.reset_selection_and_downloads();
            }
            Err(e) => {
                self.error = Some(format!("加载历史记录失败: {}", e));
                self.loading = false;
            }
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn begin_loading(&mut self) {
        self.loading = true;
        self.error = None;
    }

    pub fn apply_history_init(&mut self, data: crate::api::history::HistoryData) {
        self.items = data
            .list
            .into_iter()
            .map(|item| HistoryCard {
                item,
                cover_protocol: None,
            })
            .collect();
        self.cursor = Some(data.cursor);
        self.has_more = !self.items.is_empty();
        self.selected = 0;
        self.scroll_offset = 0;
        self.loading = false;
        self.error = None;
        self.reset_selection_and_downloads();
    }

    pub fn start_load_more_request(&mut self) -> Option<crate::api::history::HistoryCursor> {
        if self.loading || !self.has_more {
            return None;
        }
        let cursor = self.cursor.clone()?;
        self.loading = true;
        Some(cursor)
    }

    pub fn apply_history_more(&mut self, data: crate::api::history::HistoryData) {
        let new_items: Vec<HistoryCard> = data
            .list
            .into_iter()
            .map(|item| HistoryCard {
                item,
                cover_protocol: None,
            })
            .collect();

        if new_items.is_empty() {
            self.has_more = false;
        } else {
            self.cursor = Some(data.cursor);
            self.items.extend(new_items);
        }
        self.loading = false;
        self.error = None;
    }

    /// A failure with nothing loaded replaces the grid; a failure while
    /// loading more keeps the loaded items visible and reports a notice, so
    /// keys never act on a hidden grid.
    pub fn apply_load_more_error(&mut self, msg: String) {
        if self.items.is_empty() {
            self.error = Some(msg);
        } else {
            self.notice = Some(msg);
        }
        self.loading = false;
    }

    /// Leave the in-flight deletion state when the request cannot be started,
    /// such as when credentials expire between confirmation and dispatch.
    pub fn cancel_deletion(&mut self) {
        self.sync_selection_mode();
    }

    pub async fn load_more(&mut self, api_client: &ApiClient) {
        if self.loading || !self.has_more {
            return;
        }

        let Some(cursor) = &self.cursor else {
            return;
        };

        self.loading = true;

        match api_client
            .get_history(
                Some(cursor.max),
                Some(cursor.view_at),
                Some(&cursor.business),
            )
            .await
        {
            Ok(data) => {
                let new_items: Vec<HistoryCard> = data
                    .list
                    .into_iter()
                    .map(|item| HistoryCard {
                        item,
                        cover_protocol: None,
                    })
                    .collect();

                if new_items.is_empty() {
                    self.has_more = false;
                } else {
                    self.cursor = Some(data.cursor);
                    self.items.extend(new_items);
                }
                self.loading = false;
            }
            Err(e) => {
                self.error = Some(format!("加载更多失败: {}", e));
                self.loading = false;
            }
        }
    }

    fn is_near_bottom(&self, visible_rows: usize) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let total_rows = self.items.len().div_ceil(Self::COLUMNS);
        let current_row = self.selected / Self::COLUMNS;
        current_row + 2 >= self.scroll_offset + visible_rows.min(total_rows)
    }

    /// Start background downloads for visible covers (non-blocking)
    pub fn start_cover_downloads(&mut self) {
        if self.items.is_empty() {
            return;
        }

        // Calculate visible range
        let visible_start = self.scroll_offset * Self::COLUMNS;
        let prefetch_rows = self.cached_visible_rows + Self::PREFETCH_BUFFER_ROWS;
        let visible_end = (visible_start + prefetch_rows * Self::COLUMNS).min(self.items.len());

        for idx in visible_start..visible_end {
            if self.items[idx].cover_protocol.is_some() || self.pending_downloads.contains(&idx) {
                continue;
            }

            let Some(cover_url) = self.items[idx].item.get_cover() else {
                continue;
            };

            self.pending_downloads.insert(idx);
            let url = cover_url.to_string();
            let tx = self.cover_tx.clone();
            let picker = Arc::clone(&self.picker);
            let generation = self.generation;

            tokio::spawn(async move {
                if let Some(img) = Self::download_image(&url).await {
                    let protocol = picker.new_resize_protocol(img);
                    let _ = tx
                        .send(CoverResult {
                            index: idx,
                            generation,
                            protocol,
                        })
                        .await;
                }
            });
        }
    }

    /// Poll for completed cover downloads (non-blocking)
    pub fn poll_cover_results(&mut self) {
        while let Ok(result) = self.cover_rx.try_recv() {
            if result.generation != self.generation {
                continue;
            }
            self.pending_downloads.remove(&result.index);
            if result.index < self.items.len() {
                self.items[result.index].cover_protocol = Some(result.protocol);
            }
        }
    }

    async fn download_image(url: &str) -> Option<DynamicImage> {
        let response = reqwest::get(url).await.ok()?;
        let bytes = response.bytes().await.ok()?;
        image::load_from_memory(&bytes).ok()
    }

    fn visible_rows(&self, height: u16) -> usize {
        (height / Self::CARD_HEIGHT).max(1) as usize
    }

    fn selected_row(&self) -> usize {
        self.selected / Self::COLUMNS
    }

    fn update_scroll(&mut self, visible_rows: usize) {
        let row = self.selected_row();
        if row < self.scroll_offset {
            self.scroll_offset = row;
        } else if row >= self.scroll_offset + visible_rows {
            self.scroll_offset = row - visible_rows + 1;
        }
    }

    fn move_page(&mut self, down: bool) -> bool {
        let Some(last_index) = self.items.len().checked_sub(1) else {
            return false;
        };
        let page_size = self
            .cached_visible_rows
            .max(1)
            .saturating_mul(Self::COLUMNS);
        let old_index = self.selected;
        self.selected = if down {
            old_index.saturating_add(page_size).min(last_index)
        } else {
            old_index.saturating_sub(page_size)
        };
        if old_index == self.selected {
            return false;
        }
        self.update_scroll(self.cached_visible_rows.max(1));
        true
    }

    fn action_for_history_item(item: &HistoryItem) -> Option<AppAction> {
        if item.history.business == "archive" {
            if let Some(bvid) = item.get_bvid() {
                let aid = item.history.oid;
                return Some(AppAction::OpenVideoDetail(bvid.to_string(), aid));
            }
            return None;
        }

        if item.history.business == "pgc" {
            return (item.kid > 0 && item.history.epid > 0).then_some(
                AppAction::OpenHistoryBangumi {
                    season_id: item.kid,
                    ep_id: item.history.epid,
                },
            );
        }

        if let Some(cvid) = item.article_id() {
            return Some(AppAction::OpenArticle(cvid));
        }

        if item.is_live() && item.live_status == 1 {
            return item.get_live_room_id().map(AppAction::OpenLiveDetail);
        }

        None
    }

    fn reset_selection_and_downloads(&mut self) {
        self.selected_keys.clear();
        self.mode = HistoryMode::Browse;
        self.notice = None;
        self.pending_downloads.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    fn sync_selection_mode(&mut self) {
        self.mode = if self.selected_keys.is_empty() {
            HistoryMode::Browse
        } else {
            HistoryMode::Selecting
        };
    }

    fn toggle_selected(&mut self) {
        let Some(key) = self
            .items
            .get(self.selected)
            .and_then(|card| card.item.history_key())
        else {
            self.notice = Some("直播记录不支持批量选择".to_string());
            return;
        };
        let newly_selected = if self.selected_keys.remove(&key) {
            false
        } else {
            self.selected_keys.insert(key);
            true
        };
        self.sync_selection_mode();
        self.notice = None;
        if newly_selected && self.selected + 1 < self.items.len() {
            self.selected += 1;
            self.update_scroll(self.cached_visible_rows.max(1));
        }
    }

    fn select_all_loaded(&mut self) {
        self.selected_keys = self
            .items
            .iter()
            .filter_map(|card| card.item.history_key())
            .collect();
        self.sync_selection_mode();
        self.notice = None;
    }

    fn invert_loaded_selection(&mut self) {
        let eligible: HashSet<_> = self
            .items
            .iter()
            .filter_map(|card| card.item.history_key())
            .collect();
        self.selected_keys = eligible.difference(&self.selected_keys).cloned().collect();
        self.sync_selection_mode();
        self.notice = None;
    }

    pub fn apply_delete_result(
        &mut self,
        successful: Vec<HistoryKey>,
        failed: Vec<(HistoryKey, String)>,
    ) {
        let successful: HashSet<_> = successful.into_iter().collect();
        let success_count = successful.len();
        self.items.retain(|card| {
            card.item
                .history_key()
                .is_none_or(|key| !successful.contains(&key))
        });
        self.pending_downloads.clear();
        self.generation = self.generation.wrapping_add(1);
        for card in &mut self.items {
            card.cover_protocol = None;
        }

        self.selected_keys = failed.iter().map(|(key, _)| key.clone()).collect();
        let failed_count = failed.len();
        self.sync_selection_mode();
        self.selected = self.selected.min(self.items.len().saturating_sub(1));
        let total_rows = self.items.len().div_ceil(Self::COLUMNS);
        self.scroll_offset = self.scroll_offset.min(total_rows.saturating_sub(1));
        self.notice = Some(if failed_count == 0 {
            format!("已删除 {success_count} 条历史记录")
        } else {
            format!("已删除 {success_count} 条，{failed_count} 条失败并保留选择")
        });
    }
}

impl Default for HistoryPage {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for HistoryPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        // Main block
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle))
            .title(Span::styled(
                " 📜 观看历史 ",
                Style::default()
                    .fg(theme.bilibili_pink)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Loading state
        if self.loading && self.items.is_empty() {
            let loading = Paragraph::new("加载中...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_muted));
            frame.render_widget(loading, inner);
            return;
        }

        // Error state
        if let Some(ref err) = self.error {
            let error = Paragraph::new(err.as_str())
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.error));
            frame.render_widget(error, inner);
            return;
        }

        // Empty state
        if self.items.is_empty() {
            let empty = Paragraph::new("暂无历史记录")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_muted));
            frame.render_widget(empty, inner);
            return;
        }

        let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
        self.render_grid(frame, chunks[0], theme);
        self.render_footer(frame, chunks[1], theme, keys);
        if self.mode == HistoryMode::ConfirmDelete {
            self.render_delete_confirmation(frame, area, theme);
        }
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        keys: &crate::storage::Keybindings,
    ) -> Option<AppAction> {
        self.handle_input_with_modifiers(key, KeyModifiers::NONE, keys)
    }

    fn handle_input_with_modifiers(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        keys: &crate::storage::Keybindings,
    ) -> Option<AppAction> {
        if self.mode == HistoryMode::Deleting {
            return None;
        }
        if self.mode == HistoryMode::ConfirmDelete {
            if key == KeyCode::Esc || keys.matches_back(key) {
                if self.implicit_delete_selection {
                    self.selected_keys.clear();
                }
                self.implicit_delete_selection = false;
                self.sync_selection_mode();
                return None;
            }
            if keys.matches_confirm(key) {
                self.implicit_delete_selection = false;
                self.mode = HistoryMode::Deleting;
                return Some(AppAction::DeleteHistoryItems(
                    self.selected_keys.iter().cloned().collect(),
                ));
            }
            return None;
        }

        if modifiers.contains(KeyModifiers::CONTROL) && key == KeyCode::Char('a') {
            self.select_all_loaded();
            return None;
        }
        if modifiers.contains(KeyModifiers::CONTROL) && key == KeyCode::Char('r') {
            self.invert_loaded_selection();
            return None;
        }
        if key == KeyCode::Char(' ') {
            self.toggle_selected();
            return None;
        }
        if key == KeyCode::Char('d') {
            self.implicit_delete_selection = self.selected_keys.is_empty();
            if self.selected_keys.is_empty() {
                let Some(key) = self
                    .items
                    .get(self.selected)
                    .and_then(|card| card.item.history_key())
                else {
                    self.notice = Some("该记录不支持删除".to_string());
                    return None;
                };
                self.selected_keys.insert(key);
            }
            self.notice = None;
            self.mode = HistoryMode::ConfirmDelete;
            return None;
        }
        if key == KeyCode::Esc || keys.matches_back(key) {
            if !self.selected_keys.is_empty() {
                self.selected_keys.clear();
                self.sync_selection_mode();
                self.notice = None;
                return None;
            }
            return Some(AppAction::BackToList);
        }

        let cols = Self::COLUMNS;
        let total = self.items.len();

        if keys.matches_quit(key) {
            return Some(AppAction::Quit);
        }
        if keys.matches_left(key) {
            if self.selected > 0 {
                self.selected -= 1;
            }
            return None;
        }
        if keys.matches_right(key) {
            if self.selected + 1 < total {
                self.selected += 1;
            }
            return None;
        }
        if keys.matches_page_down(key) {
            self.move_page(true);
            if self.is_near_bottom(self.cached_visible_rows) {
                return Some(AppAction::LoadMoreHistory);
            }
            return None;
        }
        if keys.matches_page_up(key) {
            self.move_page(false);
            return None;
        }
        if keys.matches_up(key) {
            if self.selected >= cols {
                self.selected -= cols;
            }
            return None;
        }
        if keys.matches_down(key) {
            if self.selected + cols < total {
                self.selected += cols;
            }
            // Check if we need to load more
            if self.is_near_bottom(self.cached_visible_rows) {
                return Some(AppAction::LoadMoreHistory);
            }
            return None;
        }
        if key == KeyCode::Char('u')
            && let Some(item) = self.items.get(self.selected)
        {
            return Some(AppAction::OpenUpPage(item.item.author_mid));
        }
        if keys.matches_confirm(key) {
            if !self.selected_keys.is_empty() {
                return None;
            }
            if let Some(card) = self.items.get(self.selected) {
                return Self::action_for_history_item(&card.item);
            }
            return None;
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
        None
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Option<AppAction> {
        if matches!(
            self.mode,
            HistoryMode::ConfirmDelete | HistoryMode::Deleting
        ) {
            return None;
        }
        let cols = Self::COLUMNS;
        let total = self.items.len();

        match event.kind {
            MouseEventKind::ScrollDown => {
                if self.selected + cols < total {
                    self.selected += cols;
                    if self.is_near_bottom(self.cached_visible_rows) {
                        return Some(AppAction::LoadMoreHistory);
                    }
                }
                None
            }
            MouseEventKind::ScrollUp => {
                if self.selected >= cols {
                    self.selected -= cols;
                }
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let inner = area.inner(Margin::new(1, 1));

                if !inner.contains(ratatui::layout::Position::new(event.column, event.row)) {
                    return None;
                }

                let card_height = Self::CARD_HEIGHT;
                // Panes narrower than the column count would otherwise divide by zero.
                let card_width = (inner.width / cols as u16).max(1);

                let relative_y = event.row - inner.y;
                let click_row = (relative_y / card_height) as usize;
                let actual_row = self.scroll_offset + click_row;

                let click_col = (event.column.saturating_sub(inner.x) / card_width) as usize;
                if click_col >= cols {
                    return None;
                }

                let click_idx = actual_row * cols + click_col;

                if click_idx < self.items.len() {
                    let now = Instant::now();
                    // While items are selected for deletion, clicks only move
                    // the cursor; opening an item would abandon the selection.
                    let is_double_click = self.mode == HistoryMode::Browse
                        && self.last_click_index == Some(click_idx)
                        && self
                            .last_click_time
                            .is_some_and(|t| now.duration_since(t).as_millis() < 500);

                    if is_double_click {
                        self.last_click_time = None;
                        self.last_click_index = None;
                        if let Some(card) = self.items.get(click_idx) {
                            return Self::action_for_history_item(&card.item);
                        }
                    } else {
                        self.selected = click_idx;
                        let visible_rows = self.visible_rows(area.height);
                        self.update_scroll(visible_rows);
                        self.last_click_time = Some(now);
                        self.last_click_index = Some(click_idx);
                    }
                }
                None
            }
            _ => None,
        }
    }
}

impl HistoryPage {
    fn render_grid(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let cols = Self::COLUMNS;
        let visible_rows = self.visible_rows(area.height);
        self.cached_visible_rows = visible_rows;
        self.update_scroll(visible_rows);

        let card_height = Self::CARD_HEIGHT;
        let card_width = area.width / cols as u16;

        let start_idx = self.scroll_offset * cols;
        let end_idx = (start_idx + visible_rows * cols).min(self.items.len());

        for (i, idx) in (start_idx..end_idx).enumerate() {
            let row = i / cols;
            let col = i % cols;

            let x = area.x + (col as u16 * card_width);
            let y = area.y + (row as u16 * card_height);

            if y + card_height > area.y + area.height {
                break;
            }

            let card_area = Rect::new(x, y, card_width, card_height);
            let is_selected = idx == self.selected;
            let is_marked = self.items[idx]
                .item
                .history_key()
                .is_some_and(|key| self.selected_keys.contains(&key));

            self.render_history_card(frame, card_area, idx, is_selected, is_marked, theme);
        }

        // Loading indicator at bottom
        if self.loading && !self.items.is_empty() {
            let loading_area = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
            let loading = Paragraph::new("加载更多...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_muted));
            frame.render_widget(loading, loading_area);
        }
    }

    fn render_history_card(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        idx: usize,
        is_selected: bool,
        is_marked: bool,
        theme: &Theme,
    ) {
        let card = &mut self.items[idx];

        // Card border
        let border_color = if is_marked {
            theme.warning
        } else if is_selected {
            theme.bilibili_pink
        } else {
            theme.border_subtle
        };

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(if is_selected {
                BorderType::Thick
            } else {
                BorderType::Rounded
            })
            .border_style(Style::default().fg(border_color));
        if is_marked {
            block = block.title(Span::styled(
                " ✓ 已选择 ",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width < 4 || inner.height < 4 {
            return;
        }

        // Split into cover area and info area
        let cover_height = 6u16.min(inner.height.saturating_sub(3));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(cover_height), Constraint::Min(3)])
            .split(inner);

        // Render cover
        if let Some(ref mut protocol) = card.cover_protocol {
            let image = StatefulImage::default();
            frame.render_stateful_widget(image, chunks[0], protocol);
        } else {
            // Placeholder with badge
            let badge = card.item.badge.as_deref().unwrap_or("");
            let placeholder = Paragraph::new(badge)
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_muted).bg(theme.bg_secondary));
            frame.render_widget(placeholder, chunks[0]);
        }

        // Info area
        let info_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Title
                Constraint::Length(1), // Author + time
                Constraint::Min(0),    // Progress/duration
            ])
            .split(chunks[1]);

        // Title (2 lines)
        let title = &card.item.title;
        let title_style = if is_selected {
            Style::default()
                .fg(theme.fg_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_primary)
        };
        let title_widget = Paragraph::new(title.as_str())
            .style(title_style)
            .wrap(Wrap { trim: true });
        frame.render_widget(title_widget, info_chunks[0]);

        // Author + view time
        let author = &card.item.author_name;
        let view_time = card.item.format_view_time();
        let info_text = format!("{} · {}", author, view_time);
        let info_widget = Paragraph::new(info_text)
            .style(Style::default().fg(theme.fg_muted))
            .wrap(Wrap { trim: true });
        frame.render_widget(info_widget, info_chunks[1]);

        // Progress / Duration
        if card.item.duration > 0 {
            let progress_text = format!(
                "{} / {}",
                card.item.format_progress(),
                card.item.format_duration()
            );
            let progress_widget =
                Paragraph::new(progress_text).style(Style::default().fg(theme.fg_secondary));
            frame.render_widget(progress_widget, info_chunks[2]);
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        if self.mode == HistoryMode::Deleting || self.notice.is_some() {
            let text = self.notice.as_deref().unwrap_or("正在删除所选历史记录...");
            frame.render_widget(
                Paragraph::new(text)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(theme.fg_secondary)),
                area,
            );
            return;
        }

        let help = shortcut_footer(
            theme,
            [
                (
                    format!(
                        "{}/{}",
                        keys.get_arrow_keys_display(),
                        keys.get_nav_keys_display()
                    ),
                    "导航".into(),
                    theme.fg_accent,
                ),
                ("Space".into(), "选择".into(), theme.fg_accent),
                ("Ctrl+A/Ctrl+R".into(), "全选/反选".into(), theme.fg_accent),
                ("d".into(), "删除".into(), theme.info),
                (keys.confirm.clone(), "详情".into(), theme.success),
                (
                    "Esc".into(),
                    format!("取消 · 已选 {}", self.selected_keys.len()),
                    theme.fg_accent,
                ),
            ],
        );
        frame.render_widget(Paragraph::new(help).alignment(Alignment::Center), area);
    }

    fn render_delete_confirmation(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let width = area.width.min(52);
        let height = area.height.min(7);
        let popup = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, popup);
        let message = format!(
            "确定删除 {} 条历史记录？\n\nEnter 确认 · Esc 返回选择",
            self.selected_keys.len()
        );
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_primary))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.warning))
                        .title(" 删除历史记录 "),
                ),
            popup,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_page() -> HistoryPage {
        let data = serde_json::from_value(serde_json::json!({
            "cursor": {"max": 0, "view_at": 0, "business": "", "ps": 20},
            "tab": null,
            "list": [
                {"title": "video", "kid": 1, "history": {"oid": 1, "bvid": "BV1", "business": "archive"}},
                {"title": "article", "kid": 2, "history": {"oid": 20, "business": "article"}},
                {"title": "live", "kid": 3, "history": {"oid": 30, "business": "live"}}
            ]
        }))
        .expect("history fixture");
        let mut page = HistoryPage::new();
        page.apply_history_init(data);
        page
    }

    #[test]
    fn selection_shortcuts_only_include_loaded_eligible_records() {
        let keys = Keybindings::default();
        let mut page = history_page();

        page.handle_input_with_modifiers(KeyCode::Char(' '), KeyModifiers::NONE, &keys);
        assert_eq!(page.selected, 1);
        assert_eq!(page.selected_keys.len(), 1);
        page.handle_input_with_modifiers(KeyCode::Esc, KeyModifiers::NONE, &keys);

        page.handle_input_with_modifiers(KeyCode::Char('a'), KeyModifiers::CONTROL, &keys);
        assert_eq!(page.selected_keys.len(), 2);
        page.handle_input_with_modifiers(KeyCode::Char('r'), KeyModifiers::CONTROL, &keys);
        assert!(page.selected_keys.is_empty());

        page.selected = 2;
        page.handle_input_with_modifiers(KeyCode::Char(' '), KeyModifiers::NONE, &keys);
        assert!(page.selected_keys.is_empty());
    }

    #[test]
    fn delete_without_selection_targets_the_hovered_record() {
        let keys = Keybindings::default();
        let mut page = history_page();
        page.selected = 1;

        page.handle_input_with_modifiers(KeyCode::Char('d'), KeyModifiers::NONE, &keys);

        assert_eq!(page.mode, HistoryMode::ConfirmDelete);
        assert_eq!(
            page.selected_keys,
            HashSet::from([HistoryKey {
                business: "article".into(),
                kid: 2,
            }])
        );
    }

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn clicks_in_a_pane_narrower_than_the_columns_do_not_panic() {
        let mut page = history_page();
        for width in 0..8 {
            page.handle_mouse(left_click(1, 1), Rect::new(0, 0, width, 20));
        }
    }

    #[test]
    fn double_click_during_selection_does_not_open_the_item() {
        let keys = Keybindings::default();
        let mut page = history_page();
        page.handle_input_with_modifiers(KeyCode::Char(' '), KeyModifiers::NONE, &keys);
        assert_eq!(page.mode, HistoryMode::Selecting);
        let area = Rect::new(0, 0, 120, 40);
        assert!(page.handle_mouse(left_click(2, 2), area).is_none());
        assert!(page.handle_mouse(left_click(2, 2), area).is_none());
        assert_eq!(page.mode, HistoryMode::Selecting);
    }

    #[test]
    fn delete_confirmation_escape_and_partial_result_preserve_failed_selection() {
        let keys = Keybindings::default();
        let mut page = history_page();
        page.handle_input_with_modifiers(KeyCode::Char('a'), KeyModifiers::CONTROL, &keys);
        page.handle_input_with_modifiers(KeyCode::Char('d'), KeyModifiers::NONE, &keys);
        assert_eq!(page.mode, HistoryMode::ConfirmDelete);

        page.handle_input_with_modifiers(KeyCode::Esc, KeyModifiers::NONE, &keys);
        assert_eq!(page.mode, HistoryMode::Selecting);
        assert_eq!(page.selected_keys.len(), 2);

        page.handle_input_with_modifiers(KeyCode::Char('d'), KeyModifiers::NONE, &keys);
        let action = page.handle_input_with_modifiers(KeyCode::Enter, KeyModifiers::NONE, &keys);
        assert!(matches!(action, Some(AppAction::DeleteHistoryItems(_))));
        assert_eq!(page.mode, HistoryMode::Deleting);

        let video = HistoryKey {
            business: "archive".into(),
            kid: 1,
        };
        let article = HistoryKey {
            business: "article".into(),
            kid: 2,
        };
        page.apply_delete_result(vec![video], vec![(article.clone(), "denied".into())]);
        assert_eq!(page.items.len(), 2);
        assert!(page.selected_keys.contains(&article));
        assert_eq!(page.mode, HistoryMode::Selecting);

        page.handle_input_with_modifiers(KeyCode::Esc, KeyModifiers::NONE, &keys);
        assert!(page.selected_keys.is_empty());
        assert_eq!(page.mode, HistoryMode::Browse);
    }

    #[test]
    fn load_more_failure_keeps_loaded_items_visible() {
        let mut page = history_page();
        page.apply_load_more_error("加载更多失败: timeout".to_string());
        assert!(page.error.is_none());
        assert_eq!(page.notice.as_deref(), Some("加载更多失败: timeout"));
        assert_eq!(page.items.len(), 3);
        assert!(!page.loading);
    }

    #[test]
    fn initial_load_failure_shows_page_error() {
        let mut page = HistoryPage::new();
        page.apply_load_more_error("加载历史记录失败: timeout".to_string());
        assert_eq!(page.error.as_deref(), Some("加载历史记录失败: timeout"));
    }

    #[test]
    fn escape_after_single_delete_prompt_returns_to_browse() {
        let keys = Keybindings::default();
        let mut page = history_page();
        page.handle_input_with_modifiers(KeyCode::Char('d'), KeyModifiers::NONE, &keys);
        assert_eq!(page.mode, HistoryMode::ConfirmDelete);
        assert_eq!(page.selected_keys.len(), 1);

        page.handle_input_with_modifiers(KeyCode::Esc, KeyModifiers::NONE, &keys);
        assert_eq!(page.mode, HistoryMode::Browse);
        assert!(page.selected_keys.is_empty());
    }

    #[test]
    fn cancel_deletion_restores_selection_mode() {
        let mut page = history_page();
        page.selected_keys.insert(HistoryKey {
            business: "archive".into(),
            kid: 1,
        });
        page.mode = HistoryMode::Deleting;

        page.cancel_deletion();

        assert_eq!(page.mode, HistoryMode::Selecting);
        assert_eq!(page.selected_keys.len(), 1);
    }
}
