//! Bangumi page with rank list

use super::video_card::{VideoCard, VideoCardGrid};
use super::{Component, Theme};
use crate::api::bangumi::SeasonRankItem;
use crate::application::AppAction;
use crate::storage::Keybindings;
use ratatui::{
    crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use std::time::Instant;

pub struct BangumiPage {
    pub index_grid: VideoCardGrid,
    pub loading: bool,
    pub error_message: Option<String>,
    /// Parallel array storing season_id for each index card
    pub index_season_ids: Vec<i64>,
    // Double-click detection
    last_click_time: Option<Instant>,
    last_click_index: Option<usize>,
}

impl BangumiPage {
    pub fn new() -> Self {
        Self {
            index_grid: VideoCardGrid::new(),
            loading: true,
            error_message: None,
            index_season_ids: Vec::new(),
            last_click_time: None,
            last_click_index: None,
        }
    }

    pub fn set_index_items(&mut self, items: Vec<SeasonRankItem>) {
        self.index_grid.clear();
        self.index_season_ids.clear();
        for item in items {
            self.index_season_ids.push(item.season_id);
            let card = VideoCard::new(
                None,
                None,
                item.display_title(),
                item.display_subtitle(),
                item.score_text(),
                item.badge_text().unwrap_or("").to_string(),
                Some(item.cover_url()),
            );
            self.index_grid.add_card(card);
        }
        self.loading = false;
        self.error_message = None;
    }

    pub fn set_error(&mut self, msg: String) {
        self.error_message = Some(msg);
        self.loading = false;
    }

    /// Get selected index action
    fn selected_index_action(&self) -> Option<AppAction> {
        let idx = self.index_grid.selected_index;
        self.index_season_ids
            .get(idx)
            .copied()
            .map(AppAction::OpenBangumiDetail)
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        let help_line = super::hint_footer(
            theme,
            [(keys.confirm.clone(), "详情".into(), theme.success)],
        );
        let help = Paragraph::new(help_line).alignment(Alignment::Center);
        frame.render_widget(help, area);
    }
}

impl Default for BangumiPage {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for BangumiPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Title
                Constraint::Min(5),    // Content
                Constraint::Length(2), // Help
            ])
            .split(area);

        // Title
        let title = Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{} ", super::icons::BANGUMI),
                Style::default().fg(theme.bilibili_pink),
            ),
            Span::styled(
                "番剧排行",
                Style::default()
                    .fg(theme.fg_primary)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border_subtle)),
        )
        .alignment(Alignment::Center);
        frame.render_widget(title, chunks[0]);

        // Content
        let content_area = chunks[1];

        if self.loading {
            let spinner = Paragraph::new("加载中...")
                .style(Style::default().fg(theme.fg_muted))
                .alignment(Alignment::Center);
            frame.render_widget(spinner, content_area);
        } else if let Some(ref err) = self.error_message {
            let error = Paragraph::new(err.as_str())
                .style(Style::default().fg(theme.error))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(error, content_area);
        } else {
            self.index_grid.render(frame, content_area, theme);
        }

        // Footer help
        self.render_footer(frame, chunks[2], theme, keys);
    }

    fn handle_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        // Global keybindings
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
        if keys.matches_refresh(key) {
            return Some(AppAction::RefreshBangumi);
        }

        if self.loading {
            return Some(AppAction::None);
        }

        if keys.matches_page_down(key) {
            self.index_grid.move_page_down();
            return Some(AppAction::None);
        }
        if keys.matches_page_up(key) {
            self.index_grid.move_page_up();
            return Some(AppAction::None);
        }

        if keys.matches_jump_top(key) {
            self.index_grid.move_first();
            return Some(AppAction::None);
        }
        if keys.matches_jump_bottom(key) {
            self.index_grid.move_last();
            return Some(AppAction::None);
        }
        if keys.matches_down(key) {
            self.index_grid.move_down();
            return Some(AppAction::None);
        }
        if keys.matches_up(key) {
            self.index_grid.move_up();
            return Some(AppAction::None);
        }
        if keys.matches_left(key) {
            self.index_grid.move_left();
            return Some(AppAction::None);
        }
        if keys.matches_right(key) {
            self.index_grid.move_right();
            return Some(AppAction::None);
        }
        if keys.matches_play(key) || keys.matches_confirm(key) {
            return self.selected_index_action();
        }

        Some(AppAction::None)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> Option<AppAction> {
        match event.kind {
            MouseEventKind::ScrollDown => {
                self.index_grid.move_down();
                return Some(AppAction::None);
            }
            MouseEventKind::ScrollUp => {
                self.index_grid.move_up();
                return Some(AppAction::None);
            }
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return None,
        }
        // The grid records where it drew each card, which accounts for the
        // sidebar offset, the header and the scroll position.
        let index = self.index_grid.index_at(event.column, event.row)?;
        self.index_grid.selected_index = index;

        let now = Instant::now();
        let is_double = self.last_click_index == Some(index)
            && self
                .last_click_time
                .is_some_and(|time| now.duration_since(time).as_millis() <= 500);
        self.last_click_time = Some(now);
        self.last_click_index = Some(index);
        if is_double {
            return self.selected_index_action();
        }

        None
    }
}
