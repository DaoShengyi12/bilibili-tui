//! Full-page article reader used by history entries.

use super::{Component, Theme, shortcut_footer};
use crate::api::{
    article::{ArticleBlock, ArticleData},
    comment::CommentItem,
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
    comments: Vec<CommentItem>,
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
            comments: Vec::new(),
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

    pub fn set_article(&mut self, article: ArticleData, comments: Vec<CommentItem>) {
        let document = article.document();
        self.blocks = document.blocks;
        self.comments = comments;
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

    pub fn start_image_downloads(&mut self) {
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
            .border_style(Style::default().fg(theme.border_subtle))
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

    fn render_comments(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle))
            .title(format!(" 评论 {} ", self.comments.len()));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.comments.is_empty() {
            frame.render_widget(
                Paragraph::new("暂无评论")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(theme.fg_muted)),
                inner,
            );
            return;
        }

        let mut lines = Vec::new();
        for comment in &self.comments {
            lines.push(Line::from(vec![
                Span::styled(
                    comment.author_name(),
                    Style::default()
                        .fg(theme.fg_accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}  👍{}", comment.format_time(), comment.format_like()),
                    Style::default().fg(theme.fg_muted),
                ),
            ]));
            lines.push(Line::styled(
                comment.message().to_string(),
                Style::default().fg(theme.fg_primary),
            ));
            lines.push(Line::default());
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
    }
}

impl Component for ArticleDetailPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
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

        frame.render_widget(
            Paragraph::new(shortcut_footer(
                theme,
                [
                    (
                        format!(
                            "{}/{}",
                            keys.get_arrow_keys_display(),
                            keys.get_nav_keys_display()
                        ),
                        "滚动".into(),
                        theme.fg_accent,
                    ),
                    (
                        format!("{}/{}", keys.page_up, keys.page_down),
                        "翻页".into(),
                        theme.fg_accent,
                    ),
                    (keys.back.clone(), "返回".into(), theme.info),
                ],
            ))
            .alignment(Alignment::Center),
            chunks[2],
        );
    }

    fn handle_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        if keys.matches_back(key) || key == KeyCode::Esc {
            return Some(AppAction::BackToList);
        }
        if keys.matches_up(key) {
            self.scroll = self.scroll.saturating_sub(1);
        } else if keys.matches_down(key) {
            self.scroll = self.scroll.saturating_add(1);
        } else if keys.matches_page_up(key) {
            self.scroll = self.scroll.saturating_sub(self.visible_height);
        } else if keys.matches_page_down(key) {
            self.scroll = self.scroll.saturating_add(self.visible_height);
        } else if keys.matches_quit(key) {
            return Some(AppAction::Quit);
        }
        None
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> Option<AppAction> {
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
