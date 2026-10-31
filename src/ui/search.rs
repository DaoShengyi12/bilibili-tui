//! Search page with video card grid display

use super::video_card::{VideoCard, VideoCardGrid};
use super::{Component, Theme, shortcut_footer};
use crate::api::search::{HotwordItem, SearchKind, SearchResultItem, SearchTarget};
use crate::application::AppAction;
use crate::storage::Keybindings;
use ratatui::{
    crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use std::time::{Duration, Instant};

/// Typing pauses this long before suggestions are requested.
const SUGGEST_DELAY: Duration = Duration::from_millis(300);

pub struct SearchPage {
    pub query: String,
    /// Characters after the input cursor; 0 keeps it at the end.
    pub cursor_back: usize,
    pub kind: SearchKind,
    /// What each card opens, by card index.
    targets: Vec<SearchTarget>,
    pub suggestions: Vec<String>,
    suggestion_selected: Option<usize>,
    last_edit: Option<Instant>,
    /// The term suggestions were last requested for.
    suggested_for: String,
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
            cursor_back: 0,
            kind: SearchKind::default(),
            targets: Vec::new(),
            suggestions: Vec::new(),
            suggestion_selected: None,
            last_edit: None,
            suggested_for: String::new(),
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

    pub fn set_results(&mut self, results: Vec<SearchResultItem>, total: i32) {
        self.grid.clear();
        self.targets.clear();
        self.add_results(results);
        self.total_results = total;
        self.loading = false;
        self.loading_more = false;
        self.input_mode = false;
        self.show_hot_list = false;
        self.error_message = None;
        self.clear_suggestions();
    }

    pub fn append_results(&mut self, results: Vec<SearchResultItem>) {
        self.add_results(results);
        self.loading_more = false;
    }

    fn add_results(&mut self, results: Vec<SearchResultItem>) {
        for item in results {
            let (bvid, aid) = match &item.target {
                SearchTarget::Video { bvid, aid } => (Some(bvid.clone()), Some(*aid)),
                _ => (None, None),
            };
            let card = VideoCard::new(
                bvid,
                aid,
                item.title,
                item.subtitle,
                item.stats,
                item.badge,
                item.cover,
            )
            .with_uploader_mid(item.uploader_mid);
            self.grid.add_card(card);
            self.targets.push(item.target);
        }
    }

    /// The action opening search result `index`.
    fn open_result(&self, index: usize) -> Option<AppAction> {
        Some(match self.targets.get(index)? {
            SearchTarget::Video { bvid, aid } => AppAction::OpenVideoDetail(bvid.clone(), *aid),
            SearchTarget::User { mid } => AppAction::OpenUpPage(*mid),
            SearchTarget::Bangumi { season_id } => AppAction::OpenBangumiDetail(*season_id),
            SearchTarget::Live { room_id } => AppAction::OpenLiveDetail(*room_id),
        })
    }

    /// Start a search for `keyword` of the current kind.
    fn begin_search(&mut self, keyword: String) -> AppAction {
        self.query = keyword.clone();
        self.cursor_back = 0;
        self.loading = true;
        self.page = 1;
        self.show_hot_list = false;
        self.clear_suggestions();
        AppAction::Search(keyword)
    }

    fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggestion_selected = None;
        self.last_edit = None;
    }

    /// Suggestions are shown instead of the hot list while typing a keyword.
    fn showing_suggestions(&self) -> bool {
        self.input_mode && !self.query.trim().is_empty() && !self.suggestions.is_empty()
    }

    /// Focus the query box, as the search key does.
    pub fn focus_input(&mut self) {
        self.input_mode = true;
        self.show_hot_list = self.hotwords_enabled;
    }

    fn after_edit(&mut self) {
        self.show_hot_list = self.hotwords_enabled;
        self.suggestion_selected = None;
        self.last_edit = Some(Instant::now());
        if self.query.trim().is_empty() {
            self.suggestions.clear();
        }
        if self.hot_selected.is_none() && !self.hotwords.is_empty() {
            self.hot_selected = Some(0);
        }
    }

    /// The term to fetch suggestions for, once typing has paused.
    pub fn suggestion_request(&mut self) -> Option<String> {
        let term = self.query.trim();
        if !self.input_mode
            || !self.hotwords_enabled
            || term.is_empty()
            || term == self.suggested_for
            || self
                .last_edit
                .is_none_or(|edited| edited.elapsed() < SUGGEST_DELAY)
        {
            return None;
        }
        self.suggested_for = term.to_string();
        Some(self.suggested_for.clone())
    }

    /// Suggestions for a term the user has since changed are dropped.
    pub fn apply_suggestions(&mut self, term: &str, suggestions: Vec<String>) {
        if self.query.trim() != term {
            return;
        }
        self.suggestions = suggestions;
        self.suggestion_selected = None;
    }

    /// Switch the result kind; returns the search to run, if any.
    pub fn switch_kind(&mut self, kind: SearchKind) -> Option<String> {
        if self.kind == kind {
            return None;
        }
        self.kind = kind;
        self.grid.clear();
        self.grid
            .set_poster_layout(matches!(kind, SearchKind::Bangumi | SearchKind::Live));
        self.targets.clear();
        self.total_results = 0;
        self.error_message = None;
        let query = self.query.trim().to_string();
        if query.is_empty() {
            return None;
        }
        self.loading = true;
        self.loading_more = false;
        self.page = 1;
        self.show_hot_list = false;
        Some(query)
    }

    fn move_suggestion(&mut self, step: isize) {
        let len = self.suggestions.len() as isize;
        self.suggestion_selected = Some(match self.suggestion_selected {
            None if step > 0 => 0,
            None => len - 1,
            Some(current) => (current as isize + step).rem_euclid(len),
        } as usize);
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
        let keyword = self
            .hot_selected
            .and_then(|idx| self.hotwords.get(idx))
            .and_then(HotwordItem::keyword_text)?;
        Some(self.begin_search(keyword))
    }

    fn draw_suggestions(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let items: Vec<ListItem> = self
            .suggestions
            .iter()
            .map(|suggestion| ListItem::new(suggestion.as_str()))
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_subtle))
                    .title(Span::styled(
                        " 搜索建议 ",
                        Style::default().fg(theme.bilibili_pink),
                    )),
            )
            .style(Style::default().fg(theme.fg_primary))
            .highlight_style(Style::default().fg(theme.bilibili_pink))
            .highlight_symbol("▶ ");
        let mut state = ListState::default().with_selected(self.suggestion_selected);
        frame.render_stateful_widget(list, area, &mut state);
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
            let loading = Paragraph::new("正在获取热搜...")
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center)
                .block(block);
            frame.render_widget(loading, area);
            return;
        }

        if let Some(err) = &self.hotword_error {
            let error_widget = Paragraph::new(err.to_string())
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

impl SearchPage {
    /// `搜索 [视频] 用户 番剧 直播` with the current kind highlighted.
    fn kind_tabs(&self, theme: &Theme) -> Line<'static> {
        let mut spans = vec![Span::styled(
            format!(" {} 搜索 ", super::icons::SEARCH),
            Style::default().fg(theme.bilibili_pink),
        )];
        for kind in SearchKind::ALL {
            spans.push(if kind == self.kind {
                Span::styled(
                    format!("[{}] ", kind.label()),
                    Style::default()
                        .fg(theme.bilibili_pink)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    format!("{} ", kind.label()),
                    Style::default().fg(theme.fg_muted),
                )
            });
        }
        Line::from(spans)
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
            .title(self.kind_tabs(theme));

        let input = if self.input_mode {
            let (before, after) = super::text::split_at_cursor(&self.query, self.cursor_back);
            Paragraph::new(format!("{before}▌{after}"))
        } else {
            Paragraph::new(self.query.as_str())
        }
        .style(input_style)
        .block(input_block);
        frame.render_widget(input, chunks[0]);

        // Results
        if self.showing_suggestions() {
            self.draw_suggestions(frame, chunks[1], theme);
        } else if self.show_hot_list {
            self.draw_hot_list(frame, chunks[1], theme);
        } else if self.loading {
            let loading = Paragraph::new("搜索中...")
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
            let error_widget = Paragraph::new(error.to_string())
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
                "没有找到相关结果"
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
                    ("Enter".to_string(), "搜索".into(), theme.success),
                    ("←/→".to_string(), "移动光标".into(), theme.fg_accent),
                    ("↑/↓".to_string(), "选择建议".into(), theme.fg_accent),
                    ("Esc".to_string(), "取消".into(), theme.info),
                    (keys.nav_next_page.clone(), "导航".into(), theme.fg_accent),
                ],
            )
        } else {
            super::hint_footer(
                theme,
                [
                    (keys.confirm.clone(), "打开".into(), theme.success),
                    (
                        format!("{}/{}", keys.section_prev, keys.section_next),
                        "类型".into(),
                        theme.fg_accent,
                    ),
                    (keys.search_focus.clone(), "搜索".into(), theme.info),
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
        if self.input_mode
            && !matches!(
                key,
                KeyCode::Up | KeyCode::Down | KeyCode::Enter | KeyCode::Esc
            )
            && let Some(changed) =
                super::text::edit_at_cursor(&mut self.query, &mut self.cursor_back, key, modifiers)
        {
            if changed {
                self.after_edit();
            }
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
                KeyCode::Up | KeyCode::Down if self.showing_suggestions() => {
                    self.move_suggestion(if key == KeyCode::Up { -1 } else { 1 });
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
                    if let Some(suggestion) = self
                        .suggestion_selected
                        .filter(|_| self.showing_suggestions())
                        .and_then(|index| self.suggestions.get(index))
                        .cloned()
                    {
                        Some(self.begin_search(suggestion))
                    } else if !self.query.trim().is_empty() {
                        Some(self.begin_search(self.query.clone()))
                    } else if self.show_hot_list {
                        self.search_selected_hotword()
                    } else {
                        Some(AppAction::None)
                    }
                }
                KeyCode::Esc => {
                    self.input_mode = false;
                    self.suggestions.clear();
                    // Back to the results that were on screen, if any.
                    if !self.grid.cards.is_empty() {
                        self.show_hot_list = false;
                    }
                    Some(AppAction::None)
                }
                _ if keys.matches_nav_next(key) => Some(AppAction::NavNext),
                _ if keys.matches_nav_prev(key) => Some(AppAction::NavPrev),
                KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete => {
                    let modifiers = crossterm::event::KeyModifiers::NONE;
                    if super::text::edit_at_cursor(
                        &mut self.query,
                        &mut self.cursor_back,
                        key,
                        modifiers,
                    ) == Some(true)
                    {
                        self.after_edit();
                    }
                    Some(AppAction::None)
                }
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
            if keys.matches_back(key) && !self.grid.cards.is_empty() {
                self.show_hot_list = false;
                return Some(AppAction::None);
            }
            if keys.matches_jump_top(key) || keys.matches_jump_bottom(key) {
                if let Some(last) = self.hotwords.len().checked_sub(1) {
                    self.hot_selected = Some(if keys.matches_jump_top(key) { 0 } else { last });
                }
                return Some(AppAction::None);
            }
            if keys.matches_search_focus(key) {
                self.focus_input();
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
                if self.grid.is_near_bottom() && !self.loading_more {
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
                if self.grid.is_near_bottom() && !self.loading_more {
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
            if keys.matches_jump_top(key) {
                self.grid.move_first();
                return Some(AppAction::None);
            }
            if keys.matches_jump_bottom(key) || keys.matches_load_more(key) {
                self.grid.move_last();
                if !self.loading_more {
                    return Some(AppAction::LoadMoreSearch);
                }
                return Some(AppAction::None);
            }
            if keys.matches_open_up(key)
                && let Some(mid) = self.grid.selected_card().and_then(|card| card.uploader_mid)
            {
                return Some(AppAction::OpenUpPage(mid));
            }
            if keys.matches_confirm(key) {
                return Some(
                    self.open_result(self.grid.selected_index)
                        .unwrap_or(AppAction::None),
                );
            }
            if keys.matches_section_prev(key) || keys.matches_section_next(key) {
                let step = if keys.matches_section_prev(key) {
                    -1
                } else {
                    1
                };
                return Some(AppAction::SwitchSearchKind(self.kind.cycle(step)));
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
                    if self.grid.is_near_bottom() && !self.loading_more {
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
                        if let Some(action) = self.open_result(click_idx) {
                            return Some(action);
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

    fn typed(page: &mut SearchPage, text: &str) {
        let keys = Keybindings::default();
        for c in text.chars() {
            page.handle_input(KeyCode::Char(c), &keys);
        }
    }

    #[test]
    fn suggestions_wait_for_a_pause_and_ignore_stale_terms() {
        let mut page = SearchPage::new();
        typed(&mut page, "rus");
        assert_eq!(page.suggestion_request(), None, "still typing");
        page.last_edit = Some(Instant::now() - SUGGEST_DELAY);
        assert_eq!(page.suggestion_request().as_deref(), Some("rus"));
        assert_eq!(page.suggestion_request(), None, "asked once per term");

        typed(&mut page, "t");
        page.apply_suggestions("rus", vec!["rush".into()]);
        assert!(page.suggestions.is_empty());
        page.apply_suggestions("rust", vec!["rust 教程".into()]);
        assert!(page.showing_suggestions());
    }

    #[test]
    fn enter_searches_the_chosen_suggestion() {
        let keys = Keybindings::default();
        let mut page = SearchPage::new();
        typed(&mut page, "rust");
        page.apply_suggestions("rust", vec!["rust 教程".into(), "rust 入门".into()]);
        page.handle_input(KeyCode::Down, &keys);
        page.handle_input(KeyCode::Down, &keys);
        let action = page.handle_input(KeyCode::Enter, &keys);
        assert!(matches!(action, Some(AppAction::Search(ref keyword)) if keyword == "rust 入门"));
        assert_eq!(page.query, "rust 入门");
        assert!(page.suggestions.is_empty());
    }

    #[test]
    fn cursor_moves_and_edits_mid_query() {
        let keys = Keybindings::default();
        let none = crossterm::event::KeyModifiers::NONE;
        let mut page = SearchPage::new();
        typed(&mut page, "ac");
        page.handle_input_with_modifiers(KeyCode::Left, none, &keys);
        page.handle_input_with_modifiers(KeyCode::Char('b'), none, &keys);
        assert_eq!(page.query, "abc");
        page.handle_input_with_modifiers(KeyCode::Home, none, &keys);
        page.handle_input_with_modifiers(KeyCode::Delete, none, &keys);
        assert_eq!(page.query, "bc");
    }

    #[test]
    fn results_of_other_kinds_open_their_pages() {
        let keys = Keybindings::default();
        let mut page = SearchPage::new();
        page.query = "q".into();
        assert_eq!(page.switch_kind(SearchKind::User).as_deref(), Some("q"));
        assert!(page.loading);
        page.set_results(
            vec![SearchResultItem {
                target: SearchTarget::User { mid: 5 },
                title: "u".into(),
                subtitle: String::new(),
                stats: String::new(),
                badge: String::new(),
                cover: None,
                uploader_mid: Some(5),
            }],
            1,
        );
        assert!(matches!(
            page.handle_input(KeyCode::Enter, &keys),
            Some(AppAction::OpenUpPage(5))
        ));
        assert!(matches!(
            page.handle_input(KeyCode::Char(']'), &keys),
            Some(AppAction::SwitchSearchKind(SearchKind::Bangumi))
        ));
        assert_eq!(page.switch_kind(SearchKind::User), None, "same kind");
    }
}
