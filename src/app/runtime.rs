use crate::app::App;
use crate::presentation::tui::{Component, Page};
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEvent},
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use std::io;

impl App {
    /// Main run loop
    pub async fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        // Initialize the first page
        self.init_current_page().await;
        self.check_session_freshness();

        // Store the last content area for mouse handling
        let mut last_content_area = Rect::default();
        let mut last_frame_area = Rect::default();

        while !self.should_quit {
            terminal.draw(|frame| {
                last_content_area = self.get_content_area(frame.area());
                last_frame_area = frame.area();
                self.draw(frame);
            })?;

            if event::poll(std::time::Duration::from_millis(100))? {
                match event::read()? {
                    // Repeats come from holding a key down; they move
                    // through lists like presses.
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        self.handle_input(key.code, key.modifiers).await;
                    }
                    Event::Paste(text) => self.handle_paste(&text).await,
                    // Terminals report one wheel event per notch, so each
                    // event scrolls one step.
                    Event::Mouse(mouse) => {
                        let on_sidebar = self.handle_sidebar_click(mouse, last_frame_area).await;
                        if !on_sidebar {
                            self.handle_mouse(mouse, last_content_area).await;
                        }
                    }
                    _ => {}
                }
            }

            // Handle background tasks (like QR code polling)
            self.tick().await;
        }
        Ok(())
    }

    /// Ask the server once per start whether the login cookies are due for a
    /// refresh; Bilibili expects web clients to renew them before expiry.
    pub(super) fn check_session_freshness(&mut self) {
        if let Some(credentials) = self
            .credentials
            .clone()
            .filter(|credentials| credentials.refresh_token.is_some())
        {
            self.send_network_command(
                crate::application::network::NetworkCommand::RefreshSession {
                    credentials,
                    force: false,
                },
            );
        }
    }

    /// Whether a text field has focus, so it receives pastes and every
    /// Ctrl/Alt chord.
    fn current_page_text_field_active(&self) -> bool {
        match &self.current_page {
            Page::Home(page) => page.search_input_active(),
            Page::Search(page) => page.input_mode,
            Page::VideoDetail(page) => page.input_mode,
            Page::DynamicDetail(page) => page.input_mode,
            Page::ArticleDetail(page) => page.input_mode,
            Page::LiveDetail(page) => page.input_mode,
            _ => false,
        }
    }

    /// Whether the page sends every key to a text field or key capture.
    fn current_page_captures_keys(&self) -> bool {
        match &self.current_page {
            Page::Home(page) => page.search_input_active(),
            Page::Search(page) => page.input_mode,
            Page::VideoDetail(page) => page.input_mode,
            Page::DynamicDetail(page) => page.input_mode,
            Page::ArticleDetail(page) => page.input_mode,
            Page::Settings(page) => page.is_capturing_input(),
            Page::LiveDetail(page) => page.captures_keys(),
            _ => false,
        }
    }

    fn current_page_uses_full_area(&self) -> bool {
        match &self.current_page {
            Page::Login(_)
            | Page::VideoDetail(_)
            | Page::DynamicDetail(_)
            | Page::ArticleDetail(_)
            | Page::BangumiDetail(_) => true,
            Page::Up(page) => !page.is_sidebar_page,
            _ => false,
        }
    }

    /// Get the content area excluding sidebar
    fn get_content_area(&self, area: Rect) -> Rect {
        if self.current_page_uses_full_area() {
            return area;
        }

        // Main layout with sidebar
        if self.show_sidebar {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(16), // Sidebar
                    Constraint::Min(40),    // Content
                ])
                .split(area)[1]
        } else {
            area
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();

        if self.current_page_uses_full_area() {
            match &mut self.current_page {
                Page::Login(page) => page.draw(frame, area, &self.theme, &self.keybindings),
                Page::VideoDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
                Page::DynamicDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
                Page::ArticleDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
                Page::BangumiDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
                Page::Up(page) => page.draw(frame, area, &self.theme, &self.keybindings),
                _ => {}
            }
            self.draw_playback_error(frame, area);
            self.draw_status_notice(frame, area);
            self.draw_help(frame, area);
            self.draw_login_prompt(frame, area);
            return;
        }

        // Main layout with sidebar
        let chunks = if self.show_sidebar {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(16), // Sidebar
                    Constraint::Min(40),    // Content
                ])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(40)])
                .split(area)
        };

        if self.show_sidebar && chunks.len() > 1 {
            self.sidebar.draw(frame, chunks[0], &self.theme);
            self.draw_page(frame, chunks[1]);
        } else {
            self.draw_page(frame, chunks[0]);
        }
        self.draw_playback_error(frame, area);
        self.draw_playback_starting(frame, area);
        self.draw_status_notice(frame, area);
        self.draw_help(frame, area);
        self.draw_login_prompt(frame, area);
    }

    fn draw_help(&mut self, frame: &mut Frame, area: Rect) {
        if self.show_help {
            let groups = crate::ui::help_for(&self.current_page, &self.keybindings);
            self.help_scroll =
                crate::ui::render_help(frame, area, &groups, &self.theme, self.help_scroll);
        }
    }

    fn draw_login_prompt(&self, frame: &mut Frame, area: Rect) {
        let Some(reason) = self.login_prompt.as_deref() else {
            return;
        };
        let width = 52.min(area.width);
        let height = 7.min(area.height);
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        let footer = crate::ui::shortcut_footer(
            &self.theme,
            [
                (
                    self.keybindings.confirm.clone(),
                    "重新登录".into(),
                    self.theme.success,
                ),
                (
                    self.keybindings.back.clone(),
                    "稍后".into(),
                    self.theme.info,
                ),
            ],
        );
        let text = vec![
            Line::from(Span::styled(
                reason.to_string(),
                Style::default().fg(self.theme.warning),
            )),
            Line::from(""),
            Line::from("保存的登录信息已无法使用，需要重新扫码登录。"),
            footer,
        ];
        let dialog = Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(self.theme.bg_primary))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(self.theme.warning))
                    .title(" 登录失效 "),
            );
        frame.render_widget(Clear, popup);
        frame.render_widget(dialog, popup);
    }

    /// The bottom-row notice for app-level failures. A lost network worker
    /// stays visible because nothing can load until the app restarts.
    fn status_notice_text(&self) -> Option<&str> {
        if self.network_worker_lost.get() {
            return Some("网络线程已停止，无法加载内容；请重启应用");
        }
        self.status_notice.as_deref()
    }

    fn draw_status_notice(&self, frame: &mut Frame, area: Rect) {
        if self.playback.last_error.is_some()
            || self.playback.status == crate::domain::playback::PlaybackStatus::Starting
        {
            return;
        }
        let Some(notice) = self.status_notice_text() else {
            return;
        };
        let bar = Rect {
            x: area.x,
            y: area.bottom().saturating_sub(1),
            width: area.width,
            height: 1.min(area.height),
        };
        let message = Paragraph::new(notice)
            .style(Style::default().fg(self.theme.warning))
            .alignment(Alignment::Center);
        frame.render_widget(Clear, bar);
        frame.render_widget(message, bar);
    }

    fn draw_playback_starting(&self, frame: &mut Frame, area: Rect) {
        if self.playback.status != crate::domain::playback::PlaybackStatus::Starting
            || self.playback.last_error.is_some()
        {
            return;
        }
        let bar = Rect {
            x: area.x,
            y: area.bottom().saturating_sub(1),
            width: area.width,
            height: 1.min(area.height),
        };
        let message = Paragraph::new("正在启动播放器...")
            .style(Style::default().fg(self.theme.warning))
            .alignment(Alignment::Center);
        frame.render_widget(Clear, bar);
        frame.render_widget(message, bar);
    }

    fn draw_playback_error(&self, frame: &mut Frame, area: Rect) {
        let Some(error) = self.playback.last_error.as_deref() else {
            return;
        };
        let popup = Rect {
            x: area.x,
            y: area.bottom().saturating_sub(3),
            width: area.width,
            height: 3.min(area.height),
        };
        let message = Paragraph::new(error)
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title(" 播放错误 "));
        frame.render_widget(message, popup);
    }

    fn draw_page(&mut self, frame: &mut Frame, area: Rect) {
        match &mut self.current_page {
            Page::Login(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::Home(page) => {
                if let Some(notice) = self.pending_home_notice.take() {
                    page.set_footer_notice(notice);
                }
                page.draw(frame, area, &self.theme, &self.keybindings);
            }
            Page::Search(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::Dynamic(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::DynamicDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::ArticleDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::VideoDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::History(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::Favorites(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::Live(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::LiveDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::Settings(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::Bangumi(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::BangumiDetail(page) => page.draw(frame, area, &self.theme, &self.keybindings),
            Page::Up(page) => page.draw(frame, area, &self.theme, &self.keybindings),
        }
    }

    async fn handle_input(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // Playback errors remain visible until the user acknowledges them with
        // the next key press; the key still performs its normal action.
        self.playback.last_error = None;
        self.status_notice = None;
        let (key, modifiers) = strip_alt_gr(key, modifiers);
        if is_ctrl_c(key, modifiers) {
            self.handle_interrupt().await;
            return;
        }
        let Some((key, modifiers)) = self.resolve_chord(key, modifiers) else {
            return;
        };
        if self.show_help {
            if !self.scroll_help(key) {
                self.show_help = false;
            }
            return;
        }
        if is_help_key(key, modifiers) && !self.current_page_captures_keys() {
            self.show_help = true;
            self.help_scroll = 0;
            return;
        }
        if self.login_prompt.is_some() {
            if self.keybindings.matches_confirm(key) {
                self.login_prompt = None;
                self.handle_action(crate::application::AppAction::SwitchToLogin)
                    .await;
            } else if self.keybindings.matches_back(key) || self.keybindings.matches_quit(key) {
                self.login_prompt = None;
            }
            return;
        }
        // Stopping works from every page, but only while a player runs and
        // no text field or key capture would receive the key instead.
        if self.playback.is_active()
            && self.keybindings.matches_stop_playback(key)
            && !self.current_page_captures_keys()
        {
            self.stop_playback();
            return;
        }
        // The theme key works on every page unless a page binds it too.
        if !self.current_page_captures_keys() && self.keybindings.is_dedicated_theme_key(key) {
            self.handle_action(crate::application::AppAction::NextTheme)
                .await;
            return;
        }
        if let Some(action) = self.send_key_to_page(key, modifiers) {
            self.handle_action(action).await;
        }
    }

    fn send_key_to_page(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<crate::application::AppAction> {
        let keys = &self.keybindings;
        match &mut self.current_page {
            Page::Login(page) => page.handle_input(key, keys),
            Page::Home(page) => page.handle_input_with_modifiers(key, modifiers, keys),
            Page::Search(page) => page.handle_input_with_modifiers(key, modifiers, keys),
            Page::Dynamic(page) => page.handle_input_with_modifiers(key, modifiers, keys),
            Page::DynamicDetail(page) => page.handle_input_with_modifiers(key, modifiers, keys),
            Page::ArticleDetail(page) => page.handle_input_with_modifiers(key, modifiers, keys),
            Page::VideoDetail(page) => page.handle_input_with_modifiers(key, modifiers, keys),
            Page::History(page) => page.handle_input_with_modifiers(key, modifiers, keys),
            Page::Favorites(page) => page.handle_input(key, keys),
            Page::Live(page) => page.handle_input(key, keys),
            Page::LiveDetail(page) => page.handle_input_with_modifiers(key, modifiers, keys),
            Page::Settings(page) => page.handle_input(key, keys),
            Page::Bangumi(page) => page.handle_input(key, keys),
            Page::BangumiDetail(page) => page.handle_input(key, keys),
            Page::Up(page) => page.handle_input(key, keys),
        }
    }

    /// Ctrl+C leaves a focused text field the way Esc does, so a copy
    /// reflex does not throw away a draft; anywhere else it quits.
    async fn handle_interrupt(&mut self) {
        if self.show_help || self.login_prompt.is_some() {
            self.show_help = false;
            self.login_prompt = None;
            return;
        }
        if self.current_page_text_field_active() {
            if let Some(action) = self.send_key_to_page(KeyCode::Esc, KeyModifiers::NONE) {
                self.handle_action(action).await;
            }
            return;
        }
        self.handle_action(crate::application::AppAction::Quit)
            .await;
    }

    /// Decide what a key held with Ctrl or Alt means. Text fields get every
    /// chord for their readline shortcuts. Elsewhere a few chords alias list
    /// keys (Ctrl+N/P, Ctrl+D/U, Ctrl+F/B), History keeps Ctrl+A/Ctrl+R, and
    /// every other chord is dropped instead of acting as its bare letter.
    fn resolve_chord(
        &self,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<(KeyCode, KeyModifiers)> {
        if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return Some((key, modifiers));
        }
        if self.current_page_text_field_active() && !self.show_help {
            return Some((key, modifiers));
        }
        if matches!(&self.current_page, Page::Settings(page) if page.is_capturing_input()) {
            return None;
        }
        if matches!(&self.current_page, Page::History(_))
            && modifiers == KeyModifiers::CONTROL
            && matches!(key, KeyCode::Char('a' | 'r'))
            && !self.show_help
            && self.login_prompt.is_none()
        {
            return Some((key, modifiers));
        }
        chord_alias(key, modifiers, &self.keybindings).map(|key| (key, KeyModifiers::NONE))
    }

    /// Move the shortcut list for scroll keys; returns `false` for keys
    /// that should close it.
    fn scroll_help(&mut self, key: KeyCode) -> bool {
        const PAGE: u16 = 10;
        let keys = &self.keybindings;
        self.help_scroll = if keys.matches_down(key) {
            self.help_scroll.saturating_add(1)
        } else if keys.matches_up(key) {
            self.help_scroll.saturating_sub(1)
        } else if keys.matches_page_down(key) || key == KeyCode::Char(' ') {
            self.help_scroll.saturating_add(PAGE)
        } else if keys.matches_page_up(key) {
            self.help_scroll.saturating_sub(PAGE)
        } else if keys.matches_jump_top(key) {
            0
        } else if keys.matches_jump_bottom(key) {
            u16::MAX
        } else {
            return false;
        };
        true
    }

    /// Bracketed paste: the text goes into the focused text field as one
    /// line. On Home and Search a paste starts a search. Anywhere else it
    /// is dropped, so pasted letters never run as shortcuts.
    async fn handle_paste(&mut self, text: &str) {
        if self.show_help || self.login_prompt.is_some() {
            return;
        }
        self.playback.last_error = None;
        self.status_notice = None;
        match &mut self.current_page {
            Page::Home(page) if !page.search_input_active() => page.begin_search(),
            Page::Search(page) if !page.input_mode => page.focus_input(),
            _ => {}
        }
        if !self.current_page_text_field_active() {
            return;
        }
        let mut actions = Vec::new();
        for character in pasted_line(text).chars() {
            match self.send_key_to_page(KeyCode::Char(character), KeyModifiers::NONE) {
                None | Some(crate::application::AppAction::None) => {}
                Some(action) => actions.push(action),
            }
        }
        for action in actions {
            self.handle_action(action).await;
        }
    }

    /// A left click on a sidebar entry opens that page. Returns whether the
    /// event was a sidebar click.
    async fn handle_sidebar_click(&mut self, event: MouseEvent, frame_area: Rect) -> bool {
        if !self.show_sidebar
            || self.current_page_uses_full_area()
            || event.kind
                != crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)
        {
            return false;
        }
        let content = self.get_content_area(frame_area);
        let sidebar_area = Rect {
            width: content.x.saturating_sub(frame_area.x),
            ..frame_area
        };
        if !sidebar_area.contains(Position::new(event.column, event.row)) {
            return false;
        }
        self.playback.last_error = None;
        self.status_notice = None;
        if let Some(item) = self.sidebar.item_at(sidebar_area, event.column, event.row) {
            self.send_network_command(crate::application::network::NetworkCommand::CancelPending);
            self.sidebar.select(item);
            self.switch_to_nav_page().await;
        }
        true
    }

    async fn handle_mouse(&mut self, event: MouseEvent, area: Rect) {
        let action = match &mut self.current_page {
            Page::Login(page) => page.handle_mouse(event, area),
            Page::Home(page) => page.handle_mouse(event, area),
            Page::Search(page) => page.handle_mouse(event, area),
            Page::Dynamic(page) => page.handle_mouse(event, area),
            Page::DynamicDetail(page) => page.handle_mouse(event, area),
            Page::ArticleDetail(page) => page.handle_mouse(event, area),
            Page::VideoDetail(page) => page.handle_mouse(event, area),
            Page::History(page) => page.handle_mouse(event, area),
            Page::Favorites(page) => page.handle_mouse(event, area),
            Page::Live(page) => page.handle_mouse(event, area),
            Page::LiveDetail(page) => page.handle_mouse(event, area),
            Page::Settings(page) => page.handle_mouse(event, area),
            Page::Bangumi(page) => page.handle_mouse(event, area),
            Page::BangumiDetail(page) => page.handle_mouse(event, area),
            Page::Up(page) => page.handle_mouse(event, area),
        };

        if let Some(action) = action {
            self.handle_action(action).await;
        }
    }

    pub(super) async fn tick(&mut self) {
        self.drain_network_events();
        self.collect_player_hubs();
        for action in std::mem::take(&mut self.deferred_actions) {
            self.handle_action(action).await;
        }
        let qrcode_poll = match &mut self.current_page {
            Page::Login(page) => page.poll_request(),
            _ => None,
        };
        let suggestion_term = match &mut self.current_page {
            Page::Search(page) => page.suggestion_request(),
            Page::Home(page) => page.search_mut().suggestion_request(),
            _ => None,
        };
        if let Some(term) = suggestion_term
            && !self.config.minimal_mode
        {
            let req_id = self.next_request_id("search_suggest");
            self.send_network_command(
                crate::application::network::NetworkCommand::LoadSearchSuggestions { req_id, term },
            );
        }
        if let Some(qrcode_key) = qrcode_poll {
            let req_id = self.next_request_id("login_poll");
            self.send_network_command(crate::application::network::NetworkCommand::PollQrcode {
                req_id,
                qrcode_key,
            });
        }
        while let Ok(event) = self.playback_event_rx.try_recv() {
            let accepted = self.playback.apply_event(&event);
            match event {
                crate::domain::playback::PlaybackEvent::Finished {
                    session_id,
                    bvid: Some(bvid),
                } => {
                    if accepted
                        && self.auto_return_after_playback.as_ref()
                            == Some(&(session_id, bvid.clone()))
                        && matches!(&self.current_page, Page::VideoDetail(page) if page.bvid == bvid)
                    {
                        self.auto_return_after_playback = None;
                        self.handle_action(crate::application::AppAction::BackToList)
                            .await;
                    } else if self
                        .auto_return_after_playback
                        .as_ref()
                        .is_some_and(|(id, _)| *id == session_id)
                    {
                        self.auto_return_after_playback = None;
                    }
                }
                crate::domain::playback::PlaybackEvent::Notice { message, .. } if accepted => {
                    self.status_notice = Some(message);
                }
                crate::domain::playback::PlaybackEvent::Failed { session_id, .. }
                    if accepted
                        && self
                            .auto_return_after_playback
                            .as_ref()
                            .is_some_and(|(id, _)| *id == session_id) =>
                {
                    self.auto_return_after_playback = None;
                }
                _ => {}
            }
        }

        let auto_play = if self.config.auto_play {
            match &mut self.current_page {
                Page::VideoDetail(page)
                    if page.auto_play_pending && !page.loading && page.video_info.is_some() =>
                {
                    page.auto_play_pending = false;
                    Some((Some(page.bvid.clone()), page.play_action()))
                }
                Page::BangumiDetail(page)
                    if page.auto_play_pending && !page.loading && page.season.is_some() =>
                {
                    page.auto_play_pending = false;
                    page.play_action().map(|action| (None, action))
                }
                _ => None,
            }
        } else {
            // Auto-play disabled: clear the pending flag so it doesn't fire
            // once the user re-enables the setting mid-session.
            if let Page::VideoDetail(page) = &mut self.current_page {
                page.auto_play_pending = false;
            }
            if let Page::BangumiDetail(page) = &mut self.current_page {
                page.auto_play_pending = false;
            }
            None
        };
        if let Some((return_bvid, action)) = auto_play {
            self.handle_action(action).await;
            if let (Some(bvid), Some(session_id)) = (return_bvid, self.playback.session_id) {
                self.auto_return_after_playback = Some((session_id, bvid));
            }
        }
        match &mut self.current_page {
            Page::Login(_) => {}
            Page::Home(page) => {
                // Non-blocking: poll completed downloads and start new ones
                page.poll_cover_results();
                page.start_cover_downloads();
            }
            Page::Search(page) => {
                page.poll_cover_results();
                page.start_cover_downloads();
            }
            Page::Dynamic(page) => {
                page.poll_cover_results();
                page.start_cover_downloads();
            }
            Page::VideoDetail(page) => {
                page.poll_cover_results();
                page.start_cover_downloads();
            }
            Page::ArticleDetail(page) => {
                page.poll_image_results();
                page.start_image_downloads();
            }
            Page::DynamicDetail(page) => {
                page.poll_image_results();
                page.start_image_downloads();
            }
            Page::Live(page) => {
                page.poll_cover_results();
                page.start_cover_downloads();
            }
            Page::History(page) => {
                page.poll_cover_results();
                page.start_cover_downloads();
            }
            Page::Favorites(page) => {
                page.videos.poll_cover_results();
                page.videos.start_cover_downloads();
            }
            Page::Bangumi(page) => {
                page.index_grid.poll_cover_results();
                page.index_grid.start_cover_downloads();
            }
            Page::Up(page) => {
                page.videos.poll_cover_results();
                page.videos.start_cover_downloads();
                page.favorite_videos.poll_cover_results();
                page.favorite_videos.start_cover_downloads();
            }
            _ => {}
        }
    }
}

/// Windows terminals report AltGr as Ctrl+Alt; a symbol typed that way is
/// a plain character, not a chord.
fn strip_alt_gr(key: KeyCode, modifiers: KeyModifiers) -> (KeyCode, KeyModifiers) {
    let alt_gr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    match key {
        KeyCode::Char(character)
            if modifiers.contains(alt_gr) && !character.is_ascii_alphanumeric() =>
        {
            (key, modifiers - alt_gr)
        }
        _ => (key, modifiers),
    }
}

fn is_ctrl_c(key: KeyCode, modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::CONTROL && matches!(key, KeyCode::Char('c' | 'C'))
}

/// `?` (fixed so it is always discoverable) or F1.
fn is_help_key(key: KeyCode, modifiers: KeyModifiers) -> bool {
    !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && (key == KeyCode::Char(crate::ui::HELP_KEY) || key == KeyCode::F(1))
}

/// The plain key a readline/less-style chord stands for outside text
/// fields: Ctrl+N/P move by one, Ctrl+D/F and Ctrl+U/B by a page.
fn chord_alias(
    key: KeyCode,
    modifiers: KeyModifiers,
    keys: &crate::storage::Keybindings,
) -> Option<KeyCode> {
    let KeyCode::Char(character) = key else {
        return None;
    };
    if modifiers != KeyModifiers::CONTROL {
        return None;
    }
    let parse = crate::storage::Keybindings::parse_keycode;
    match character.to_ascii_lowercase() {
        'n' => Some(KeyCode::Down),
        'p' => Some(KeyCode::Up),
        'd' | 'f' => parse(&keys.page_down),
        'u' | 'b' => parse(&keys.page_up),
        _ => None,
    }
}

/// The most characters one paste inserts; inserting is per character.
const MAX_PASTE_CHARS: usize = 4096;

/// Pasted text as one line: line breaks and tabs become spaces, other
/// control characters are dropped and trailing line breaks are trimmed.
fn pasted_line(text: &str) -> String {
    text.trim_end_matches(['\r', '\n'])
        .replace("\r\n", " ")
        .chars()
        .filter_map(|character| match character {
            '\n' | '\r' | '\t' => Some(' '),
            character if character.is_control() => None,
            character => Some(character),
        })
        .take(MAX_PASTE_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::minimal_app;
    use crate::application::AppAction;
    use crate::presentation::tui::{NavItem, SettingsPage, UpPage};

    fn logged_in_minimal_app() -> App {
        let (mut app, _commands) = minimal_app();
        app.credentials = Some(crate::infrastructure::persistence::Credentials {
            sessdata: String::new(),
            bili_jct: String::new(),
            dede_user_id: "1".into(),
            dede_user_id_ckmd5: None,
            refresh_token: None,
        });
        app
    }

    fn select_own_space(app: &mut App) {
        app.sidebar.select(NavItem::MySpace);
        app.current_page = Page::Up(Box::new(UpPage::new_sidebar(1)));
    }

    fn app_with_active_player() -> (App, tokio_util::sync::CancellationToken) {
        let (mut app, _rx) = minimal_app();
        app.keybindings = crate::storage::Keybindings::default();
        let stop = tokio_util::sync::CancellationToken::new();
        app.player_stop = Some(stop.clone());
        app.playback.begin_starting(3);
        (app, stop)
    }

    #[tokio::test]
    async fn stop_key_stops_the_running_player_and_ignores_its_exit() {
        let (mut app, stop) = app_with_active_player();

        app.handle_input(KeyCode::Char('x'), KeyModifiers::NONE)
            .await;

        assert!(stop.is_cancelled());
        assert!(!app.playback.is_active());
        assert!(
            !app.playback
                .apply_event(&crate::domain::playback::PlaybackEvent::Failed {
                    session_id: 3,
                    error: "MPV exited".into(),
                })
        );
        assert!(app.playback.last_error.is_none());
    }

    #[tokio::test]
    async fn stop_key_is_typed_into_a_focused_search_box() {
        let (mut app, stop) = app_with_active_player();
        let mut page = crate::presentation::tui::SearchPage::new();
        page.input_mode = true;
        app.current_page = Page::Search(page);

        app.handle_input(KeyCode::Char('x'), KeyModifiers::NONE)
            .await;

        assert!(!stop.is_cancelled());
        assert!(app.playback.is_active());
        assert!(matches!(&app.current_page, Page::Search(page) if page.query == "x"));
    }

    #[tokio::test]
    async fn starting_a_player_stops_the_previous_one() {
        let (mut app, _rx) = minimal_app();
        let (control_tx, control_rx) = std::sync::mpsc::channel();
        for session_id in [1, 2] {
            let control_tx = control_tx.clone();
            app.start_playback(session_id, "test", move |control| {
                let _ = control_tx.send(control.stop.clone());
                async move {
                    control.stop.cancelled().await;
                    Ok(())
                }
            });
        }
        let first = control_rx.recv().unwrap();
        let second = control_rx.recv().unwrap();

        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        assert_eq!(app.playback.session_id, Some(2));
    }

    #[tokio::test]
    async fn lost_network_worker_is_reported_and_stays_visible() {
        let (mut app, rx) = minimal_app();
        drop(rx);
        assert!(app.status_notice_text().is_none());

        app.handle_action(AppAction::CancelPendingLoads).await;

        assert!(app.status_notice_text().unwrap().contains("网络线程已停止"));
        app.handle_input(KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert!(app.status_notice_text().is_some());
    }

    #[tokio::test]
    async fn failed_save_shows_a_notice_until_the_next_key() {
        let (mut app, _rx) = minimal_app();
        app.report_save_result("保存设置", Err(anyhow::anyhow!("permission denied")));
        assert_eq!(
            app.status_notice_text(),
            Some("保存设置失败：permission denied")
        );

        app.handle_input(KeyCode::Char('j'), KeyModifiers::NONE)
            .await;

        assert!(app.status_notice_text().is_none());
    }

    #[tokio::test]
    async fn minimal_sidebar_tab_cycle_reaches_settings_after_own_space() {
        let mut app = logged_in_minimal_app();
        app.sidebar.select(NavItem::Settings);
        app.current_page =
            Page::Settings(Box::new(SettingsPage::default().with_minimal_mode(true)));
        for expected in [
            NavItem::Search,
            NavItem::Dynamic,
            NavItem::History,
            NavItem::Favorites,
            NavItem::MySpace,
            NavItem::Settings,
        ] {
            app.handle_input(KeyCode::Tab, KeyModifiers::NONE).await;
            assert_eq!(app.sidebar.selected, expected);
        }
        assert!(matches!(app.current_page, Page::Settings(_)));
    }

    #[tokio::test]
    async fn minimal_sidebar_reverse_tab_crosses_own_space_while_loading() {
        let mut app = logged_in_minimal_app();
        app.sidebar.select(NavItem::Settings);
        app.current_page = Page::Settings(Box::default());
        app.handle_input(KeyCode::BackTab, KeyModifiers::SHIFT)
            .await;
        assert!(
            matches!(&app.current_page, Page::Up(page) if page.is_sidebar_page && page.loading)
        );
        app.handle_input(KeyCode::BackTab, KeyModifiers::SHIFT)
            .await;
        assert_eq!(app.sidebar.selected, NavItem::Favorites);
        assert!(matches!(app.current_page, Page::Favorites(_)));
    }

    #[tokio::test]
    async fn minimal_own_space_escape_keeps_sidebar_navigation_reachable() {
        let mut app = logged_in_minimal_app();
        select_own_space(&mut app);
        app.handle_input(KeyCode::Esc, KeyModifiers::NONE).await;
        assert_eq!(app.sidebar.selected, NavItem::Favorites);
        app.handle_input(KeyCode::Tab, KeyModifiers::NONE).await;
        app.handle_input(KeyCode::Tab, KeyModifiers::NONE).await;
        assert_eq!(app.sidebar.selected, NavItem::Settings);
        assert!(matches!(app.current_page, Page::Settings(_)));
    }

    #[tokio::test]
    async fn minimal_video_back_restores_own_space_before_tab_to_settings() {
        let mut app = logged_in_minimal_app();
        select_own_space(&mut app);
        app.handle_action(AppAction::OpenVideoDetail("BV1test".into(), 1))
            .await;
        app.handle_input(KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(matches!(&app.current_page, Page::Up(page) if page.is_sidebar_page));
        assert_eq!(app.sidebar.selected, NavItem::MySpace);
        app.handle_input(KeyCode::Tab, KeyModifiers::NONE).await;
        assert!(matches!(app.current_page, Page::Settings(_)));
    }

    #[tokio::test]
    async fn minimal_own_space_error_keeps_configured_navigation_keys_working() {
        let mut app = logged_in_minimal_app();
        select_own_space(&mut app);
        if let Page::Up(page) = &mut app.current_page {
            page.set_error("加载失败".into());
        }
        app.keybindings.nav_next_page = "n".into();
        app.handle_input(KeyCode::Char('n'), KeyModifiers::NONE)
            .await;
        assert_eq!(app.sidebar.selected, NavItem::Settings);
        assert!(matches!(app.current_page, Page::Settings(_)));
    }

    #[tokio::test]
    async fn minimal_own_space_draw_keeps_sidebar_and_content_area_consistent() {
        let mut app = logged_in_minimal_app();
        select_own_space(&mut app);
        let area = Rect::new(0, 0, 100, 30);
        assert_eq!(app.get_content_area(area), Rect::new(16, 0, 84, 30));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.replace(' ', "").contains("我的主页"));
        assert!(content.replace(' ', "").contains("设置"));
        app.current_page = Page::Up(Box::new(UpPage::new(1)));
        assert_eq!(app.get_content_area(area), area);
    }

    #[tokio::test]
    async fn login_prompt_takes_keys_until_answered() {
        let (mut app, _rx) = minimal_app();
        app.keybindings = crate::storage::Keybindings::default();
        app.login_prompt = Some("登录已失效".into());

        app.handle_input(KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert!(app.login_prompt.is_some());

        app.handle_input(KeyCode::Enter, KeyModifiers::NONE).await;
        assert!(app.login_prompt.is_none());
        assert!(matches!(app.current_page, Page::Login(_)));
    }

    #[tokio::test]
    async fn login_prompt_is_dismissed_with_back() {
        let (mut app, _rx) = minimal_app();
        app.keybindings = crate::storage::Keybindings::default();
        app.login_prompt = Some("登录已失效".into());
        app.handle_input(KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(app.login_prompt.is_none());
        assert!(!matches!(app.current_page, Page::Login(_)));
    }

    #[tokio::test]
    async fn clicking_a_sidebar_entry_opens_its_page() {
        let (mut app, _rx) = minimal_app();
        app.config.minimal_mode = false;
        app.sidebar.set_minimal_mode(false);
        app.current_page = Page::Settings(Box::default());
        app.sidebar.select(NavItem::Settings);
        let click = MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 4,
            row: 9, // fifth entry: Live
            modifiers: KeyModifiers::NONE,
        };

        assert!(
            app.handle_sidebar_click(click, Rect::new(0, 0, 100, 30))
                .await
        );

        assert_eq!(app.sidebar.selected, NavItem::Live);
        assert!(matches!(app.current_page, Page::Live(_)));
    }

    #[tokio::test]
    async fn ctrl_c_quits_but_only_closes_a_focused_text_field() {
        let (mut app, _rx) = minimal_app();
        app.keybindings = crate::storage::Keybindings::default();
        let mut page = crate::presentation::tui::SearchPage::new();
        page.input_mode = true;
        page.query = "draft".into();
        app.current_page = Page::Search(page);

        app.handle_input(KeyCode::Char('c'), KeyModifiers::CONTROL)
            .await;
        assert!(!app.should_quit);
        assert!(matches!(&app.current_page, Page::Search(page) if !page.input_mode));

        app.handle_input(KeyCode::Char('c'), KeyModifiers::CONTROL)
            .await;
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn ctrl_chords_do_not_run_their_bare_letter() {
        let (mut app, _rx) = minimal_app();
        app.keybindings = crate::storage::Keybindings::default();
        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1".into(), 1);
        page.loading = false;
        app.current_page = Page::VideoDetail(Box::new(page));

        // `c` opens the comment box; Ctrl+C must not, and Ctrl+Q must not quit.
        app.handle_input(KeyCode::Char('q'), KeyModifiers::CONTROL)
            .await;
        assert!(!app.should_quit);
        assert!(matches!(&app.current_page, Page::VideoDetail(page) if !page.input_mode));
    }

    #[test]
    fn readline_chords_alias_list_keys() {
        let keys = crate::storage::Keybindings::default();
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(
            chord_alias(KeyCode::Char('n'), ctrl, &keys),
            Some(KeyCode::Down)
        );
        assert_eq!(
            chord_alias(KeyCode::Char('p'), ctrl, &keys),
            Some(KeyCode::Up)
        );
        assert_eq!(
            chord_alias(KeyCode::Char('d'), ctrl, &keys),
            Some(KeyCode::PageDown)
        );
        assert_eq!(
            chord_alias(KeyCode::Char('u'), ctrl, &keys),
            Some(KeyCode::PageUp)
        );
        assert_eq!(chord_alias(KeyCode::Char('w'), ctrl, &keys), None);
        assert_eq!(
            chord_alias(KeyCode::Char('n'), KeyModifiers::ALT, &keys),
            None
        );
    }

    #[test]
    fn alt_gr_symbols_are_plain_characters() {
        let alt_gr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(
            strip_alt_gr(KeyCode::Char('@'), alt_gr),
            (KeyCode::Char('@'), KeyModifiers::NONE)
        );
        assert_eq!(
            strip_alt_gr(KeyCode::Char('a'), alt_gr),
            (KeyCode::Char('a'), alt_gr)
        );
    }

    #[test]
    fn pasted_text_becomes_one_line() {
        assert_eq!(pasted_line("BV1xx411c7mD\n"), "BV1xx411c7mD");
        assert_eq!(pasted_line("a\r\nb\tc\u{7}d"), "a b cd");
        assert_eq!(
            pasted_line(&"字".repeat(MAX_PASTE_CHARS + 5))
                .chars()
                .count(),
            MAX_PASTE_CHARS
        );
    }

    #[tokio::test]
    async fn paste_goes_to_the_search_box_and_never_runs_shortcuts() {
        let (mut app, _rx) = minimal_app();
        app.keybindings = crate::storage::Keybindings::default();
        app.current_page = Page::Search(crate::presentation::tui::SearchPage::new());
        app.handle_paste("q 测试\n").await;
        assert!(!app.should_quit);
        assert!(
            matches!(&app.current_page, Page::Search(page) if page.input_mode && page.query == "q 测试")
        );

        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1".into(), 1);
        page.loading = false;
        app.current_page = Page::VideoDetail(Box::new(page));
        app.handle_paste("q").await;
        assert!(!app.should_quit, "a paste outside a text field is dropped");
    }

    #[tokio::test]
    async fn help_scrolls_with_list_keys_and_closes_on_others() {
        let (mut app, _rx) = minimal_app();
        app.keybindings = crate::storage::Keybindings::default();
        app.handle_input(KeyCode::F(1), KeyModifiers::NONE).await;
        assert!(app.show_help);
        app.handle_input(KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        app.handle_input(KeyCode::Char('d'), KeyModifiers::CONTROL)
            .await;
        assert!(app.show_help);
        assert_eq!(app.help_scroll, 11);
        app.handle_input(KeyCode::Home, KeyModifiers::NONE).await;
        assert_eq!(app.help_scroll, 0);
        app.handle_input(KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(!app.show_help);
    }
}
