//! Search page with video card grid display

use super::video_card::{VideoCard, VideoCardGrid};
use super::{Component, Theme, shortcut_footer};
use crate::api::search::{HotwordItem, SearchVideoItem};
use crate::application::AppAction;
use crate::storage::Keybindings;
use ratatui::{
    crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use std::time::Instant;

pub struct SearchPage {
    pub query: String,
    pub grid: VideoCardGrid,
    pub loading: bool,
    pub error_message: Option<String>,
    pub input_mode: bool,
    pub hotwords: Vec<HotwordItem>,
    pub hotword_error: Option<String>,
    pub hotword_loading: bool,
    pub show_hot_list: bool,
    hotwords_enabled: bool,
    hot_selected: Option<usize>,
    pub page: i32,
    pub total_results: i32,
    pub loading_more: bool,
    last_click_time: Option<Instant>,
    last_click_index: Option<usize>,
    /// First hot-list row drawn last frame; clicks map through it.
    hot_list_offset: std::cell::Cell<usize>,
}

impl SearchPage {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            grid: VideoCardGrid::new_list(),
            loading: false,
            error_message: None,
            input_mode: true,
            hotwords: Vec::new(),
            hotword_error: None,
            hotword_loading: false,
            show_hot_list: true,
            hotwords_enabled: true,
            hot_selected: None,
            page: 1,
            total_results: 0,
            loading_more: false,
            last_click_time: None,
            last_click_index: None,
            hot_list_offset: std::cell::Cell::new(0),
        }
    }

    pub fn disable_hotwords(&mut self) {
        self.hotwords_enabled = false;
        self.show_hot_list = false;
        self.hotword_loading = false;
        self.hotwords.clear();
        self.hot_selected = None;
        self.hotword_error = None;
    }

    pub fn set_results(&mut self, results: Vec<SearchVideoItem>, total: i32) {
        self.grid.clear();
        for item in results {
            let card = VideoCard::new(
                item.bvid.clone(),
                item.aid,
                item.display_title(),
                item.author_name().to_string(),
                item.format_play(),
                item.duration.clone().unwrap_or_default(),
                item.cover_url(),
            )
            .with_uploader_mid(item.mid);
            self.grid.add_card(card);
        }
        self.total_results = total;
        self.loading = false;
        self.loading_more = false;
        self.input_mode = false;
        self.show_hot_list = false;
        self.error_message = None;
    }

    pub fn append_results(&mut self, results: Vec<SearchVideoItem>) {
        for item in results {
            let card = VideoCard::new(
                item.bvid.clone(),
                item.aid,
                item.display_title(),
                item.author_name().to_string(),
                item.format_play(),
                item.duration.clone().unwrap_or_default(),
                item.cover_url(),
            )
            .with_uploader_mid(item.mid);
            self.grid.add_card(card);
        }
        self.loading_more = false;
    }

    pub fn set_error(&mut self, msg: String) {
        self.error_message = Some(msg);
        self.loading = false;
        self.loading_more = false;
        self.show_hot_list = false;
    }

    pub fn start_hotword_loading(&mut self) {
        self.hotword_loading = true;
        self.hotword_error = None;
        self.hot_selected = None;
    }

    pub fn set_hotwords(&mut self, hotwords: Vec<HotwordItem>) {
        self.hotwords = hotwords;
        self.hotword_loading = false;
        self.hotword_error = None;
        self.hot_selected = if self.hotwords.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    pub fn set_hotword_error(&mut self, msg: String) {
        self.hotword_error = Some(msg);
        self.hotword_loading = false;
    }

    pub fn poll_cover_results(&mut self) {
        self.grid.poll_cover_results();
    }

    pub fn start_cover_downloads(&mut self) {
        self.grid.start_cover_downloads();
    }

    fn select_hotword(&mut self, idx: usize) {
        if idx < self.hotwords.len() {
            self.hot_selected = Some(idx);
        }
    }

    fn search_selected_hotword(&mut self) -> Option<AppAction> {
        if let Some(idx) = self.hot_selected
            && let Some(item) = self.hotwords.get(idx)
            && let Some(keyword) = item.keyword_text()
        {
            self.query = keyword.clone();
            self.loading = true;
            self.page = 1;
            self.show_hot_list = false;
            return Some(AppAction::Search(keyword));
        }
        None
    }

    fn draw_hot_list(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle))
            .title(Span::styled(
                " 热搜榜 ",
                Style::default().fg(theme.bilibili_pink),
            ));

        if self.hotword_loading {
            let loading = Paragraph::new("⏳ 正在获取热搜...")
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center)
                .block(block);
            frame.render_widget(loading, area);
            return;
        }

        if let Some(err) = &self.hotword_error {
            let error_widget = Paragraph::new(format!("❌ {}", err))
                .style(Style::default().fg(theme.error))
                .alignment(Alignment::Center)
                .block(block);
            frame.render_widget(error_widget, area);
            return;
        }

        if self.hotwords.is_empty() {
            let empty = Paragraph::new("暂无热搜数据")
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center)
                .block(block);
            frame.render_widget(empty, area);
            return;
        }

        let items: Vec<ListItem> = self
            .hotwords
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let mut spans = vec![
                    Span::styled(
                        format!("{:>2}. ", idx + 1),
                        Style::default().fg(theme.fg_muted),
                    ),
                    Span::styled(item.display_text(), Style::default().fg(theme.fg_primary)),
                ];

                if let Some(badge) = item.badge() {
                    spans.push(Span::styled(
                        format!(" [{}]", badge),
                        Style::default().fg(theme.bilibili_pink),
                    ));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().fg(theme.bilibili_pink))
            .highlight_symbol("▶ ");

        let mut state = ListState::default()
            .with_offset(self.hot_list_offset.get())
            .with_selected(self.hot_selected);
        frame.render_stateful_widget(list, area, &mut state);
        self.hot_list_offset.set(state.offset());
    }
}

impl Default for SearchPage {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for SearchPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Search input
                Constraint::Min(10),   // Results grid
                Constraint::Length(2), // Help
            ])
            .split(area);

        // Search input
        let input_style = if self.input_mode {
            Style::default().fg(theme.warning)
        } else {
            Style::default().fg(theme.fg_primary)
        };

        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(if self.input_mode {
                Style::default().fg(theme.bilibili_pink)
            } else {
                Style::default().fg(theme.border_subtle)
            })
            .title(Span::styled(
                " 🔍 搜索视频 ",
                Style::default().fg(theme.bilibili_pink),
            ));

        let cursor_char = if self.input_mode { "▌" } else { "" };
        let input = Paragraph::new(format!("{}{}", self.query, cursor_char))
            .style(input_style)
            .block(input_block);
        frame.render_widget(input, chunks[0]);

        // Results
        if self.show_hot_list {
            self.draw_hot_list(frame, chunks[1], theme);
        } else if self.loading {
            let loading = Paragraph::new("⏳ 搜索中...")
                .style(Style::default().fg(theme.warning))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.border_unfocused))
                        .title(Span::styled(
                            format!(" 结果 ({}) ", self.total_results),
                            Style::default().fg(theme.fg_secondary),
                        )),
                );
            frame.render_widget(loading, chunks[1]);
        } else if let Some(error) = &self.error_message {
            let error_widget = Paragraph::new(format!("❌ {}", error))
                .style(Style::default().fg(theme.error))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.border_unfocused)),
                );
            frame.render_widget(error_widget, chunks[1]);
        } else if self.grid.cards.is_empty() {
            let empty = Paragraph::new(if self.query.is_empty() {
                "输入关键词开始搜索"
            } else {
                "没有找到相关视频"
            })
            .style(Style::default().fg(theme.fg_secondary))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_unfocused)),
            );
            frame.render_widget(empty, chunks[1]);
        } else {
            // Render with header
            let header = Paragraph::new(Line::from(vec![
                Span::styled(" 搜索结果 ", Style::default().fg(theme.bilibili_pink)),
                Span::styled(
                    format!("({}/{})", self.grid.cards.len(), self.total_results),
                    Style::default().fg(theme.fg_muted),
                ),
                if self.loading_more {
                    Span::styled(" 加载中...", Style::default().fg(theme.warning))
                } else {
                    Span::raw("")
                },
            ]))
            .block(
                Block::default()
                    .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_subtle)),
            );

            let header_area = Rect {
                height: 2,
                ..chunks[1]
            };
            let grid_area = Rect {
                y: chunks[1].y + 2,
                height: chunks[1].height.saturating_sub(2),
                ..chunks[1]
            };

            frame.render_widget(header, header_area);
            self.grid.render(frame, grid_area, theme);
        }

        // Help
        let help = if self.input_mode {
            shortcut_footer(
                theme,
                [
                    (keys.confirm.clone(), "搜索".into(), theme.success),
                    (keys.back.clone(), "取消".into(), theme.info),
                    (keys.nav_next_page.clone(), "导航".into(), theme.fg_accent),
                ],
            )
        } else {
            shortcut_footer(
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
                    (
                        format!("{}/{}", keys.page_up, keys.page_down),
                        "翻页".into(),
                        theme.fg_accent,
                    ),
                    (keys.confirm.clone(), "详情".into(), theme.success),
                    (keys.search_focus.clone(), "搜索".into(), theme.info),
                    (keys.nav_next_page.clone(), "切换".into(), theme.info),
                ],
            )
        };
        let help = Paragraph::new(help).alignment(Alignment::Center);
        frame.render_widget(help, chunks[2]);
    }

    fn handle_input_with_modifiers(
        &mut self,
        key: KeyCode,
        modifiers: crossterm::event::KeyModifiers,
        keys: &crate::storage::Keybindings,
    ) -> Option<AppAction> {
        if self.input_mode && super::text::apply_line_edit(&mut self.query, key, modifiers) {
            self.show_hot_list = self.hotwords_enabled;
            return Some(AppAction::None);
        }
        self.handle_input(key, keys)
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        keys: &crate::storage::Keybindings,
    ) -> Option<AppAction> {
        if self.input_mode {
            match key {
                KeyCode::Char(c) => {
                    self.query.push(c);
                    self.show_hot_list = self.hotwords_enabled;
                    if self.hot_selected.is_none() && !self.hotwords.is_empty() {
                        self.hot_selected = Some(0);
                    }
                    Some(AppAction::None)
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.show_hot_list = self.hotwords_enabled;
                    if self.hot_selected.is_none() && !self.hotwords.is_empty() {
                        self.hot_selected = Some(0);
                    }
                    Some(AppAction::None)
                }
                KeyCode::Up => {
                    if self.show_hot_list && !self.hotwords.is_empty() {
                        let len = self.hotwords.len();
                        let current = self.hot_selected.unwrap_or(0);
                        let next = if current == 0 { len - 1 } else { current - 1 };
                        self.hot_selected = Some(next);
                    }
                    Some(AppAction::None)
                }
                KeyCode::Down => {
                    if self.show_hot_list && !self.hotwords.is_empty() {
                        let len = self.hotwords.len();
                        let current = self.hot_selected.unwrap_or(0);
                        let next = (current + 1) % len;
                        self.hot_selected = Some(next);
                    }
                    Some(AppAction::None)
                }
                KeyCode::Enter => {
                    if !self.query.trim().is_empty() {
                        self.loading = true;
                        self.page = 1;
                        self.show_hot_list = false;
                        Some(AppAction::Search(self.query.clone()))
                    } else if self.show_hot_list {
                        self.search_selected_hotword()
                    } else {
                        Some(AppAction::None)
                    }
                }
                KeyCode::Esc => {
                    self.input_mode = false;
                    Some(AppAction::None)
                }
                _ if keys.matches_nav_next(key) => Some(AppAction::NavNext),
                _ if keys.matches_nav_prev(key) => Some(AppAction::NavPrev),
                _ => Some(AppAction::None),
            }
        } else if self.show_hot_list {
            if keys.matches_up(key) {
                if !self.hotwords.is_empty() {
                    let len = self.hotwords.len();
                    let current = self.hot_selected.unwrap_or(0);
                    let next = if current == 0 { len - 1 } else { current - 1 };
                    self.hot_selected = Some(next);
                }
                return Some(AppAction::None);
            }
            if keys.matches_down(key) {
                if !self.hotwords.is_empty() {
                    let len = self.hotwords.len();
                    let current = self.hot_selected.unwrap_or(0);
                    let next = (current + 1) % len;
                    self.hot_selected = Some(next);
                }
                return Some(AppAction::None);
            }
            if keys.matches_confirm(key) {
                return self.search_selected_hotword();
            }
            if keys.matches_search_focus(key) {
                self.input_mode = true;
                self.show_hot_list = self.hotwords_enabled;
                return Some(AppAction::None);
            }
            if keys.matches_nav_next(key) {
                return Some(AppAction::NavNext);
            }
            if keys.matches_nav_prev(key) {
                return Some(AppAction::NavPrev);
            }
            if keys.matches_quit(key) {
                return Some(AppAction::Quit);
            }
            Some(AppAction::None)
        } else {
            if keys.matches_page_down(key) {
                self.grid.move_page_down();
                if self.grid.is_near_bottom(self.grid.cached_visible_rows) && !self.loading_more {
                    return Some(AppAction::LoadMoreSearch);
                }
                return Some(AppAction::None);
            }
            if keys.matches_page_up(key) {
                self.grid.move_page_up();
                return Some(AppAction::None);
            }
            if keys.matches_down(key) {
                self.grid.move_down();
                // Check for pagination
                if self.grid.is_near_bottom(3) && !self.loading_more {
                    return Some(AppAction::LoadMoreSearch);
                }
                return Some(AppAction::None);
            }
            if keys.matches_up(key) {
                self.grid.move_up();
                return Some(AppAction::None);
            }
            if keys.matches_right(key) {
                self.grid.move_right();
                return Some(AppAction::None);
            }
            if keys.matches_left(key) {
                self.grid.move_left();
                return Some(AppAction::None);
            }
            if key == KeyCode::Char('u')
                && let Some(mid) = self.grid.selected_card().and_then(|card| card.uploader_mid)
            {
                return Some(AppAction::OpenUpPage(mid));
            }
            if keys.matches_confirm(key) {
                if let Some(card) = self.grid.selected_card()
                    && let (Some(bvid), Some(aid)) = (&card.bvid, card.aid)
                {
                    return Some(AppAction::OpenVideoDetail(bvid.clone(), aid));
                }
                return Some(AppAction::None);
            }
            if keys.matches_search_focus(key) {
                self.input_mode = true;
                self.show_hot_list = self.hotwords_enabled;
                if self.hot_selected.is_none() && !self.hotwords.is_empty() {
                    self.hot_selected = Some(0);
                }
                return Some(AppAction::None);
            }
            if keys.matches_nav_next(key) {
                return Some(AppAction::NavNext);
            }
            if keys.matches_nav_prev(key) {
                return Some(AppAction::NavPrev);
            }
            if keys.matches_quit(key) {
                return Some(AppAction::Quit);
            }
            Some(AppAction::None)
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Option<AppAction> {
        // Don't handle mouse in input mode
        if self.input_mode {
            return None;
        }

        // Handle hot list mouse interactions
        if self.show_hot_list {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(10),
                    Constraint::Length(2),
                ])
                .split(area);

            let list_area = chunks[1];

            if !list_area.contains(ratatui::layout::Position::new(event.column, event.row)) {
                return None;
            }

            return match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    // Convert click position to list index (account for top border)
                    let row_offset = event.row.saturating_sub(list_area.y + 1);
                    let idx = self.hot_list_offset.get() + row_offset as usize;
                    if idx < self.hotwords.len() {
                        self.select_hotword(idx);
                        return self.search_selected_hotword();
                    }
                    None
                }
                MouseEventKind::ScrollDown => {
                    if !self.hotwords.is_empty() {
                        let len = self.hotwords.len();
                        let current = self.hot_selected.unwrap_or(0);
                        let next = (current + 1) % len;
                        self.hot_selected = Some(next);
                    }
                    Some(AppAction::None)
                }
                MouseEventKind::ScrollUp => {
                    if !self.hotwords.is_empty() {
                        let len = self.hotwords.len();
                        let current = self.hot_selected.unwrap_or(0);
                        let next = if current == 0 { len - 1 } else { current - 1 };
                        self.hot_selected = Some(next);
                    }
                    Some(AppAction::None)
                }
                _ => None,
            };
        }

        if self.show_hot_list {
            return None;
        }

        match event.kind {
            MouseEventKind::ScrollDown => {
                if self.grid.move_down() {
                    // Only check pagination if actually moved
                    if self.grid.is_near_bottom(3) && !self.loading_more {
                        return Some(AppAction::LoadMoreSearch);
                    }
                }
                None
            }
            MouseEventKind::ScrollUp => {
                self.grid.move_up();
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let click_idx = self.grid.index_at(event.column, event.row)?;

                if click_idx < self.grid.cards.len() {
                    let now = Instant::now();
                    let is_double_click = self.last_click_index == Some(click_idx)
                        && self
                            .last_click_time
                            .is_some_and(|t| now.duration_since(t).as_millis() < 500);

                    if is_double_click {
                        self.last_click_time = None;
                        self.last_click_index = None;
                        if let Some(card) = self.grid.cards.get(click_idx)
                            && let (Some(bvid), Some(aid)) = (&card.bvid, card.aid)
                        {
                            return Some(AppAction::OpenVideoDetail(bvid.clone(), aid));
                        }
                    } else {
                        self.grid.selected_index = click_idx;
                        self.grid.update_scroll(self.grid.cached_visible_rows);
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

#[cfg(test)]
mod minimal_mode_tests {
    use super::*;

    #[test]
    fn new_search_results_end_a_superseded_load_more() {
        let mut page = SearchPage::new();
        page.loading = true;
        page.loading_more = true;
        page.set_results(Vec::new(), 0);
        assert!(!page.loading && !page.loading_more);
    }

    #[test]
    fn typing_and_refocusing_search_keeps_hotwords_disabled() {
        let mut page = SearchPage::new();
        page.disable_hotwords();
        let keys = Keybindings::default();
        for key in [
            KeyCode::Char('x'),
            KeyCode::Backspace,
            KeyCode::Esc,
            KeyCode::Char('/'),
        ] {
            page.handle_input(key, &keys);
            assert!(!page.show_hot_list);
        }
    }
}
