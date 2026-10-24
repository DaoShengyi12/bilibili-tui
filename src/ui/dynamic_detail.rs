//! Dynamic detail page for viewing image/text dynamics

use super::text::wrap_to_width;
use super::{Component, Theme, footer_notice, shortcut_footer};
use crate::api::comment::{CommentBatch, CommentItem, CommentSort};
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
                "🎬 视频: {}",
                item.video_title().unwrap_or("无标题")
            ));
            lines.push(format!(
                "   ▶ {}  💬 {}  {}",
                item.video_play(),
                item.video_danmaku(),
                item.video_duration()
            ));
        }
        DynamicKind::Article => {
            lines.push(format!("📰 专栏: {}", item.summary()));
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
                    "📺 直播间 {} · {state}: {}",
                    live.room_id, live.title
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

/// A comment page to load for a dynamic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentRequest {
    pub oid: i64,
    pub comment_type: i32,
    pub sort: CommentSort,
    /// `None` loads the first page and replaces the list.
    pub offset: Option<String>,
}

/// Image download result
pub struct ImageResult {
    pub index: usize,
    pub protocol: StatefulProtocol,
}

pub struct DynamicDetailPage {
    pub dynamic_id: String,
    pub dynamic_item: Option<DynamicItem>,
    pub comments: Vec<CommentItem>,
    pub loading: bool,
    pub error_message: Option<String>,
    /// Cursor of the next comment page; `None` once all are loaded.
    pub comment_offset: Option<String>,
    pub comment_sort: CommentSort,
    pub content_scroll: usize,
    pub comment_scroll: usize,
    pub has_more_comments: bool,
    pub loading_more_comments: bool,
    pub image_urls: Vec<String>,
    pub image_protocols: Vec<Option<StatefulProtocol>>,
    pub current_image_index: usize,
    pub picker: Arc<Picker>,
    // Async image loading
    image_tx: mpsc::Sender<ImageResult>,
    image_rx: mpsc::Receiver<ImageResult>,
    pending_downloads: HashSet<usize>,
    // Comment action support
    pub liked_comments: HashSet<i64>,
    pub input_mode: bool,
    pub input_buffer: String,
    /// Transient message shown in the footer until the next key press.
    pub notice: Option<String>,
    pub selected_comment: usize,
    /// Inner widths of the detail and comment panes from the last frame.
    /// Lines are wrapped to them so that one string is one screen row and
    /// scroll positions computed from line counts match the screen.
    content_width: std::cell::Cell<usize>,
    comment_width: std::cell::Cell<usize>,
}

impl DynamicDetailPage {
    pub fn new(dynamic_id: String) -> Self {
        let picker = super::image_picker();
        let (image_tx, image_rx) = mpsc::channel(8);

        Self {
            dynamic_id,
            dynamic_item: None,
            comments: Vec::new(),
            loading: true,
            error_message: None,
            comment_offset: None,
            comment_sort: CommentSort::default(),
            content_scroll: 0,
            content_width: std::cell::Cell::new(60),
            comment_width: std::cell::Cell::new(80),
            comment_scroll: 0,
            has_more_comments: true,
            loading_more_comments: false,
            image_urls: Vec::new(),
            image_protocols: Vec::new(),
            current_image_index: 0,
            picker,
            image_tx,
            image_rx,
            pending_downloads: HashSet::new(),
            liked_comments: HashSet::new(),
            input_mode: false,
            input_buffer: String::new(),
            notice: None,
            selected_comment: 0,
        }
    }

    /// Start background downloads for images (non-blocking)
    /// Reopen the comment box with a draft that could not be posted.
    pub fn restore_comment_draft(&mut self, message: String, notice: String) {
        self.input_mode = true;
        self.input_buffer = message;
        self.notice = Some(notice);
    }

    pub fn start_image_downloads(&mut self) {
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

    /// The comment area of the loaded dynamic: `(oid, type)`.
    pub fn comment_target(&self) -> Option<(i64, i32)> {
        let item = self.dynamic_item.as_ref()?;
        Some((item.comment_oid(&self.dynamic_id)?, item.comment_type()))
    }

    /// Marks the next comment page as loading and returns the comment
    /// target, sort and cursor, or `None` when there is nothing to load.
    pub fn begin_load_more_comments(&mut self) -> Option<CommentRequest> {
        if !self.has_more_comments || self.loading_more_comments {
            return None;
        }
        let (oid, comment_type) = self.comment_target()?;
        let offset = self.comment_offset.clone()?;
        self.loading_more_comments = true;
        Some(CommentRequest {
            oid,
            comment_type,
            sort: self.comment_sort,
            offset: Some(offset),
        })
    }

    pub fn set_comments(&mut self, batch: CommentBatch) {
        self.comments = batch.comments;
        self.has_more_comments = batch.next_offset.is_some();
        self.comment_offset = batch.next_offset;
        self.comment_scroll = 0;
        self.selected_comment = 0;
        self.loading_more_comments = false;
    }

    /// A comment page for `sort`; pages for a sort no longer selected are
    /// dropped.
    pub fn apply_comment_batch(&mut self, sort: CommentSort, append: bool, batch: CommentBatch) {
        if sort != self.comment_sort {
            return;
        }
        if !append {
            self.set_comments(batch);
            return;
        }
        self.comments.extend(batch.comments);
        self.has_more_comments = batch.next_offset.is_some();
        self.comment_offset = batch.next_offset;
        self.loading_more_comments = false;
    }

    /// Switch between hot and newest-first comments and return the first
    /// page to load, or `None` while the dynamic is still loading.
    pub fn begin_comment_sort_toggle(&mut self) -> Option<CommentRequest> {
        if self.loading {
            return None;
        }
        let (oid, comment_type) = self.comment_target()?;
        self.comment_sort = self.comment_sort.toggled();
        self.set_comments(CommentBatch::default());
        self.loading_more_comments = true;
        Some(CommentRequest {
            oid,
            comment_type,
            sort: self.comment_sort,
            offset: None,
        })
    }

    pub fn apply_more_comments_error(&mut self) {
        self.loading_more_comments = false;
    }

    fn get_content_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        if let Some(ref item) = self.dynamic_item {
            // Author and time
            lines.push(format!("👤 UP主: {}", item.author_name()));
            lines.push(format!("🕒 发布时间: {}", item.pub_time()));
            lines.push(String::new());

            // Image count
            if !self.image_urls.is_empty() {
                lines.push(format!("🖼️  图片数量: {} 张", self.image_urls.len()));
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
                lines.push("📝 动态内容:".to_string());
                lines.push(String::new());
                for line in wrap_to_width(text, width) {
                    lines.push(format!("  {}", line));
                }
                lines.push(String::new());
            }
            push_major_lines(&mut lines, item, width);
            if item.kind() == DynamicKind::Forward {
                lines.push("↪ 转发自:".to_string());
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

    /// Move the selected comment and scroll so its first line is at the top.
    /// Selection and scroll used to advance independently (comment index vs.
    /// line index), so Enter could like a comment that was not on screen.
    fn move_comment_selection(&mut self, delta: isize) {
        if self.comments.is_empty() {
            self.selected_comment = 0;
            self.comment_scroll = 0;
            return;
        }
        let last = self.comments.len() - 1;
        self.selected_comment = self.selected_comment.saturating_add_signed(delta).min(last);
        self.comment_scroll = self.comment_start_line(self.selected_comment);
    }

    fn comment_start_line(&self, index: usize) -> usize {
        self.get_comment_lines()
            .iter()
            .take(index)
            .map(Vec::len)
            .sum()
    }

    fn get_comment_lines(&self) -> Vec<Vec<String>> {
        let mut comment_blocks = Vec::new();

        for (idx, comment) in self.comments.iter().enumerate() {
            let mut block = Vec::new();

            // Comment header
            let level = comment
                .member
                .as_ref()
                .and_then(|m| m.level_info.as_ref())
                .and_then(|l| l.current_level)
                .unwrap_or(0);

            let width = self.comment_width.get();
            // The selected comment's header gets a one-column marker.
            block.extend(wrap_to_width(
                &format!(
                    "💬 {} [Lv.{}]  👍 {}  {}",
                    comment.author_name(),
                    level,
                    comment.format_like(),
                    comment.format_time()
                ),
                width.saturating_sub(1),
            ));

            // Comment content
            let message = comment.message();
            for line in wrap_to_width(message, width.saturating_sub(3)) {
                block.push(format!("   {}", line));
            }

            // Reply count
            if comment.reply_count() > 0 {
                block.push(format!("   └─ {} 条回复", comment.reply_count()));
            }

            // Add separator except for last comment
            if idx < self.comments.len() - 1 {
                block.push(String::new());
                block.push("─".repeat(width));
            }
            block.push(String::new());

            comment_blocks.push(block);
        }

        if comment_blocks.is_empty() {
            let text = if self.loading_more_comments {
                "⏳ 加载评论中..."
            } else {
                "暂无评论"
            };
            comment_blocks.push(vec![text.to_string()]);
        }

        comment_blocks
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
                "📷 图文动态详情"
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
                    " ✏️ 发表评论 ",
                    Style::default()
                        .fg(theme.bilibili_pink)
                        .add_modifier(Modifier::BOLD),
                ));

            let input_text = format!("{}_", self.input_buffer);
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
                    (keys.confirm.clone(), "发送评论".into(), theme.success),
                    (keys.back.clone(), "取消".into(), theme.info),
                ],
            )
        } else if !self.image_urls.is_empty() {
            shortcut_footer(
                theme,
                [
                    (
                        format!("{}/{}", keys.nav_left, keys.nav_right),
                        "图片".into(),
                        theme.fg_accent,
                    ),
                    (
                        format!("{}/{}", keys.nav_up, keys.nav_down),
                        "滚动".into(),
                        theme.fg_accent,
                    ),
                    (keys.confirm.clone(), "点赞".into(), theme.success),
                    (keys.comment.clone(), "评论".into(), theme.info),
                    (
                        keys.comment_sort.clone(),
                        self.comment_sort.toggled().label().into(),
                        theme.info,
                    ),
                    (keys.load_more.clone(), "加载更多".into(), theme.info),
                    (keys.play.clone(), "打开原内容".into(), theme.success),
                    (keys.back.clone(), "返回".into(), theme.info),
                ],
            )
        } else {
            shortcut_footer(
                theme,
                [
                    (
                        format!("{}/{}", keys.nav_up, keys.nav_down),
                        "滚动".into(),
                        theme.fg_accent,
                    ),
                    (keys.confirm.clone(), "点赞".into(), theme.success),
                    (keys.comment.clone(), "评论".into(), theme.info),
                    (
                        keys.comment_sort.clone(),
                        self.comment_sort.toggled().label().into(),
                        theme.info,
                    ),
                    (keys.load_more.clone(), "加载更多".into(), theme.info),
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
        if self.input_mode && super::text::apply_line_edit(&mut self.input_buffer, key, modifiers) {
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
                KeyCode::Esc => {
                    self.input_mode = false;
                    self.input_buffer.clear();
                    return Some(AppAction::None);
                }
                KeyCode::Enter => {
                    if !self.input_buffer.is_empty()
                        && let Some(ref item) = self.dynamic_item
                    {
                        let comment_type = item.comment_type();
                        if let Some(oid) = item.comment_oid(&self.dynamic_id) {
                            let message = self.input_buffer.clone();
                            self.input_buffer.clear();
                            self.input_mode = false;
                            return Some(AppAction::AddComment {
                                oid,
                                comment_type,
                                message,
                                root: None,
                            });
                        }
                    }
                    return Some(AppAction::None);
                }
                KeyCode::Backspace => {
                    self.input_buffer.pop();
                    return Some(AppAction::None);
                }
                KeyCode::Char(c) => {
                    self.input_buffer.push(c);
                    return Some(AppAction::None);
                }
                _ => return Some(AppAction::None),
            }
        }

        if keys.matches_quit(key) || keys.matches_back(key) {
            return Some(AppAction::BackToList);
        }
        if keys.matches_comment(key) {
            self.input_mode = true;
            self.input_buffer.clear();
            return Some(AppAction::None);
        }
        if keys.matches_left(key) {
            // Previous image
            if !self.image_urls.is_empty() && self.current_image_index > 0 {
                self.current_image_index -= 1;
            }
            return Some(AppAction::None);
        }
        if keys.matches_right(key) {
            // Next image
            if self.current_image_index + 1 < self.image_urls.len() {
                self.current_image_index += 1;
            }
            return Some(AppAction::None);
        }
        if keys.matches_load_more(key) {
            return Some(AppAction::LoadMoreComments);
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
        if keys.matches_comment_sort(key) {
            return Some(AppAction::ToggleCommentSort);
        }
        if keys.matches_down(key) {
            self.move_comment_selection(1);
            return Some(AppAction::None);
        }
        if keys.matches_up(key) {
            self.move_comment_selection(-1);
            return Some(AppAction::None);
        }
        if keys.matches_confirm(key) {
            // Like the currently selected comment
            if let Some(ref item) = self.dynamic_item
                && self.selected_comment < self.comments.len()
            {
                let comment = &self.comments[self.selected_comment];
                let comment_type = item.comment_type();
                if let Some(oid) = item.comment_oid(&self.dynamic_id) {
                    return Some(AppAction::LikeComment {
                        oid,
                        rpid: comment.rpid,
                        comment_type,
                    });
                }
            }
            return Some(AppAction::None);
        }
        Some(AppAction::None)
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> Option<AppAction> {
        // Don't handle mouse in input mode
        if self.input_mode {
            return None;
        }

        match event.kind {
            MouseEventKind::ScrollDown => {
                self.move_comment_selection(1);
                None
            }
            MouseEventKind::ScrollUp => {
                self.move_comment_selection(-1);
                None
            }
            _ => None,
        }
    }
}

impl DynamicDetailPage {
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
            .border_style(Style::default().fg(theme.border_focused))
            .title(format!(
                " 图片 {}/{} [h/l 切换] ",
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

        let display_lines: Vec<Line> = content_lines
            .iter()
            .skip(self.content_scroll)
            .take(visible_height)
            .map(|line| Line::from(line.as_str()))
            .collect();

        let content = Paragraph::new(display_lines)
            .style(Style::default().fg(theme.fg_primary))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_focused))
                    .title(" 动态详情 "),
            );
        frame.render_widget(content, area);
    }

    fn draw_comments(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.comment_width
            .set(area.width.saturating_sub(2) as usize);
        let comment_blocks = self.get_comment_lines();
        let has_comments = !self.comments.is_empty();

        // Flatten all comment blocks, marking the selected comment's lines so
        // the like target is visible.
        let mut all_lines = Vec::new();
        for (index, block) in comment_blocks.into_iter().enumerate() {
            let selected = has_comments && index == self.selected_comment;
            for (line_index, line) in block.into_iter().enumerate() {
                all_lines.push((selected, line_index == 0, line));
            }
        }

        let visible_height = area.height.saturating_sub(2) as usize;
        let selected_style = Style::default().fg(theme.fg_accent);

        let display_lines: Vec<Line> = all_lines
            .iter()
            .skip(self.comment_scroll)
            .take(visible_height)
            .map(
                |(selected, is_header, line)| match (*selected, *is_header) {
                    (true, true) => Line::from(format!("▶{line}"))
                        .style(selected_style.add_modifier(Modifier::BOLD)),
                    (true, false) => Line::from(line.as_str()).style(selected_style),
                    _ => Line::from(line.as_str()),
                },
            )
            .collect();

        let comments = Paragraph::new(display_lines)
            .style(Style::default().fg(theme.fg_primary))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_focused))
                    .title(format!(
                        " 评论 ({}) · {} ",
                        self.comments.len(),
                        self.comment_sort.label()
                    )),
            );
        frame.render_widget(comments, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_with_comments(messages: &[&str]) -> DynamicDetailPage {
        let mut page = DynamicDetailPage::new("1".to_string());
        page.comments = messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                serde_json::from_value(serde_json::json!({
                    "rpid": index as i64 + 1,
                    "oid": 1, "mid": 1, "parent": 0,
                    "content": {"message": message}
                }))
                .expect("comment fixture")
            })
            .collect();
        page
    }

    #[test]
    fn comment_lines_fit_the_pane_so_scroll_matches_screen_rows() {
        let page = page_with_comments(&[
            "这是一条没有空格而且非常非常长的中文评论，用来检查换行是否按照终端列宽计算",
            "second",
        ]);
        page.comment_width.set(20);
        let blocks = page.get_comment_lines();
        for line in blocks.iter().flatten() {
            assert!(
                crate::ui::text::display_width(line) <= 20,
                "{line:?} is wider than the pane"
            );
        }
        assert!(blocks[0].iter().any(|line| line == &"─".repeat(20)));
        assert_eq!(page.comment_start_line(1), blocks[0].len());
    }

    #[test]
    fn moving_selection_scrolls_to_the_selected_comment() {
        let mut page = page_with_comments(&["first", "second", "third"]);
        let keys = Keybindings::default();

        page.handle_input(KeyCode::Down, &keys);
        page.handle_input(KeyCode::Down, &keys);
        assert_eq!(page.selected_comment, 2);
        assert_eq!(page.comment_scroll, page.comment_start_line(2));

        page.handle_input(KeyCode::Down, &keys);
        assert_eq!(page.selected_comment, 2);

        page.handle_input(KeyCode::Up, &keys);
        assert_eq!(page.selected_comment, 1);
        assert_eq!(page.comment_scroll, page.comment_start_line(1));
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
