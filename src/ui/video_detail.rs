//! Video detail page showing video info, comments, and related videos

use super::video_card::VideoCardGrid;
use super::{Component, Theme, footer_notice, shortcut_footer};
use crate::api::client::{ArchiveRelation, TripleResult};
use crate::api::comment::{CommentBatch, CommentItem, CommentSort};
use crate::api::video::{RelatedVideoItem, VideoInfo};
use crate::application::{AppAction, VideoActionRequest};
use crate::storage::Keybindings;
use ratatui::{
    crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use std::collections::HashSet;
use std::time::Instant;

/// What the text box at the bottom is typing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputTarget {
    #[default]
    Comment,
    Danmaku,
}

#[derive(Clone, Copy, PartialEq)]
pub enum DetailFocus {
    Comments,
    Episodes,
    Related,
}

pub struct VideoDetailPage {
    pub bvid: String,
    pub aid: i64,
    pub video_info: Option<VideoInfo>,
    pub comments: Vec<CommentItem>,
    pub related_videos: Vec<RelatedVideoItem>,
    pub recommendations_enabled: bool,
    pub related_card_grid: VideoCardGrid,
    pub loading: bool,
    pub error_message: Option<String>,
    /// Cursor of the next comment page; `None` once all are loaded.
    pub comment_offset: Option<String>,
    pub comment_sort: CommentSort,
    pub comment_scroll: usize,
    pub related_scroll: usize,
    pub focus: DetailFocus,
    pub has_more_comments: bool,
    pub loading_more_comments: bool,
    pub expanded_comment: Option<i64>,
    pub comment_replies: Vec<CommentItem>,
    pub loading_replies: bool,
    pub liked_comments: HashSet<i64>,
    pub input_mode: bool,
    pub input_target: InputTarget,
    pub input_buffer: String,
    /// The account's like/coin/favourite/follow state; `None` for guests.
    pub relation: Option<ArchiveRelation>,
    /// A video action was sent and its result has not arrived.
    pub action_pending: bool,
    /// Transient message shown in the footer until the next key press.
    pub notice: Option<String>,
    last_click_time: Option<Instant>,
    last_click_index: Option<usize>,
    /// Current episode index for multi-part videos (0-based)
    pub current_page_index: usize,
    /// Scroll position in episode list
    pub episode_scroll: usize,
    pub auto_play_pending: bool,
}

impl VideoDetailPage {
    pub fn new(bvid: String, aid: i64) -> Self {
        let mut related_card_grid = VideoCardGrid::new();
        related_card_grid.columns = 2;
        related_card_grid.card_height = 8;

        Self {
            bvid,
            aid,
            video_info: None,
            comments: Vec::new(),
            related_videos: Vec::new(),
            recommendations_enabled: true,
            related_card_grid,
            loading: true,
            error_message: None,
            comment_offset: None,
            comment_sort: CommentSort::default(),
            comment_scroll: 0,
            related_scroll: 0,
            focus: DetailFocus::Comments,
            has_more_comments: true,
            loading_more_comments: false,
            expanded_comment: None,
            comment_replies: Vec::new(),
            loading_replies: false,
            liked_comments: HashSet::new(),
            input_mode: false,
            input_target: InputTarget::Comment,
            input_buffer: String::new(),
            relation: None,
            action_pending: false,
            notice: None,
            last_click_time: None,
            last_click_index: None,
            current_page_index: 0,
            episode_scroll: 0,
            auto_play_pending: true,
        }
    }

    /// Reopen the comment box with a draft that could not be posted.
    pub fn restore_comment_draft(&mut self, message: String, notice: String) {
        self.input_mode = true;
        self.input_target = InputTarget::Comment;
        self.input_buffer = message;
        self.notice = Some(notice);
    }

    /// Reopen the danmaku box with a message that could not be sent.
    pub fn restore_danmaku_draft(&mut self, message: String, notice: String) {
        self.input_mode = true;
        self.input_target = InputTarget::Danmaku;
        self.input_buffer = message;
        self.notice = Some(notice);
    }

    /// The part playing or about to play.
    fn current_cid(&self) -> Option<i64> {
        self.get_pages()
            .and_then(|pages| pages.get(self.current_page_index))
            .map(|page| page.cid)
            .or_else(|| self.video_info.as_ref().map(|info| info.cid))
            .filter(|cid| *cid > 0)
    }

    /// The request for a video action key, or `None` for other keys.
    fn video_action_for(&self, key: KeyCode, keys: &Keybindings) -> Option<VideoActionRequest> {
        let info = self.video_info.as_ref()?;
        let aid = info.aid;
        let relation = self.relation.unwrap_or_default();
        Some(if keys.matches_video_like(key) {
            VideoActionRequest::Like {
                aid,
                like: !relation.like,
            }
        } else if keys.matches_video_coin(key) {
            VideoActionRequest::Coin { aid }
        } else if keys.matches_video_favorite(key) {
            VideoActionRequest::Favorite {
                aid,
                favorite: !relation.favorite,
            }
        } else if keys.matches_video_triple(key) {
            VideoActionRequest::Triple { aid }
        } else if keys.matches_watch_later(key) {
            VideoActionRequest::WatchLater { aid }
        } else if keys.matches_follow(key) {
            VideoActionRequest::Follow {
                aid,
                mid: info.owner.mid,
                follow: !relation.attention,
            }
        } else {
            return None;
        })
    }

    /// Record the server's answer to a video action.
    pub fn apply_video_action(
        &mut self,
        request: VideoActionRequest,
        result: Result<Option<TripleResult>, String>,
    ) {
        self.action_pending = false;
        if let Err(error) = result {
            if let VideoActionRequest::SendDanmaku { message, .. } = request {
                self.restore_danmaku_draft(message, format!("弹幕发送失败：{error}"));
            } else {
                self.notice = Some(format!("操作失败：{error}"));
            }
            return;
        }
        let relation = self.relation.get_or_insert_with(ArchiveRelation::default);
        self.notice = Some(match request {
            VideoActionRequest::Like { like, .. } => {
                relation.like = like;
                if like { "已点赞" } else { "已取消点赞" }.to_string()
            }
            VideoActionRequest::Coin { .. } => {
                relation.coin = (relation.coin + 1).min(2);
                "已投币".to_string()
            }
            VideoActionRequest::Favorite { favorite, .. } => {
                relation.favorite = favorite;
                if favorite {
                    "已加入默认收藏夹"
                } else {
                    "已取消收藏"
                }
                .to_string()
            }
            VideoActionRequest::Triple { .. } => {
                let triple = result.ok().flatten().unwrap_or_default();
                relation.like |= triple.like;
                relation.favorite |= triple.fav;
                if triple.coin {
                    relation.coin = triple.multiply.max(relation.coin).min(2);
                }
                format!(
                    "三连：点赞{} 投币{} 收藏{}",
                    if triple.like { "✓" } else { "✗" },
                    if triple.coin { "✓" } else { "✗" },
                    if triple.fav { "✓" } else { "✗" }
                )
            }
            VideoActionRequest::WatchLater { .. } => "已添加到稍后再看".to_string(),
            VideoActionRequest::Follow { follow, .. } => {
                relation.attention = follow;
                if follow {
                    "已关注UP主"
                } else {
                    "已取消关注"
                }
                .to_string()
            }
            VideoActionRequest::SendDanmaku { progress_ms, .. } => format!(
                "弹幕已发送到 {:02}:{:02}",
                progress_ms / 60_000,
                progress_ms / 1000 % 60
            ),
        });
    }

    pub fn play_action(&self) -> AppAction {
        if let Some(pages) = self.get_pages() {
            if pages.len() > 1 {
                return AppAction::PlayVideoWithPages {
                    bvid: self.bvid.clone(),
                    aid: self.aid,
                    pages: pages.clone(),
                    current_index: self.current_page_index,
                };
            }
            if let Some(page) = pages.first() {
                return AppAction::PlayVideo {
                    bvid: self.bvid.clone(),
                    aid: self.aid,
                    cid: page.cid,
                    duration: page.duration,
                };
            }
        }
        let (cid, duration) = self
            .video_info
            .as_ref()
            .map_or((0, 0), |info| (info.cid, info.duration.unwrap_or(0)));
        AppAction::PlayVideo {
            bvid: self.bvid.clone(),
            aid: self.aid,
            cid,
            duration,
        }
    }

    /// Marks the next comment page as loading and returns its sort and
    /// cursor, or `None` when there is nothing more or a page is in flight.
    pub fn begin_load_more_comments(&mut self) -> Option<(CommentSort, String)> {
        if !self.has_more_comments || self.loading_more_comments {
            return None;
        }
        let offset = self.comment_offset.clone()?;
        self.loading_more_comments = true;
        Some((self.comment_sort, offset))
    }

    /// Replace the comment list with the first page of a load.
    pub fn set_comments(&mut self, batch: CommentBatch) {
        self.comments = batch.comments;
        self.has_more_comments = batch.next_offset.is_some();
        self.comment_offset = batch.next_offset;
        self.comment_scroll = 0;
        self.expanded_comment = None;
        self.comment_replies.clear();
        self.loading_replies = false;
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

    /// Switch between hot and newest-first comments and return the sort to
    /// load, or `None` while the page itself is still loading.
    pub fn begin_comment_sort_toggle(&mut self) -> Option<CommentSort> {
        if self.loading || self.video_info.is_none() {
            return None;
        }
        self.comment_sort = self.comment_sort.toggled();
        self.set_comments(CommentBatch::default());
        self.loading_more_comments = true;
        Some(self.comment_sort)
    }

    pub fn apply_more_comments_error(&mut self) {
        self.loading_more_comments = false;
    }

    /// Collapses the selected comment's replies when they are shown, or
    /// expands them and returns the comment to load replies for.
    pub fn toggle_comment_replies(&mut self) -> Option<i64> {
        let comment = self.comments.get(self.comment_scroll)?;
        let comment_rpid = comment.rpid;

        if self.expanded_comment == Some(comment_rpid) {
            self.expanded_comment = None;
            self.comment_replies.clear();
            self.loading_replies = false;
            return None;
        }
        if comment.reply_count() == 0 {
            return None;
        }

        self.expanded_comment = Some(comment_rpid);
        self.comment_replies.clear();
        self.loading_replies = true;
        Some(comment_rpid)
    }

    /// Replies for a comment that is no longer expanded are ignored. A failed
    /// load (`None`) leaves the comment expanded with no replies.
    pub fn apply_comment_replies(&mut self, root: i64, replies: Option<Vec<CommentItem>>) {
        if self.expanded_comment != Some(root) {
            return;
        }
        self.comment_replies = replies.unwrap_or_default();
        self.loading_replies = false;
    }

    /// Poll for completed related video cover downloads
    pub fn poll_cover_results(&mut self) {
        self.related_card_grid.poll_cover_results();
    }

    /// Start background downloads for visible related video covers
    pub fn start_cover_downloads(&mut self) {
        self.related_card_grid.start_cover_downloads();
    }

    /// Check if scrolling near bottom of comments
    fn is_near_comments_bottom(&self, visible_count: usize) -> bool {
        if self.comments.is_empty() {
            return false;
        }
        self.comment_scroll + visible_count >= self.comments.len().saturating_sub(2)
    }

    fn render_video_info(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle))
            .title(Span::styled(
                " 📹 视频信息 ",
                Style::default().fg(theme.bilibili_pink),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if let Some(ref info) = self.video_info {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Title
                    Constraint::Length(1), // Author
                    Constraint::Length(1), // Stats
                    Constraint::Min(1),    // Description
                ])
                .split(inner);

            // Title
            let title = Paragraph::new(info.title.clone()).style(
                Style::default()
                    .fg(theme.fg_primary)
                    .add_modifier(Modifier::BOLD),
            );
            frame.render_widget(title, chunks[0]);

            // Author
            let relation = self.relation.unwrap_or_default();
            let mut author_spans = vec![Span::styled(
                format!("UP: {}", info.owner.name),
                Style::default().fg(theme.bilibili_pink),
            )];
            if self.relation.is_some() {
                author_spans.push(Span::styled(
                    if relation.attention {
                        "  [已关注]"
                    } else {
                        "  [未关注]"
                    },
                    Style::default().fg(theme.fg_secondary),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(author_spans)), chunks[1]);
            let marked = |active: bool| {
                if active {
                    Style::default()
                        .fg(theme.bilibili_pink)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg_secondary)
                }
            };

            // Stats
            let stats = Paragraph::new(Line::from(vec![
                Span::styled("▶ ", Style::default().fg(theme.fg_secondary)),
                Span::styled(
                    info.stat.format_views(),
                    Style::default().fg(theme.fg_secondary),
                ),
                Span::styled(" · 💬 ", Style::default().fg(theme.fg_secondary)),
                Span::styled(
                    info.stat.format_danmaku(),
                    Style::default().fg(theme.fg_secondary),
                ),
                Span::styled(" · 👍 ", Style::default().fg(theme.fg_secondary)),
                Span::styled(info.stat.format_like(), marked(relation.like)),
                Span::styled(" · 💰 ", Style::default().fg(theme.fg_secondary)),
                Span::styled(
                    if relation.coin > 0 {
                        format!("{} (已投{})", info.stat.format_coin(), relation.coin)
                    } else {
                        info.stat.format_coin()
                    },
                    marked(relation.coin > 0),
                ),
                Span::styled(" · ⭐ ", Style::default().fg(theme.fg_secondary)),
                Span::styled(info.stat.format_favorite(), marked(relation.favorite)),
            ]));
            frame.render_widget(stats, chunks[2]);

            // Description
            if let Some(desc) = &info.desc {
                let char_count = desc.chars().count();
                let desc_text: String = if char_count > 100 {
                    desc.chars().take(100).collect::<String>() + "..."
                } else {
                    desc.clone()
                };
                let description = Paragraph::new(desc_text)
                    .style(Style::default().fg(theme.fg_secondary))
                    .wrap(Wrap { trim: true });
                frame.render_widget(description, chunks[3]);
            }
        } else {
            let loading = Paragraph::new("加载中...")
                .style(Style::default().fg(theme.warning))
                .alignment(Alignment::Center);
            frame.render_widget(loading, inner);
        }
    }

    fn render_comments(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let is_focused = self.focus == DetailFocus::Comments;
        let border_style = if is_focused {
            Style::default().fg(theme.border_focused)
        } else {
            Style::default().fg(theme.border_unfocused)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled(
                format!(" 💬 评论 · {} ", self.comment_sort.label()),
                Style::default().fg(if is_focused {
                    theme.bilibili_pink
                } else {
                    theme.fg_muted
                }),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.comments.is_empty() {
            let text = if self.loading_more_comments {
                "⏳ 加载评论中..."
            } else {
                "暂无评论"
            };
            let empty = Paragraph::new(text)
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center);
            frame.render_widget(empty, inner);
            return;
        }

        // Build all items with replies
        let mut all_items = Vec::new();
        let item_height = 3;

        for (idx, comment) in self.comments.iter().enumerate() {
            let is_selected = idx == self.comment_scroll;
            let is_expanded = self.expanded_comment == Some(comment.rpid);

            // Main comment
            let reply_indicator = if comment.reply_count() > 0 {
                if is_expanded { "▼" } else { "▶" }
            } else {
                " "
            };

            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        format!("{} ", reply_indicator),
                        Style::default().fg(theme.fg_accent),
                    ),
                    Span::styled(
                        comment.author_name(),
                        Style::default()
                            .fg(theme.bilibili_pink)
                            .add_modifier(if is_selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    Span::styled(
                        format!("  {}", comment.format_time()),
                        Style::default().fg(theme.fg_secondary),
                    ),
                ]),
                Line::from(vec![Span::styled(
                    truncate_str(comment.message(), 60),
                    Style::default().fg(theme.fg_primary),
                )]),
                Line::from(vec![Span::styled(
                    format!(
                        "👍 {}  💬 {} 回复",
                        comment.format_like(),
                        comment.reply_count()
                    ),
                    Style::default().fg(theme.fg_secondary),
                )]),
            ];
            all_items.push(ListItem::new(lines));

            // Show replies if expanded
            if is_expanded {
                if self.loading_replies {
                    all_items.push(ListItem::new(vec![Line::from(vec![Span::styled(
                        "  ⏳ 加载回复中...",
                        Style::default().fg(theme.warning),
                    )])]));
                } else {
                    for reply in &self.comment_replies {
                        let reply_lines = vec![
                            Line::from(vec![
                                Span::styled("    ↳ ", Style::default().fg(theme.fg_secondary)),
                                Span::styled(
                                    reply.author_name(),
                                    Style::default().fg(Color::Rgb(150, 150, 200)),
                                ),
                                Span::styled(
                                    format!("  {}", reply.format_time()),
                                    Style::default().fg(theme.fg_secondary),
                                ),
                            ]),
                            Line::from(vec![
                                Span::styled("      ", Style::default()),
                                Span::styled(
                                    truncate_str(reply.message(), 55),
                                    Style::default().fg(theme.fg_primary),
                                ),
                            ]),
                            Line::from(vec![
                                Span::styled("      ", Style::default()),
                                Span::styled(
                                    format!("👍 {}", reply.format_like()),
                                    Style::default().fg(theme.fg_secondary),
                                ),
                            ]),
                        ];
                        all_items.push(ListItem::new(reply_lines));
                    }
                }
            }
        }

        // Start the window at the selected comment. Replies are list items
        // too, so the comment index alone would scroll past it.
        let visible_count = (inner.height as usize / item_height).max(1);
        let display_items: Vec<ListItem> = all_items
            .into_iter()
            .skip(self.selected_comment_item())
            .take(visible_count)
            .collect();

        let list = List::new(display_items);
        frame.render_widget(list, inner);
    }

    /// Index of the selected comment among the rendered list items, which
    /// include the replies (or loading row) of an expanded comment above it.
    fn selected_comment_item(&self) -> usize {
        let expanded_above = self.expanded_comment.is_some_and(|rpid| {
            self.comments
                .iter()
                .take(self.comment_scroll)
                .any(|comment| comment.rpid == rpid)
        });
        let reply_items = if !expanded_above {
            0
        } else if self.loading_replies {
            1
        } else {
            self.comment_replies.len()
        };
        self.comment_scroll + reply_items
    }

    fn render_related(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let is_focused = self.focus == DetailFocus::Related;
        let border_style = if is_focused {
            Style::default().fg(theme.border_focused)
        } else {
            Style::default().fg(theme.border_unfocused)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled(
                " 📺 相关推荐 ",
                Style::default().fg(if is_focused {
                    theme.bilibili_pink
                } else {
                    theme.fg_muted
                }),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.related_card_grid.cards.is_empty() {
            let empty = Paragraph::new("暂无相关视频")
                .style(Style::default().fg(theme.fg_secondary))
                .alignment(Alignment::Center);
            frame.render_widget(empty, inner);
            return;
        }

        // Sync scroll position with grid
        self.related_card_grid.selected_index = self.related_scroll;

        // Render the video card grid
        self.related_card_grid.render(frame, inner, theme);
    }

    /// Check if video has multiple parts
    fn has_multiple_pages(&self) -> bool {
        self.video_info
            .as_ref()
            .and_then(|info| info.pages.as_ref())
            .map(|pages| pages.len() > 1)
            .unwrap_or(false)
    }

    /// Get the video pages
    fn get_pages(&self) -> Option<&Vec<crate::api::video::VideoPage>> {
        self.video_info
            .as_ref()
            .and_then(|info| info.pages.as_ref())
    }

    fn render_episodes(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let is_focused = self.focus == DetailFocus::Episodes;
        let border_style = if is_focused {
            Style::default().fg(theme.border_focused)
        } else {
            Style::default().fg(theme.border_unfocused)
        };

        let pages = match self.get_pages() {
            Some(p) => p,
            None => return,
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled(
                format!(" 📑 选集 ({}) ", pages.len()),
                Style::default().fg(if is_focused {
                    theme.bilibili_pink
                } else {
                    theme.fg_muted
                }),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let visible_count = inner.height as usize;
        let scroll_offset = if self.episode_scroll >= visible_count {
            self.episode_scroll - visible_count + 1
        } else {
            0
        };

        let items: Vec<ListItem> = pages
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_count)
            .map(|(idx, page)| {
                let is_current = idx == self.current_page_index;
                let is_selected = idx == self.episode_scroll;

                // Format duration as mm:ss
                let duration = {
                    let mins = page.duration / 60;
                    let secs = page.duration % 60;
                    format!("{:02}:{:02}", mins, secs)
                };

                let prefix = if is_current { "▶ " } else { "  " };
                let title = truncate_str(&page.part, 30);

                let style = if is_selected && is_focused {
                    Style::default()
                        .fg(theme.bilibili_pink)
                        .add_modifier(Modifier::BOLD)
                } else if is_current {
                    Style::default().fg(theme.fg_accent)
                } else {
                    Style::default().fg(theme.fg_primary)
                };

                ListItem::new(Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(
                        format!("P{} ", page.page),
                        Style::default().fg(theme.fg_secondary),
                    ),
                    Span::styled(title, style),
                    Span::styled(
                        format!("  {}", duration),
                        Style::default().fg(theme.fg_muted),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items);
        frame.render_widget(list, inner);
    }
}

impl Component for VideoDetailPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        // Adjust layout based on input mode
        let chunks = if self.input_mode {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6), // Video info
                    Constraint::Min(8),    // Comments + Related
                    Constraint::Length(3), // Input box
                    Constraint::Length(2), // Help
                ])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6), // Video info
                    Constraint::Min(10),   // Comments + Related
                    Constraint::Length(2), // Help
                ])
                .split(area)
        };

        // Video info
        self.render_video_info(frame, chunks[0], theme);

        if self.loading {
            let loading = Paragraph::new("⏳ 加载中...")
                .style(Style::default().fg(theme.warning))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded),
                );
            frame.render_widget(loading, chunks[1]);
        } else if let Some(error) = &self.error_message {
            let error_widget = Paragraph::new(format!("❌ {}", error))
                .style(Style::default().fg(theme.error))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded),
                );
            frame.render_widget(error_widget, chunks[1]);
        } else if !self.recommendations_enabled {
            if self.has_multiple_pages() {
                let content =
                    Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                        .split(chunks[1]);
                self.render_comments(frame, content[0], theme);
                self.render_episodes(frame, content[1], theme);
            } else {
                self.render_comments(frame, chunks[1], theme);
            }
        } else {
            // Comments and Related split
            let content_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(60), // Comments
                    Constraint::Percentage(40), // Related + Episodes
                ])
                .split(chunks[1]);

            self.render_comments(frame, content_chunks[0], theme);

            // Right panel: Episodes (if multi-part) + Related videos
            if self.has_multiple_pages() {
                let right_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(50), // Episodes
                        Constraint::Percentage(50), // Related
                    ])
                    .split(content_chunks[1]);

                self.render_episodes(frame, right_chunks[0], theme);
                self.render_related(frame, right_chunks[1], theme);
            } else {
                self.render_related(frame, content_chunks[1], theme);
            }
        }

        // Input box (only in input mode)
        if self.input_mode {
            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.bilibili_pink))
                .title(Span::styled(
                    match self.input_target {
                        InputTarget::Comment => " ✏️ 发表评论 ",
                        InputTarget::Danmaku => " ✏️ 发送弹幕 ",
                    },
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
                    (
                        "Enter".to_string(),
                        match self.input_target {
                            InputTarget::Comment => "发送评论",
                            InputTarget::Danmaku => "发送弹幕",
                        }
                        .into(),
                        theme.success,
                    ),
                    (keys.back.clone(), "取消".into(), theme.info),
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
                    (keys.nav_next_page.clone(), "切换".into(), theme.info),
                    (keys.confirm.clone(), "点赞/选择".into(), theme.success),
                    (keys.comment.clone(), "评论".into(), theme.info),
                    (keys.toggle_replies.clone(), "回复".into(), theme.info),
                    (
                        keys.comment_sort.clone(),
                        self.comment_sort.toggled().label().into(),
                        theme.info,
                    ),
                    (keys.play.clone(), "播放".into(), theme.success),
                    (keys.back.clone(), "返回".into(), theme.info),
                ],
            )
        };
        let actions = shortcut_footer(
            theme,
            [
                (keys.video_like.clone(), "赞".into(), theme.bilibili_pink),
                (keys.video_coin.clone(), "币".into(), theme.bilibili_pink),
                (
                    keys.video_favorite.clone(),
                    "藏".into(),
                    theme.bilibili_pink,
                ),
                (
                    keys.video_triple.clone(),
                    "三连".into(),
                    theme.bilibili_pink,
                ),
                (keys.watch_later.clone(), "稍后看".into(), theme.info),
                (keys.follow.clone(), "关注".into(), theme.info),
                (keys.send_danmaku.clone(), "弹幕".into(), theme.info),
                (keys.open_up.clone(), "UP主".into(), theme.info),
                (keys.stop_playback.clone(), "停止播放".into(), theme.warning),
            ],
        );
        let lines = if self.input_mode || self.notice.is_some() {
            vec![help]
        } else {
            vec![help, actions]
        };
        let help = Paragraph::new(lines).alignment(Alignment::Center);
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
                KeyCode::Enter if self.input_target == InputTarget::Danmaku => {
                    let message = self.input_buffer.trim().to_string();
                    if let (false, Some(info), Some(cid)) =
                        (message.is_empty(), &self.video_info, self.current_cid())
                    {
                        self.input_buffer.clear();
                        self.input_mode = false;
                        return Some(AppAction::VideoAction(VideoActionRequest::SendDanmaku {
                            aid: info.aid,
                            cid,
                            message,
                            progress_ms: 0,
                        }));
                    }
                    return Some(AppAction::None);
                }
                KeyCode::Enter => {
                    if !self.input_buffer.is_empty() {
                        let message = self.input_buffer.clone();
                        self.input_buffer.clear();
                        self.input_mode = false;
                        return Some(AppAction::AddComment {
                            oid: self.aid,
                            comment_type: 1, // Video comment type
                            message,
                            root: None,
                        });
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
        if keys.matches_open_up(key)
            && let Some(info) = &self.video_info
        {
            return Some(AppAction::OpenUpPage(info.owner.mid));
        }
        if let Some(request) = self.video_action_for(key, keys) {
            if self.action_pending {
                return Some(AppAction::None);
            }
            return Some(AppAction::VideoAction(request));
        }
        if keys.matches_send_danmaku(key) && self.video_info.is_some() {
            self.input_mode = true;
            self.input_target = InputTarget::Danmaku;
            self.input_buffer.clear();
            return Some(AppAction::None);
        }
        if keys.matches_play(key) {
            return Some(self.play_action());
        }
        if keys.matches_comment(key) {
            // Enter comment input mode
            self.input_mode = true;
            self.input_target = InputTarget::Comment;
            self.input_buffer.clear();
            return Some(AppAction::None);
        }
        if keys.matches_toggle_replies(key) {
            if self.focus == DetailFocus::Comments {
                return Some(AppAction::ToggleCommentReplies);
            }
            return Some(AppAction::None);
        }
        if keys.matches_comment_sort(key) {
            return Some(AppAction::ToggleCommentSort);
        }
        // Tab switches focus between Comments, Episodes, and Related (page-specific, not nav)
        if key == KeyCode::Tab {
            self.focus = if !self.recommendations_enabled {
                if self.has_multiple_pages() && self.focus == DetailFocus::Comments {
                    DetailFocus::Episodes
                } else {
                    DetailFocus::Comments
                }
            } else if self.has_multiple_pages() {
                match self.focus {
                    DetailFocus::Comments => DetailFocus::Episodes,
                    DetailFocus::Episodes => DetailFocus::Related,
                    DetailFocus::Related => DetailFocus::Comments,
                }
            } else {
                match self.focus {
                    DetailFocus::Comments => DetailFocus::Related,
                    DetailFocus::Episodes => DetailFocus::Related,
                    DetailFocus::Related => DetailFocus::Comments,
                }
            };
            return Some(AppAction::None);
        }
        if keys.matches_down(key) {
            match self.focus {
                DetailFocus::Comments => {
                    if self.comment_scroll + 1 < self.comments.len() {
                        self.comment_scroll += 1;
                    }
                    // Check if near bottom to load more comments
                    if self.is_near_comments_bottom(10)
                        && self.has_more_comments
                        && !self.loading_more_comments
                    {
                        return Some(AppAction::LoadMoreComments);
                    }
                }
                DetailFocus::Episodes => {
                    if let Some(pages) = self.get_pages()
                        && self.episode_scroll + 1 < pages.len()
                    {
                        self.episode_scroll += 1;
                    }
                }
                DetailFocus::Related => {
                    if self.related_card_grid.move_down() {
                        self.related_scroll = self.related_card_grid.selected_index;
                    }
                }
            }
            return Some(AppAction::None);
        }
        if keys.matches_up(key) {
            match self.focus {
                DetailFocus::Comments => {
                    if self.comment_scroll > 0 {
                        self.comment_scroll -= 1;
                    }
                }
                DetailFocus::Episodes => {
                    if self.episode_scroll > 0 {
                        self.episode_scroll -= 1;
                    }
                }
                DetailFocus::Related => {
                    if self.related_card_grid.move_up() {
                        self.related_scroll = self.related_card_grid.selected_index;
                    }
                }
            }
            return Some(AppAction::None);
        }
        if keys.matches_left(key) {
            if self.focus == DetailFocus::Related && self.related_card_grid.move_left() {
                self.related_scroll = self.related_card_grid.selected_index;
            }
            return Some(AppAction::None);
        }
        if keys.matches_right(key) {
            if self.focus == DetailFocus::Related && self.related_card_grid.move_right() {
                self.related_scroll = self.related_card_grid.selected_index;
            }
            return Some(AppAction::None);
        }
        if keys.matches_confirm(key) {
            match self.focus {
                DetailFocus::Comments => {
                    // Like the currently selected comment
                    if self.comment_scroll < self.comments.len() {
                        let comment = &self.comments[self.comment_scroll];
                        return Some(AppAction::LikeComment {
                            oid: self.aid,
                            rpid: comment.rpid,
                            comment_type: 1,
                        });
                    }
                }
                DetailFocus::Episodes => {
                    // Select and play the episode with auto-advance
                    if let Some(pages) = self.get_pages().cloned()
                        && self.episode_scroll < pages.len()
                    {
                        self.current_page_index = self.episode_scroll;
                        return Some(AppAction::PlayVideoWithPages {
                            bvid: self.bvid.clone(),
                            aid: self.aid,
                            pages,
                            current_index: self.episode_scroll,
                        });
                    }
                }
                DetailFocus::Related => {
                    if let Some(card) = self.related_card_grid.selected_card()
                        && let Some(bvid) = &card.bvid
                    {
                        let aid = card.aid.unwrap_or(0);
                        return Some(AppAction::OpenVideoDetail(bvid.clone(), aid));
                    }
                }
            }
            return Some(AppAction::None);
        }
        Some(AppAction::None)
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Option<AppAction> {
        if self.input_mode {
            return None;
        }

        match event.kind {
            MouseEventKind::ScrollDown => {
                match self.focus {
                    DetailFocus::Comments => {
                        if self.comment_scroll + 1 < self.comments.len() {
                            self.comment_scroll += 1;
                            if self.is_near_comments_bottom(10)
                                && self.has_more_comments
                                && !self.loading_more_comments
                            {
                                return Some(AppAction::LoadMoreComments);
                            }
                        }
                    }
                    DetailFocus::Related => {
                        if self.related_card_grid.move_down() {
                            self.related_scroll = self.related_card_grid.selected_index;
                        }
                    }
                    DetailFocus::Episodes => {
                        if let Some(pages) = self.get_pages()
                            && self.episode_scroll + 1 < pages.len()
                        {
                            self.episode_scroll += 1;
                        }
                    }
                }
                None
            }
            MouseEventKind::ScrollUp => {
                match self.focus {
                    DetailFocus::Comments => {
                        if self.comment_scroll > 0 {
                            self.comment_scroll -= 1;
                        }
                    }
                    DetailFocus::Related => {
                        if self.related_card_grid.move_up() {
                            self.related_scroll = self.related_card_grid.selected_index;
                        }
                    }
                    DetailFocus::Episodes => {
                        if self.episode_scroll > 0 {
                            self.episode_scroll -= 1;
                        }
                    }
                }
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.focus != DetailFocus::Related {
                    return None;
                }

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(6),
                        Constraint::Min(10),
                        Constraint::Length(2),
                    ])
                    .split(area);

                if self.loading {
                    return None;
                }

                if let Some(error) = &self.error_message
                    && (error.contains("视频信息") || error.contains("加载视频"))
                {
                    return None;
                }

                let content_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .split(chunks[1]);

                if !self.recommendations_enabled {
                    return None;
                }
                let related_area = content_chunks[1];

                if !related_area.contains(ratatui::layout::Position::new(event.column, event.row)) {
                    return None;
                }

                let click_idx = self.related_card_grid.index_at(event.column, event.row)?;

                if click_idx < self.related_card_grid.cards.len() {
                    let now = Instant::now();
                    let is_double_click = self.last_click_index == Some(click_idx)
                        && self
                            .last_click_time
                            .is_some_and(|t| now.duration_since(t).as_millis() < 500);

                    if is_double_click {
                        self.last_click_time = None;
                        self.last_click_index = None;
                        if let Some(card) = self.related_card_grid.cards.get(click_idx)
                            && let Some(ref bvid) = card.bvid
                        {
                            let aid = card.aid.unwrap_or(0);
                            return Some(AppAction::OpenVideoDetail(bvid.clone(), aid));
                        }
                    } else {
                        self.related_card_grid.selected_index = click_idx;
                        self.related_card_grid
                            .update_scroll(self.related_card_grid.cached_visible_rows);
                        self.related_scroll = self.related_card_grid.selected_index;
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

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        s.chars()
            .take(max_len.saturating_sub(3))
            .collect::<String>()
            + "..."
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod comment_loading_tests {
    use super::*;

    fn comment(rpid: i64, rcount: i32) -> CommentItem {
        serde_json::from_value(serde_json::json!({
            "rpid": rpid, "oid": 1, "mid": 1, "parent": 0, "rcount": rcount
        }))
        .unwrap()
    }

    fn batch(ids: &[i64], next: Option<&str>) -> CommentBatch {
        CommentBatch {
            comments: ids.iter().map(|id| comment(*id, 0)).collect(),
            next_offset: next.map(str::to_string),
        }
    }

    #[test]
    fn load_more_comments_runs_one_page_at_a_time_and_ends_on_failure() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.set_comments(batch(&[1], Some("p2")));
        assert_eq!(
            page.begin_load_more_comments(),
            Some((CommentSort::Hot, "p2".to_string()))
        );
        assert_eq!(page.begin_load_more_comments(), None);
        page.apply_more_comments_error();
        assert!(!page.loading_more_comments);

        page.begin_load_more_comments();
        page.apply_comment_batch(CommentSort::Hot, true, batch(&[5], None));
        assert_eq!(page.comments.len(), 2);
        assert!(!page.has_more_comments && !page.loading_more_comments);
        assert_eq!(page.begin_load_more_comments(), None);
    }

    #[test]
    fn sort_toggle_reloads_comments_and_drops_pages_of_the_old_sort() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.loading = false;
        page.video_info = Some(
            serde_json::from_value(serde_json::json!({
                "bvid": "BV1test", "aid": 1, "title": "t", "cid": 1,
                "owner": {"mid": 1, "name": "up", "face": ""},
                "stat": {"view": 0, "danmaku": 0, "like": 0, "coin": 0, "favorite": 0, "share": 0}
            }))
            .unwrap(),
        );
        page.set_comments(batch(&[1, 2], Some("p2")));
        page.comment_scroll = 1;

        assert_eq!(page.begin_comment_sort_toggle(), Some(CommentSort::Time));
        assert!(page.comments.is_empty() && page.loading_more_comments);
        assert_eq!(page.comment_scroll, 0);

        page.apply_comment_batch(CommentSort::Hot, true, batch(&[9], None));
        assert!(
            page.comments.is_empty(),
            "a page of the old sort is ignored"
        );
        page.apply_comment_batch(CommentSort::Time, false, batch(&[3], Some("t2")));
        assert_eq!(page.comments[0].rpid, 3);
        assert_eq!(page.comment_offset.as_deref(), Some("t2"));
        assert!(!page.loading_more_comments);
    }

    #[test]
    fn sort_cannot_change_while_the_page_loads() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        assert_eq!(page.begin_comment_sort_toggle(), None);
        assert_eq!(page.comment_sort, CommentSort::Hot);
    }

    #[test]
    fn comment_box_handles_ctrl_shortcuts_instead_of_typing_them() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.input_mode = true;
        page.input_buffer = "first second".into();
        let keys = Keybindings::default();
        page.handle_input_with_modifiers(
            KeyCode::Char('w'),
            crossterm::event::KeyModifiers::CONTROL,
            &keys,
        );
        assert_eq!(page.input_buffer, "first ");
        page.handle_input_with_modifiers(
            KeyCode::Char('x'),
            crossterm::event::KeyModifiers::NONE,
            &keys,
        );
        assert_eq!(page.input_buffer, "first x");
    }

    #[test]
    fn selected_comment_stays_first_after_replies_expand_above_it() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.comments = vec![comment(7, 2), comment(8, 0), comment(9, 0)];
        assert_eq!(page.toggle_comment_replies(), Some(7));
        page.apply_comment_replies(7, Some(vec![comment(70, 0), comment(71, 0)]));

        assert_eq!(page.selected_comment_item(), 0);
        page.comment_scroll = 2;
        // Items: c7, r70, r71, c8, c9 -> the selected c9 is item 4.
        assert_eq!(page.selected_comment_item(), 4);
    }

    #[test]
    fn replies_for_a_collapsed_comment_are_ignored() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.comments = vec![comment(7, 2)];
        assert_eq!(page.toggle_comment_replies(), Some(7));
        assert!(page.loading_replies);
        assert_eq!(page.toggle_comment_replies(), None);
        assert!(!page.loading_replies);

        page.apply_comment_replies(7, Some(vec![comment(8, 0)]));
        assert!(page.comment_replies.is_empty());
    }

    fn loaded_page() -> VideoDetailPage {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.loading = false;
        page.video_info = Some(
            serde_json::from_value(serde_json::json!({
                "bvid": "BV1test", "aid": 1, "title": "t", "cid": 11,
                "owner": {"mid": 42, "name": "up", "face": ""},
                "stat": {"view": 0, "danmaku": 0, "like": 0, "coin": 0, "favorite": 0, "share": 0}
            }))
            .unwrap(),
        );
        page
    }

    #[test]
    fn action_keys_toggle_from_the_current_relation() {
        let keys = Keybindings::default();
        let mut page = loaded_page();
        page.relation = Some(ArchiveRelation {
            like: true,
            ..ArchiveRelation::default()
        });
        assert!(matches!(
            page.handle_input(KeyCode::Char('a'), &keys),
            Some(AppAction::VideoAction(VideoActionRequest::Like {
                like: false,
                ..
            }))
        ));
        assert!(matches!(
            page.handle_input(KeyCode::Char('F'), &keys),
            Some(AppAction::VideoAction(VideoActionRequest::Follow {
                mid: 42,
                follow: true,
                ..
            }))
        ));
        page.action_pending = true;
        assert!(matches!(
            page.handle_input(KeyCode::Char('b'), &keys),
            Some(AppAction::None)
        ));
    }

    #[test]
    fn action_results_update_relation_and_failures_keep_it() {
        let mut page = loaded_page();
        page.action_pending = true;
        page.apply_video_action(VideoActionRequest::Coin { aid: 1 }, Ok(None));
        assert_eq!(page.relation.unwrap().coin, 1);
        assert!(!page.action_pending);
        page.apply_video_action(
            VideoActionRequest::Favorite {
                aid: 1,
                favorite: true,
            },
            Err("API error (-111): csrf".into()),
        );
        assert!(!page.relation.unwrap().favorite);
        assert!(page.notice.as_deref().unwrap().contains("-111"));
        page.apply_video_action(
            VideoActionRequest::Triple { aid: 1 },
            Ok(Some(TripleResult {
                like: true,
                coin: true,
                fav: true,
                multiply: 2,
            })),
        );
        let relation = page.relation.unwrap();
        assert!(relation.like && relation.favorite && relation.coin == 2);
    }

    #[test]
    fn danmaku_box_sends_to_the_current_part() {
        let keys = Keybindings::default();
        let mut page = loaded_page();
        page.handle_input(KeyCode::Char('d'), &keys);
        assert!(page.input_mode && page.input_target == InputTarget::Danmaku);
        for c in "hi".chars() {
            page.handle_input(KeyCode::Char(c), &keys);
        }
        let action = page.handle_input(KeyCode::Enter, &keys);
        assert!(matches!(
            action,
            Some(AppAction::VideoAction(VideoActionRequest::SendDanmaku { cid: 11, ref message, .. }))
                if message == "hi"
        ));
        page.apply_video_action(
            VideoActionRequest::SendDanmaku {
                aid: 1,
                cid: 11,
                message: "hi".into(),
                progress_ms: 0,
            },
            Err("timeout".into()),
        );
        assert!(page.input_mode && page.input_buffer == "hi");
        assert_eq!(page.input_target, InputTarget::Danmaku);
    }
}

#[cfg(test)]
mod minimal_mode_tests {
    use super::*;

    #[test]
    fn minimal_mode_tab_never_focuses_related_videos() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.recommendations_enabled = false;
        let keys = Keybindings::default();
        for _ in 0..3 {
            page.handle_input(KeyCode::Tab, &keys);
            assert!(matches!(page.focus, DetailFocus::Comments));
        }
    }

    #[test]
    fn minimal_mode_keeps_multipart_episodes_in_tab_cycle() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.recommendations_enabled = false;
        page.video_info = Some(serde_json::from_value(serde_json::json!({
            "bvid": "BV1test", "aid": 1, "cid": 1, "title": "test", "owner": {"mid": 1, "name": "test", "face": ""},
            "stat": {"view": 0, "danmaku": 0, "like": 0, "coin": 0, "favorite": 0, "share": 0},
            "pages": [{"cid": 1, "page": 1, "part": "one", "duration": 10}, {"cid": 2, "page": 2, "part": "two", "duration": 10}]
        })).unwrap());
        let keys = Keybindings::default();
        page.handle_input(KeyCode::Tab, &keys);
        assert!(matches!(page.focus, DetailFocus::Episodes));
        page.handle_input(KeyCode::Tab, &keys);
        assert!(matches!(page.focus, DetailFocus::Comments));
    }

    #[test]
    fn minimal_mode_draw_omits_recommendation_panel() {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        page.loading = false;
        page.recommendations_enabled = false;
        terminal
            .draw(|frame| {
                page.draw(
                    frame,
                    frame.area(),
                    &Theme::default(),
                    &Keybindings::default(),
                )
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!content.contains("相关推荐"));
    }
}
