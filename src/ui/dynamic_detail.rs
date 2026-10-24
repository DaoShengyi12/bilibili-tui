//! Dynamic detail page for viewing image/text dynamics

use super::{Component, Theme, footer_notice, shortcut_footer};
use crate::api::comment::CommentItem;
use crate::api::dynamic::DynamicItem;
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
    pub comment_page: i32,
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
            comment_page: 1,
            content_scroll: 0,
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

    /// Marks the next comment page as loading and returns the comment
    /// target and page number, or `None` when there is nothing to load.
    pub fn begin_load_more_comments(&mut self) -> Option<(i64, i32, i32)> {
        if !self.has_more_comments || self.loading_more_comments {
            return None;
        }
        let item = self.dynamic_item.as_ref()?;
        let oid = item.comment_oid(&self.dynamic_id)?;
        let comment_type = item.comment_type();
        self.loading_more_comments = true;
        Some((oid, comment_type, self.comment_page + 1))
    }

    pub fn apply_more_comments(&mut self, page: i32, replies: Option<Vec<CommentItem>>) {
        match replies {
            Some(replies) if !replies.is_empty() => {
                self.comments.extend(replies);
                self.comment_page = page;
            }
            _ => self.has_more_comments = false,
        }
        self.loading_more_comments = false;
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

            if let Some(text) = content_text
                && !text.is_empty()
            {
                lines.push("📝 动态内容:".to_string());
                lines.push(String::new());
                for line in wrap_text(text, 60) {
                    lines.push(format!("  {}", line));
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

            block.push(format!(
                "💬 {} [Lv.{}]  👍 {}  {}",
                comment.author_name(),
                level,
                comment.format_like(),
                comment.format_time()
            ));

            // Comment content
            let message = comment.message();
            for line in wrap_text(message, 80) {
                block.push(format!("   {}", line));
            }

            // Reply count
            if comment.reply_count() > 0 {
                block.push(format!("   └─ {} 条回复", comment.reply_count()));
            }

            // Add separator except for last comment
            if idx < self.comments.len() - 1 {
                block.push(String::new());
                block.push("─".repeat(80));
            }
            block.push(String::new());

            comment_blocks.push(block);
        }

        if comment_blocks.is_empty() {
            comment_blocks.push(vec!["暂无评论".to_string()]);
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
                    ("n".into(), "加载更多".into(), theme.info),
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
                    ("n".into(), "加载更多".into(), theme.info),
                    (keys.back.clone(), "返回".into(), theme.info),
                ],
            )
        };
        let help = Paragraph::new(help).alignment(Alignment::Center);
        frame.render_widget(help, help_chunk);
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
        // 'n' for load more comments (page-specific)
        if key == KeyCode::Char('n') {
            return Some(AppAction::LoadMoreComments);
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
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(content, area);
    }

    fn draw_comments(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
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
                    .title(format!(" 评论 ({}) ", self.comments.len())),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(comments, area);
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0;

    for word in text.split_whitespace() {
        let word_width = word.chars().count();
        if current_width + word_width + 1 > width && !current_line.is_empty() {
            lines.push(current_line.clone());
            current_line.clear();
            current_width = 0;
        }

        if !current_line.is_empty() {
            current_line.push(' ');
            current_width += 1;
        }
        current_line.push_str(word);
        current_width += word_width;
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        lines.push(text.to_string());
    }

    lines
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
}
