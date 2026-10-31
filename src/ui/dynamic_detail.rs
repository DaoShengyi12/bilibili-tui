//! Dynamic detail page for viewing image/text dynamics

use super::comments::{CommentPanel, CommentTarget, PanelRequest};
use super::text::wrap_to_width;
use super::{Component, Theme, footer_notice, shortcut_footer};
use crate::api::dynamic::{DynamicItem, DynamicKind};
use crate::application::AppAction;
use crate::storage::Keybindings;
use image::DynamicImage;
use ratatui::{
    crossterm::event::{KeyCode, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Lines describing a dynamic's video, article or live room.
fn push_major_lines(lines: &mut Vec<String>, item: &DynamicItem, width: usize) {
    match item.kind() {
        DynamicKind::Video => {
            lines.push(format!(
                "{} 视频: {}",
                super::icons::VIDEO,
                item.video_title().unwrap_or("无标题")
            ));
            lines.push(format!(
                "   {} {}  {} {}  {}",
                super::icons::PLAY,
                item.video_play(),
                super::icons::DANMAKU,
                item.video_danmaku(),
                item.video_duration()
            ));
        }
        DynamicKind::Article => {
            lines.push(format!(
                "{} 专栏: {}",
                super::icons::ARTICLE,
                item.summary()
            ));
            if let Some(desc) = item.article_desc() {
                for line in wrap_to_width(desc, width.saturating_sub(3)) {
                    lines.push(format!("   {line}"));
                }
            }
        }
        DynamicKind::Live => {
            if let Some(live) = item.live() {
                let state = if live.is_live {
                    "直播中"
                } else {
                    "未开播"
                };
                lines.push(format!(
                    "{} 直播间 {} · {state}: {}",
                    super::icons::LIVE,
                    live.room_id,
                    live.title
                ));
                if let Some(watched) = live.watched {
                    lines.push(format!("   {watched}"));
                }
            }
        }
        _ => return,
    }
    lines.push(String::new());
}

/// Where the play key goes from a dynamic: its own video, article or live
/// room, or the forwarded one's.
fn linked_content_action(item: &DynamicItem) -> Option<AppAction> {
    match item.kind() {
        DynamicKind::Video => Some(AppAction::OpenVideoDetail(
            item.video_bvid()?.to_string(),
            0,
        )),
        DynamicKind::Article => Some(AppAction::OpenArticle(item.article()?.0)),
        DynamicKind::Live => Some(AppAction::OpenLiveDetail(item.live()?.room_id)),
        DynamicKind::Forward => {
            let orig = item.forwarded()?;
            linked_content_action(orig)
                .or_else(|| orig.id_str.clone().map(AppAction::OpenDynamicDetail))
        }
        _ => None,
    }
}

/// Image download result
pub struct ImageResult {
    pub index: usize,
    pub protocol: StatefulProtocol,
}

pub struct DynamicDetailPage {
    pub dynamic_id: String,
    pub dynamic_item: Option<DynamicItem>,
    pub comments: CommentPanel,
    pub loading: bool,
    pub error_message: Option<String>,
    pub content_scroll: usize,
    pub image_urls: Vec<String>,
    pub image_protocols: Vec<Option<StatefulProtocol>>,
    pub current_image_index: usize,
    pub picker: Arc<Picker>,
    // Async image loading
    image_tx: mpsc::Sender<ImageResult>,
    image_rx: mpsc::Receiver<ImageResult>,
    pending_downloads: HashSet<usize>,
    pub input_mode: bool,
    /// The thread and comment a reply goes to, and the name shown.
    reply_to: Option<(i64, i64, String)>,
    pub input_buffer: String,
    /// Characters after the cursor in `input_buffer`.
    pub input_cursor: usize,
    /// Tab moves the keys between the comments (default) and the text,
    /// which then scrolls and pages images with Left/Right.
    pub content_focused: bool,
    /// Rows the text could scroll and rows on screen, from the last frame.
    content_max_scroll: std::cell::Cell<usize>,
    content_visible_rows: std::cell::Cell<usize>,
    /// Transient message shown in the footer until the next key press.
    pub notice: Option<String>,
    /// Inner width of the detail pane from the last frame. Lines are
    /// wrapped to it so that one string is one screen row.
    content_width: std::cell::Cell<usize>,
}

impl DynamicDetailPage {
    pub fn new(dynamic_id: String) -> Self {
        let picker = super::image_picker();
        let (image_tx, image_rx) = mpsc::channel(8);

        Self {
            dynamic_id,
            dynamic_item: None,
            comments: CommentPanel::new(),
            loading: true,
            error_message: None,
            content_scroll: 0,
            content_width: std::cell::Cell::new(60),
            image_urls: Vec::new(),
            image_protocols: Vec::new(),
            current_image_index: 0,
            picker,
            image_tx,
            image_rx,
            pending_downloads: HashSet::new(),
            input_mode: false,
            reply_to: None,
            input_buffer: String::new(),
            input_cursor: 0,
            content_focused: false,
            content_max_scroll: std::cell::Cell::new(0),
            content_visible_rows: std::cell::Cell::new(1),
            notice: None,
        }
    }

    /// Reopen the comment box with a draft that could not be posted.
    pub fn restore_comment_draft(&mut self, message: String, notice: String) {
        self.input_mode = true;
        self.input_buffer = message;
        self.input_cursor = 0;
        self.notice = Some(notice);
    }

    /// Open the comment box. A draft closed with Esc comes back when the
    /// box reopens for the same reply.
    fn open_input(&mut self, reply_to: Option<(i64, i64, String)>) {
        if self.reply_to != reply_to {
            self.input_buffer.clear();
            self.input_cursor = 0;
        }
        self.input_mode = true;
        self.reply_to = reply_to;
    }

    /// Scroll the text by `delta` rows within what the last frame showed.
    fn scroll_content(&mut self, delta: isize) {
        self.content_scroll = self
            .content_scroll
            .saturating_add_signed(delta)
            .min(self.content_max_scroll.get());
    }

    /// Start background downloads for images (non-blocking)
    pub fn start_image_downloads(&mut self) {
        self.comments.start_picture_downloads();
        for (idx, url) in self.image_urls.iter().enumerate() {
            // Skip if already has protocol or is pending
            if self.image_protocols[idx].is_some() || self.pending_downloads.contains(&idx) {
                continue;
            }

            self.pending_downloads.insert(idx);
            let tx = self.image_tx.clone();
            let picker = Arc::clone(&self.picker);
            let url = url.clone();

            tokio::spawn(async move {
                if let Some(img) = Self::download_image(&url).await {
                    let protocol = picker.new_resize_protocol(img);
                    let _ = tx
                        .send(ImageResult {
                            index: idx,
                            protocol,
                        })
                        .await;
                }
            });
        }
    }

    /// Poll for completed image downloads (non-blocking)
    pub fn poll_image_results(&mut self) {
        self.comments.poll_picture_results();
        while let Ok(result) = self.image_rx.try_recv() {
            if result.index < self.image_protocols.len() {
                self.image_protocols[result.index] = Some(result.protocol);
                self.pending_downloads.remove(&result.index);
            }
        }
    }

    async fn download_image(url: &str) -> Option<DynamicImage> {
        let response = reqwest::get(url).await.ok()?;
        let bytes = response.bytes().await.ok()?;
        image::load_from_memory(&bytes).ok()
    }

    /// The comment area of the loaded dynamic.
    pub fn comment_target(&self) -> Option<CommentTarget> {
        let item = self.dynamic_item.as_ref()?;
        Some(CommentTarget {
            oid: item.comment_oid(&self.dynamic_id)?,
            comment_type: item.comment_type(),
        })
    }

    /// Turn a panel request into an action; `Compose` opens the reply box.
    fn comment_action(&mut self, request: PanelRequest) -> AppAction {
        if let PanelRequest::Compose { root, parent, to } = request {
            let reply_to = root
                .zip(parent)
                .zip(to)
                .map(|((root, parent), to)| (root, parent, to));
            self.open_input(reply_to);
            return AppAction::None;
        }
        match self.comment_target() {
            Some(target) => AppAction::Comment(target, request),
            None => AppAction::None,
        }
    }

    fn get_content_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        if let Some(ref item) = self.dynamic_item {
            // Author and time
            lines.push(format!("{} {}", super::icons::ACCOUNT, item.author_name()));
            lines.push(format!("{} {}", super::icons::CLOCK, item.pub_time()));
            lines.push(String::new());

            // Image count
            if !self.image_urls.is_empty() {
                lines.push(format!(
                    "{} {} 张图片",
                    super::icons::PICTURE,
                    self.image_urls.len()
                ));
                lines.push(String::new());
            }

            // Dynamic content (both draw desc and opus text)
            let content_text = if let Some(text) = item.opus_text() {
                Some(text)
            } else {
                item.desc_text()
            };

            let width = self.content_width.get().saturating_sub(2);
            if let Some(text) = content_text
                && !text.is_empty()
            {
                for line in wrap_to_width(text, width) {
                    lines.push(format!("  {}", line));
                }
                lines.push(String::new());
            }
            push_major_lines(&mut lines, item, width);
            if item.kind() == DynamicKind::Forward {
                lines.push(format!("{} 转发自", super::icons::FORWARD));
                match item.forwarded() {
                    Some(orig) => {
                        lines.push(format!("  @{}  {}", orig.author_name(), orig.pub_time()));
                        let text = orig.opus_text().or(orig.desc_text()).unwrap_or_default();
                        for line in wrap_to_width(text, width.saturating_sub(2)) {
                            lines.push(format!("    {line}"));
                        }
                        push_major_lines(&mut lines, orig, width.saturating_sub(2));
                    }
                    None => lines.push("  原动态已删除".to_string()),
                }
                lines.push(String::new());
            }
        } else {
            lines.push("加载中...".to_string());
        }

        lines
    }
}

impl Component for DynamicDetailPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        // Adjust layout based on input mode
        let chunks = if self.input_mode {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Title
                    Constraint::Min(8),    // Main content
                    Constraint::Length(3), // Input box
                    Constraint::Length(2), // Help
                ])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Title
                    Constraint::Min(10),   // Main content
                    Constraint::Length(2), // Help
                ])
                .split(area)
        };

        // Title
        let title_text = if let Some(ref item) = self.dynamic_item {
            if item.is_draw() || item.is_opus() {
                "图文动态详情"
            } else {
                "📄 动态详情"
            }
        } else {
            "📄 动态详情"
        };

        let title = Paragraph::new(title_text)
            .style(
                Style::default()
                    .fg(theme.fg_accent)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_unfocused)),
            );
        frame.render_widget(title, chunks[0]);

        // Main content
        if self.loading {
            let loading_text = "加载中...";
            let loading = Paragraph::new(loading_text)
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.border_focused)),
                );
            frame.render_widget(loading, chunks[1]);
        } else if let Some(ref err) = self.error_message {
            let error_text = format!("错误: {}", err);
            let error = Paragraph::new(error_text)
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Red)),
                );
            frame.render_widget(error, chunks[1]);
        } else {
            self.draw_main_layout(frame, chunks[1], theme);
        }

        // Input box (only in input mode)
        if self.input_mode {
            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.bilibili_pink))
                .title(Span::styled(
                    match &self.reply_to {
                        Some((_, _, to)) => format!(" 回复 @{to} "),
                        None => " 发表评论 ".to_string(),
                    },
                    Style::default()
                        .fg(theme.bilibili_pink)
                        .add_modifier(Modifier::BOLD),
                ));

            let input_text = super::text::input_with_cursor(
                &self.input_buffer,
                self.input_cursor,
                usize::from(chunks[2].width.saturating_sub(2)),
            );
            let input = Paragraph::new(input_text)
                .style(Style::default().fg(theme.fg_primary))
                .block(input_block);
            frame.render_widget(input, chunks[2]);
        }

        // Help
        let help_chunk = if self.input_mode {
            chunks[3]
        } else {
            chunks[2]
        };
        let help = if let Some(notice) = &self.notice {
            footer_notice(theme, notice)
        } else if self.input_mode {
            shortcut_footer(
                theme,
                [
                    ("Enter".to_string(), "发送评论".into(), theme.success),
                    ("Esc".to_string(), "收起（保留草稿）".into(), theme.info),
                    ("Ctrl+U".to_string(), "清空".into(), theme.fg_muted),
                ],
            )
        } else if !self.image_urls.is_empty() {
            super::hint_footer(
                theme,
                [
                    (
                        format!("{}/{}", keys.nav_left, keys.nav_right),
                        "图片".into(),
                        theme.fg_accent,
                    ),
                    (
                        keys.nav_next_page.clone(),
                        if self.content_focused {
                            "切到评论"
                        } else {
                            "切到正文"
                        }
                        .into(),
                        theme.info,
                    ),
                    (keys.comment.clone(), "评论".into(), theme.info),
                    (keys.play.clone(), "打开原内容".into(), theme.success),
                    (keys.back.clone(), "返回".into(), theme.info),
                ],
            )
        } else {
            super::hint_footer(
                theme,
                [
                    (
                        keys.nav_next_page.clone(),
                        if self.content_focused {
                            "切到评论"
                        } else {
                            "切到正文"
                        }
                        .into(),
                        theme.info,
                    ),
                    (keys.comment.clone(), "评论".into(), theme.info),
                    (keys.play.clone(), "打开原内容".into(), theme.success),
                    (keys.back.clone(), "返回".into(), theme.info),
                ],
            )
        };
        let help = Paragraph::new(help).alignment(Alignment::Center);
        frame.render_widget(help, help_chunk);
    }

    fn handle_input_with_modifiers(
        &mut self,
        key: KeyCode,
        modifiers: crossterm::event::KeyModifiers,
        keys: &crate::storage::Keybindings,
    ) -> Option<AppAction> {
        if self.input_mode
            && !super::text::is_input_cancel(key, keys)
            && key != KeyCode::Enter
            && super::text::edit_at_cursor(
                &mut self.input_buffer,
                &mut self.input_cursor,
                key,
                modifiers,
            )
            .is_some()
        {
            self.notice = None;
            return Some(AppAction::None);
        }
        self.handle_input(key, keys)
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        keys: &crate::storage::Keybindings,
    ) -> Option<AppAction> {
        self.notice = None;
        // Handle input mode for adding comments
        if self.input_mode {
            match key {
                _ if super::text::is_input_cancel(key, keys) => {
                    // The draft stays for the next time the box opens.
                    self.input_mode = false;
                    return Some(AppAction::None);
                }
                KeyCode::Enter => {
                    let message = self.input_buffer.trim().to_string();
                    if !message.is_empty()
                        && let Some(target) = self.comment_target()
                    {
                        self.input_buffer.clear();
                        self.input_cursor = 0;
                        self.input_mode = false;
                        // The reply target stays so a failed post reopens
                        // as the same reply.
                        let reply_to = self.reply_to.clone();
                        return Some(AppAction::AddComment {
                            oid: target.oid,
                            comment_type: target.comment_type,
                            message,
                            root: reply_to.as_ref().map(|(root, _, _)| *root),
                            parent: reply_to.as_ref().map(|(_, parent, _)| *parent),
                        });
                    }
                    return Some(AppAction::None);
                }
                _ => {
                    super::text::edit_at_cursor(
                        &mut self.input_buffer,
                        &mut self.input_cursor,
                        key,
                        crossterm::event::KeyModifiers::NONE,
                    );
                    return Some(AppAction::None);
                }
            }
        }

        if keys.matches_back(key) && self.comments.close_thread() {
            return Some(AppAction::None);
        }
        if keys.matches_quit(key) || keys.matches_back(key) {
            return Some(AppAction::BackToList);
        }
        if keys.matches_comment_sort(key) {
            if self.loading || self.comment_target().is_none() {
                return Some(AppAction::None);
            }
            let request = self.comments.begin_sort_toggle();
            return Some(self.comment_action(request));
        }
        if keys.matches_nav_next(key) || keys.matches_nav_prev(key) {
            self.content_focused = !self.content_focused;
            return Some(AppAction::None);
        }
        if self.content_focused
            && let Some(action) = self.handle_content_key(key, keys)
        {
            return Some(action);
        }
        // Left/Right page images; everything else list-like goes to comments.
        if keys.matches_left(key)
            && !self.image_urls.is_empty()
            && self.comments.open_thread_root().is_none()
        {
            // Previous image
            if !self.image_urls.is_empty() && self.current_image_index > 0 {
                self.current_image_index -= 1;
            }
            return Some(AppAction::None);
        }
        if keys.matches_right(key)
            && !self.image_urls.is_empty()
            && self.comments.open_thread_root().is_none()
            && self
                .comments
                .selected_comment()
                .is_none_or(|c| c.reply_count() == 0)
        {
            // Next image
            if self.current_image_index + 1 < self.image_urls.len() {
                self.current_image_index += 1;
            }
            return Some(AppAction::None);
        }
        if keys.matches_play(key) {
            return Some(
                self.dynamic_item
                    .as_ref()
                    .and_then(linked_content_action)
                    .unwrap_or(AppAction::None),
            );
        }
        if keys.matches_open_up(key)
            && let Some(mid) = self.dynamic_item.as_ref().and_then(DynamicItem::author_mid)
        {
            return Some(AppAction::OpenUpPage(mid));
        }
        if self.content_focused {
            // Only writing a new comment reaches the unfocused comments.
            if keys.matches_comment(key) && self.comment_target().is_some() {
                self.open_input(None);
            }
            return Some(AppAction::None);
        }
        if let Some(handled) = self.comments.handle_key(key, keys) {
            return Some(handled.map_or(AppAction::None, |request| self.comment_action(request)));
        }
        Some(AppAction::None)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> Option<AppAction> {
        if self.content_focused
            && matches!(
                event.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            )
        {
            self.scroll_content(if event.kind == MouseEventKind::ScrollDown {
                1
            } else {
                -1
            });
            return Some(AppAction::None);
        }
        // Don't handle mouse in input mode
        if self.input_mode {
            return None;
        }

        self.comments
            .handle_mouse(event)
            .map(|request| self.comment_action(request))
    }
}

impl DynamicDetailPage {
    /// Keys for the focused text: scroll it and page the images. `None`
    /// leaves the key to the shared handling below.
    fn handle_content_key(
        &mut self,
        key: KeyCode,
        keys: &crate::storage::Keybindings,
    ) -> Option<AppAction> {
        let page = self.content_visible_rows.get().max(1) as isize;
        if keys.matches_down(key) {
            self.scroll_content(1);
        } else if keys.matches_up(key) {
            self.scroll_content(-1);
        } else if keys.matches_page_down(key) {
            self.scroll_content(page);
        } else if keys.matches_page_up(key) {
            self.scroll_content(-page);
        } else if keys.matches_jump_top(key) {
            self.content_scroll = 0;
        } else if keys.matches_jump_bottom(key) {
            self.content_scroll = self.content_max_scroll.get();
        } else if keys.matches_left(key) {
            self.current_image_index = self.current_image_index.saturating_sub(1);
        } else if keys.matches_right(key) {
            if self.current_image_index + 1 < self.image_urls.len() {
                self.current_image_index += 1;
            }
        } else {
            return None;
        }
        Some(AppAction::None)
    }

    fn draw_main_layout(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let has_images = !self.image_urls.is_empty();

        // Main layout: left side images (if any), right side content+comments
        let main_chunks = if has_images {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(50), // Images
                    Constraint::Percentage(50), // Content + Comments
                ])
                .split(area)
        } else {
            Layout::default()
                .constraints([Constraint::Percentage(100)])
                .split(area)
        };

        // Draw images on the left
        if has_images {
            self.draw_images(frame, main_chunks[0], theme);
        }

        // Right side: content details + comments
        let right_area = if has_images {
            main_chunks[1]
        } else {
            main_chunks[0]
        };

        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40), // Details
                Constraint::Percentage(60), // Comments
            ])
            .split(right_area);

        self.draw_details(frame, right_chunks[0], theme);
        self.draw_comments(frame, right_chunks[1], theme);
    }

    fn draw_images(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_unfocused))
            .title(format!(
                " 图片 {}/{} ",
                self.current_image_index + 1,
                self.image_urls.len()
            ));

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        // Render the current image if loaded
        if let Some(Some(protocol)) = self.image_protocols.get_mut(self.current_image_index) {
            use ratatui_image::StatefulImage;
            let image_widget = StatefulImage::new();
            frame.render_stateful_widget(image_widget, inner_area, protocol);
        } else {
            // Show loading text
            let loading_text = if self.pending_downloads.contains(&self.current_image_index) {
                "加载中..."
            } else {
                "等待加载..."
            };
            let loading = Paragraph::new(loading_text)
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center);
            frame.render_widget(loading, inner_area);
        }
    }

    fn draw_details(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.content_width
            .set(area.width.saturating_sub(2) as usize);
        let content_lines = self.get_content_lines();
        let visible_height = area.height.saturating_sub(2) as usize;
        let max_scroll = content_lines.len().saturating_sub(visible_height);
        self.content_max_scroll.set(max_scroll);
        self.content_visible_rows.set(visible_height);

        let display_lines: Vec<Line> = content_lines
            .iter()
            .skip(self.content_scroll.min(max_scroll))
            .take(visible_height)
            .map(|line| Line::from(line.as_str()))
            .collect();

        let content = Paragraph::new(display_lines)
            .style(Style::default().fg(theme.fg_primary))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(if self.content_focused {
                        theme.border_focused
                    } else {
                        theme.border_unfocused
                    }))
                    .title(if max_scroll > 0 {
                        format!(
                            " 动态详情 {}/{} ",
                            self.content_scroll.min(max_scroll) + 1,
                            max_scroll + 1
                        )
                    } else {
                        " 动态详情 ".to_string()
                    }),
            );
        frame.render_widget(content, area);
    }

    fn draw_comments(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.comments
            .render(frame, area, !self.content_focused, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_keys_use_the_dynamic_comment_target() {
        let mut page = DynamicDetailPage::new("1".into());
        page.loading = false;
        page.dynamic_item = Some(
            serde_json::from_value(serde_json::json!({
                "id_str": "1",
                "basic": {"comment_type": 11, "comment_id_str": "42"}
            }))
            .unwrap(),
        );
        page.comments
            .set_comments(crate::api::comment::CommentBatch {
                comments: vec![
                    serde_json::from_value(serde_json::json!({
                        "rpid": 9, "oid": 42, "mid": 1, "parent": 0
                    }))
                    .unwrap(),
                ],
                next_offset: None,
            });
        let action = page.handle_input(KeyCode::Enter, &Keybindings::default());
        assert!(matches!(
            action,
            Some(AppAction::Comment(
                CommentTarget {
                    oid: 42,
                    comment_type: 11
                },
                PanelRequest::Like { rpid: 9, .. }
            ))
        ));
    }

    #[test]
    fn play_key_opens_the_forwarded_video() {
        let mut page = DynamicDetailPage::new("1".into());
        page.loading = false;
        page.dynamic_item = Some(
            serde_json::from_value(serde_json::json!({
                "id_str": "1",
                "type": "DYNAMIC_TYPE_FORWARD",
                "orig": {"id_str": "2", "modules": {"module_dynamic": {"major": {
                    "type": "MAJOR_TYPE_ARCHIVE", "archive": {"bvid": "BV1orig"}
                }}}}
            }))
            .unwrap(),
        );
        let action = page.handle_input(KeyCode::Char('p'), &Keybindings::default());
        assert!(matches!(action, Some(AppAction::OpenVideoDetail(bvid, _)) if bvid == "BV1orig"));
        assert!(
            page.get_content_lines()
                .iter()
                .any(|line| line.contains("转发自"))
        );
    }
}
