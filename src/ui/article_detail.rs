//! Full-page article reader used by history entries.

use super::comments::{CommentPanel, CommentTarget, PanelRequest};
use super::{Component, Theme, footer_notice, shortcut_footer};
use crate::api::{
    article::{ArticleBlock, ArticleData},
    comment::{CommentBatch, CommentType},
};
use crate::application::AppAction;
use crate::storage::Keybindings;
use image::DynamicImage;
use ratatui::{
    crossterm::event::{KeyCode, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;

struct ImageResult {
    index: usize,
    protocol: Option<StatefulProtocol>,
}

struct ArticleImageState<'a> {
    urls: &'a [String],
    protocols: &'a mut [Option<StatefulProtocol>],
    failed: &'a HashSet<usize>,
}

pub struct ArticleDetailPage {
    pub cvid: i64,
    pub article: Option<ArticleData>,
    pub loading: bool,
    pub error_message: Option<String>,
    blocks: Vec<ArticleBlock>,
    pub comments: CommentPanel,
    /// Keys go to the comment panel instead of scrolling the article.
    comments_focused: bool,
    pub input_mode: bool,
    pub input_buffer: String,
    /// Characters after the cursor in `input_buffer`.
    pub input_cursor: usize,
    reply_to: Option<(i64, i64, String)>,
    pub notice: Option<String>,
    image_urls: Vec<String>,
    image_protocols: Vec<Option<StatefulProtocol>>,
    scroll: u16,
    visible_height: u16,
    picker: Arc<Picker>,
    image_tx: mpsc::Sender<ImageResult>,
    image_rx: mpsc::Receiver<ImageResult>,
    pending_downloads: HashSet<usize>,
    failed_images: HashSet<usize>,
}

impl ArticleDetailPage {
    pub fn new(cvid: i64) -> Self {
        let picker = super::image_picker();
        let (image_tx, image_rx) = mpsc::channel(8);
        Self {
            cvid,
            article: None,
            loading: true,
            error_message: None,
            blocks: Vec::new(),
            comments: CommentPanel::new(),
            comments_focused: false,
            input_mode: false,
            input_buffer: String::new(),
            input_cursor: 0,
            reply_to: None,
            notice: None,
            image_urls: Vec::new(),
            image_protocols: Vec::new(),
            scroll: 0,
            visible_height: 1,
            picker,
            image_tx,
            image_rx,
            pending_downloads: HashSet::new(),
            failed_images: HashSet::new(),
        }
    }

    pub fn set_article(&mut self, article: ArticleData, comments: CommentBatch) {
        let document = article.document();
        self.blocks = document.blocks;
        self.comments.set_comments(comments);
        self.image_urls = document.image_urls;
        self.image_protocols = (0..self.image_urls.len()).map(|_| None).collect();
        self.article = Some(article);
        self.loading = false;
        self.error_message = None;
        self.scroll = 0;
        self.pending_downloads.clear();
        self.failed_images.clear();
    }

    pub fn set_error(&mut self, message: String) {
        self.loading = false;
        self.error_message = Some(message);
    }

    /// Comments of an article live under its article ID.
    pub fn comment_target(&self) -> Option<CommentTarget> {
        let article = self.article.as_ref()?;
        Some(CommentTarget {
            oid: if article.id > 0 {
                article.id
            } else {
                self.cvid
            },
            comment_type: CommentType::Article.as_i32(),
        })
    }

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

    pub fn start_image_downloads(&mut self) {
        self.comments.start_picture_downloads();
        if self.pending_downloads.len() >= 4 {
            return;
        }
        let Some(index) = (0..self.image_urls.len()).find(|index| {
            self.image_protocols[*index].is_none()
                && !self.pending_downloads.contains(index)
                && !self.failed_images.contains(index)
        }) else {
            return;
        };
        let url = self.image_urls[index].clone();
        self.pending_downloads.insert(index);
        let tx = self.image_tx.clone();
        let picker = Arc::clone(&self.picker);
        tokio::spawn(async move {
            let protocol = download_image(&url)
                .await
                .map(|image| picker.new_resize_protocol(image));
            let _ = tx.send(ImageResult { index, protocol }).await;
        });
    }

    pub fn poll_image_results(&mut self) {
        self.comments.poll_picture_results();
        while let Ok(result) = self.image_rx.try_recv() {
            self.pending_downloads.remove(&result.index);
            if let Some(protocol) = result.protocol {
                if let Some(slot) = self.image_protocols.get_mut(result.index) {
                    *slot = Some(protocol);
                }
            } else {
                self.failed_images.insert(result.index);
            }
        }
    }

    fn render_document(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if self.comments_focused {
                theme.border_subtle
            } else {
                theme.border_focused
            }))
            .title(" 正文 ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.visible_height = inner.height.max(1);

        let heights = self
            .blocks
            .iter()
            .map(|block| article_block_height(block, inner.width))
            .collect::<Vec<_>>();
        let total_height = heights.iter().copied().sum::<u16>().max(1);
        self.scroll = self
            .scroll
            .min(total_height.saturating_sub(self.visible_height));

        let viewport_start = self.scroll;
        let viewport_end = viewport_start.saturating_add(self.visible_height);
        let mut document_y = 0u16;
        for (article_block, block_height) in self.blocks.iter().zip(heights) {
            let block_end = document_y.saturating_add(block_height);
            if block_end <= viewport_start {
                document_y = block_end;
                continue;
            }
            if document_y >= viewport_end {
                break;
            }
            let visible_start = document_y.max(viewport_start);
            let visible_end = block_end.min(viewport_end);
            let render_area = Rect::new(
                inner.x,
                inner.y + visible_start.saturating_sub(viewport_start),
                inner.width,
                visible_end.saturating_sub(visible_start),
            );
            if render_area.is_empty() {
                document_y = block_end;
                continue;
            }
            let clipped_rows = visible_start.saturating_sub(document_y);
            match article_block {
                ArticleBlock::Text(text) => {
                    frame.render_widget(
                        Paragraph::new(text.as_str())
                            .wrap(Wrap { trim: false })
                            .scroll((clipped_rows, 0))
                            .style(Style::default().fg(theme.fg_primary)),
                        render_area,
                    );
                }
                ArticleBlock::Embedded(text) => {
                    frame.render_widget(
                        Paragraph::new(text.as_str())
                            .alignment(Alignment::Center)
                            .style(
                                Style::default()
                                    .fg(theme.fg_accent)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        render_area,
                    );
                }
                ArticleBlock::Image { url, alt } => {
                    let mut images = ArticleImageState {
                        urls: &self.image_urls,
                        protocols: &mut self.image_protocols,
                        failed: &self.failed_images,
                    };
                    Self::render_inline_image(frame, render_area, url, alt, theme, &mut images);
                }
            }
            document_y = block_end;
        }
    }

    fn render_inline_image(
        frame: &mut Frame,
        area: Rect,
        url: &str,
        alt: &str,
        theme: &Theme,
        images: &mut ArticleImageState<'_>,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle))
            .title(format!(" {alt} "));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let Some(index) = images.urls.iter().position(|candidate| candidate == url) else {
            return;
        };
        if let Some(Some(protocol)) = images.protocols.get_mut(index) {
            frame.render_stateful_widget(StatefulImage::default(), inner, protocol);
        } else {
            let message = if images.failed.contains(&index) {
                "图片加载失败"
            } else {
                "图片加载中..."
            };
            frame.render_widget(Paragraph::new(message).alignment(Alignment::Center), inner);
        }
    }

    fn render_comments(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.comments
            .render(frame, area, self.comments_focused, theme);
    }
}

impl Component for ArticleDetailPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(if self.input_mode { 3 } else { 0 }),
            Constraint::Length(1),
        ])
        .split(area);
        let title = self
            .article
            .as_ref()
            .map(|article| article.title.as_str())
            .unwrap_or("加载专栏...");
        let metadata = self
            .article
            .as_ref()
            .map(|article| {
                let author = article
                    .author
                    .as_ref()
                    .map(|author| author.name.as_str())
                    .unwrap_or("");
                let published = chrono::DateTime::from_timestamp(article.publish_time, 0)
                    .map(|date| {
                        date.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                match (author.is_empty(), published.is_empty()) {
                    (false, false) => format!("{author} · {published}"),
                    (false, true) => author.to_string(),
                    (true, false) => published,
                    (true, true) => String::new(),
                }
            })
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(format!("{title}\n{metadata}"))
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(theme.fg_primary)
                        .add_modifier(Modifier::BOLD),
                )
                .block(Block::default().borders(Borders::BOTTOM)),
            chunks[0],
        );

        if self.loading {
            frame.render_widget(
                Paragraph::new("加载中...").alignment(Alignment::Center),
                chunks[1],
            );
        } else if let Some(error) = &self.error_message {
            frame.render_widget(
                Paragraph::new(error.as_str())
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(theme.error)),
                chunks[1],
            );
        } else {
            let content =
                Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)])
                    .split(chunks[1]);
            self.render_document(frame, content[0], theme);
            self.render_comments(frame, content[1], theme);
        }

        if self.input_mode {
            let title = match &self.reply_to {
                Some((_, _, to)) => format!(" 回复 @{to} "),
                None => " 发表评论 ".to_string(),
            };
            frame.render_widget(
                Paragraph::new(super::text::input_with_cursor(
                    &self.input_buffer,
                    self.input_cursor,
                    usize::from(chunks[2].width.saturating_sub(2)),
                ))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.bilibili_pink))
                        .title(title),
                ),
                chunks[2],
            );
        }
        let footer = if let Some(notice) = &self.notice {
            footer_notice(theme, notice)
        } else if self.input_mode {
            shortcut_footer(
                theme,
                [
                    ("Enter".to_string(), "发送".into(), theme.success),
                    ("Esc".to_string(), "收起（保留草稿）".into(), theme.info),
                    ("Ctrl+U".to_string(), "清空".into(), theme.fg_muted),
                ],
            )
        } else {
            super::hint_footer(
                theme,
                [
                    (
                        format!("{}/{}", keys.nav_up, keys.nav_down),
                        "滚动".into(),
                        theme.fg_accent,
                    ),
                    (
                        keys.nav_next_page.clone(),
                        if self.comments_focused {
                            "看正文"
                        } else {
                            "看评论"
                        }
                        .into(),
                        theme.info,
                    ),
                    (keys.comment.clone(), "评论".into(), theme.info),
                    (keys.back.clone(), "返回".into(), theme.info),
                ],
            )
        };
        frame.render_widget(
            Paragraph::new(footer).alignment(Alignment::Center),
            chunks[3],
        );
    }

    fn handle_input_with_modifiers(
        &mut self,
        key: KeyCode,
        modifiers: crossterm::event::KeyModifiers,
        keys: &Keybindings,
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

    fn handle_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        self.notice = None;
        if self.input_mode {
            match key {
                // The draft stays for the next time the box opens.
                _ if super::text::is_input_cancel(key, keys) => self.input_mode = false,
                KeyCode::Enter => {
                    let message = self.input_buffer.trim().to_string();
                    if !message.is_empty()
                        && let Some(target) = self.comment_target()
                    {
                        self.input_mode = false;
                        self.input_buffer.clear();
                        self.input_cursor = 0;
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
                }
                _ => {
                    super::text::edit_at_cursor(
                        &mut self.input_buffer,
                        &mut self.input_cursor,
                        key,
                        crossterm::event::KeyModifiers::NONE,
                    );
                }
            }
            return Some(AppAction::None);
        }
        if keys.matches_nav_next(key) || keys.matches_nav_prev(key) {
            self.comments_focused = !self.comments_focused;
            return Some(AppAction::None);
        }
        if self.comments_focused {
            if keys.matches_back(key) && self.comments.close_thread() {
                return Some(AppAction::None);
            }
            if keys.matches_comment_sort(key) && self.comment_target().is_some() {
                let request = self.comments.begin_sort_toggle();
                return Some(self.comment_action(request));
            }
            if let Some(handled) = self.comments.handle_key(key, keys) {
                return Some(
                    handled.map_or(AppAction::None, |request| self.comment_action(request)),
                );
            }
        }
        // `q` leaves a detail page like Esc, as on the other detail pages.
        if keys.matches_back(key) || keys.matches_quit(key) {
            return Some(AppAction::BackToList);
        }
        if keys.matches_comment_sort(key) && self.comment_target().is_some() {
            let request = self.comments.begin_sort_toggle();
            return Some(self.comment_action(request));
        }
        if keys.matches_comment(key) && self.comment_target().is_some() {
            return Some(self.comment_action(PanelRequest::Compose {
                root: None,
                parent: None,
                to: None,
            }));
        }
        if keys.matches_up(key) {
            self.scroll = self.scroll.saturating_sub(1);
        } else if keys.matches_down(key) {
            self.scroll = self.scroll.saturating_add(1);
        } else if keys.matches_page_up(key) {
            self.scroll = self.scroll.saturating_sub(self.visible_height);
        } else if keys.matches_page_down(key) {
            self.scroll = self.scroll.saturating_add(self.visible_height);
        } else if keys.matches_jump_top(key) {
            self.scroll = 0;
        } else if keys.matches_jump_bottom(key) {
            // Drawing clamps this to the last screen.
            self.scroll = u16::MAX;
        }
        None
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> Option<AppAction> {
        if self.comments_focused {
            return self
                .comments
                .handle_mouse(event)
                .map(|request| self.comment_action(request));
        }
        match event.kind {
            MouseEventKind::ScrollUp => self.scroll = self.scroll.saturating_sub(1),
            MouseEventKind::ScrollDown => self.scroll = self.scroll.saturating_add(1),
            _ => {}
        }
        None
    }
}

async fn download_image(url: &str) -> Option<DynamicImage> {
    let response = reqwest::get(url).await.ok()?;
    let bytes = response.bytes().await.ok()?;
    image::load_from_memory(&bytes).ok()
}

fn article_block_height(block: &ArticleBlock, width: u16) -> u16 {
    match block {
        ArticleBlock::Text(text) => {
            let width = width.max(1) as usize;
            text.lines()
                .map(|line| Line::from(line).width().max(1).div_ceil(width) as u16)
                .sum::<u16>()
                .saturating_add(1)
        }
        ArticleBlock::Image { .. } => 14,
        ArticleBlock::Embedded(_) => 2,
    }
}
