//! Video detail page showing video info, comments, and related videos

use super::comments::{CommentPanel, CommentTarget, PanelRequest};
use super::video_card::VideoCardGrid;
use super::{Component, Theme, footer_notice, shortcut_footer};
use crate::api::client::{ArchiveRelation, TripleResult};
use crate::api::video::{RelatedVideoItem, VideoInfo};
use crate::application::{AppAction, VideoActionRequest};
use crate::storage::Keybindings;
use ratatui::{
    crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
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
    pub comments: CommentPanel,
    pub related_videos: Vec<RelatedVideoItem>,
    pub recommendations_enabled: bool,
    pub related_card_grid: VideoCardGrid,
    pub loading: bool,
    pub error_message: Option<String>,
    pub related_scroll: usize,
    pub focus: DetailFocus,
    pub input_mode: bool,
    /// The thread and comment a reply goes to, and the name shown.
    reply_to: Option<(i64, i64, String)>,
    pub input_target: InputTarget,
    pub input_buffer: String,
    /// Characters after the cursor in `input_buffer`.
    pub input_cursor: usize,
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
    /// Rows of the episode list on screen in the last frame.
    episode_visible_rows: std::cell::Cell<usize>,
    pub auto_play_pending: bool,
}

impl VideoDetailPage {
    pub fn new(bvid: String, aid: i64) -> Self {
        let related_card_grid = VideoCardGrid::new_list();

        Self {
            bvid,
            aid,
            video_info: None,
            comments: CommentPanel::new(),
            related_videos: Vec::new(),
            recommendations_enabled: true,
            related_card_grid,
            loading: true,
            error_message: None,
            related_scroll: 0,
            focus: DetailFocus::Comments,
            input_mode: false,
            reply_to: None,
            input_target: InputTarget::Comment,
            input_buffer: String::new(),
            input_cursor: 0,
            relation: None,
            action_pending: false,
            notice: None,
            last_click_time: None,
            last_click_index: None,
            current_page_index: 0,
            episode_scroll: 0,
            episode_visible_rows: std::cell::Cell::new(1),
            auto_play_pending: true,
        }
    }

    /// Reopen the comment box with a draft that could not be posted.
    pub fn restore_comment_draft(&mut self, message: String, notice: String) {
        self.input_mode = true;
        self.input_target = InputTarget::Comment;
        self.input_buffer = message;
        self.input_cursor = 0;
        self.notice = Some(notice);
    }

    /// Reopen the danmaku box with a message that could not be sent.
    pub fn restore_danmaku_draft(&mut self, message: String, notice: String) {
        self.input_mode = true;
        self.input_target = InputTarget::Danmaku;
        self.reply_to = None;
        self.input_buffer = message;
        self.input_cursor = 0;
        self.notice = Some(notice);
    }

    /// Open the input box for `target`. A draft closed with Esc comes back
    /// when the box reopens for the same target and reply.
    fn open_input(&mut self, target: InputTarget, reply_to: Option<(i64, i64, String)>) {
        if self.input_target != target || self.reply_to != reply_to {
            self.input_buffer.clear();
            self.input_cursor = 0;
        }
        self.input_mode = true;
        self.input_target = target;
        self.reply_to = reply_to;
    }

    fn clear_input(&mut self) {
        self.input_mode = false;
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    /// Panes that are drawn and can take the keys, in Tab order.
    fn focusable_panes(&self) -> Vec<DetailFocus> {
        let mut panes = vec![DetailFocus::Comments];
        if self.loading || self.error_message.is_some() {
            return panes;
        }
        if self.has_multiple_pages() {
            panes.push(DetailFocus::Episodes);
        }
        if self.recommendations_enabled {
            panes.push(DetailFocus::Related);
        }
        panes
    }

    fn cycle_focus(&mut self, forward: bool) {
        let panes = self.focusable_panes();
        let current = panes.iter().position(|pane| *pane == self.focus);
        let next = match (current, forward) {
            (None, _) => 0,
            (Some(index), true) => (index + 1) % panes.len(),
            (Some(index), false) => (index + panes.len() - 1) % panes.len(),
        };
        self.focus = panes[next];
    }

    fn set_episode(&mut self, index: usize) {
        let count = self.get_pages().map_or(0, Vec::len);
        if count > 0 {
            self.episode_scroll = index.min(count - 1);
        }
    }

    /// Up/Down/PageUp/PageDown/Home/End for the episode list and the
    /// related grid. `None` leaves the key to the rest of the handler.
    fn handle_list_key(
        &mut self,
        key: KeyCode,
        keys: &crate::storage::Keybindings,
    ) -> Option<AppAction> {
        let moves = [
            keys.matches_down(key),
            keys.matches_up(key),
            keys.matches_page_down(key),
            keys.matches_page_up(key),
            keys.matches_jump_top(key),
            keys.matches_jump_bottom(key),
        ];
        // The last flag (jump to the end) is the `else` branch below.
        let [down, up, page_down, page_up, top, _] = moves;
        if !moves.contains(&true) {
            return None;
        }
        match self.focus {
            DetailFocus::Comments => {}
            DetailFocus::Episodes => {
                let page = self.episode_visible_rows.get().max(1);
                let current = self.episode_scroll;
                self.set_episode(if down {
                    current + 1
                } else if up {
                    current.saturating_sub(1)
                } else if page_down {
                    current + page
                } else if page_up {
                    current.saturating_sub(page)
                } else if top {
                    0
                } else {
                    usize::MAX
                });
            }
            DetailFocus::Related => {
                let grid = &mut self.related_card_grid;
                let moved = if down {
                    grid.move_down()
                } else if up {
                    grid.move_up()
                } else if page_down {
                    grid.move_page_down()
                } else if page_up {
                    grid.move_page_up()
                } else if top {
                    grid.move_first()
                } else {
                    grid.move_last()
                };
                if moved {
                    self.related_scroll = self.related_card_grid.selected_index;
                }
            }
        }
        Some(AppAction::None)
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

    /// The comment area of the video.
    pub fn comment_target(&self) -> CommentTarget {
        CommentTarget {
            oid: self.aid,
            comment_type: 1,
        }
    }

    /// Turn a panel request into an action; `Compose` opens the reply box.
    fn comment_action(&mut self, request: PanelRequest) -> AppAction {
        if let PanelRequest::Compose { root, parent, to } = request {
            let reply_to = root
                .zip(parent)
                .zip(to)
                .map(|((root, parent), to)| (root, parent, to));
            self.open_input(InputTarget::Comment, reply_to);
            return AppAction::None;
        }
        AppAction::Comment(self.comment_target(), request)
    }

    /// Poll for completed related video cover downloads
    pub fn poll_cover_results(&mut self) {
        self.related_card_grid.poll_cover_results();
        self.comments.poll_picture_results();
    }

    /// Start background downloads for visible related video covers
    pub fn start_cover_downloads(&mut self) {
        self.related_card_grid.start_cover_downloads();
        self.comments.start_picture_downloads();
    }

    fn render_video_info(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle))
            .title(Span::styled(
                " 视频信息 ",
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
                Span::styled(
                    format!("  {} ", super::icons::DANMAKU),
                    Style::default().fg(theme.fg_secondary),
                ),
                Span::styled(
                    info.stat.format_danmaku(),
                    Style::default().fg(theme.fg_secondary),
                ),
                Span::styled(
                    format!("  {} ", super::icons::LIKE),
                    Style::default().fg(theme.fg_secondary),
                ),
                Span::styled(info.stat.format_like(), marked(relation.like)),
                Span::styled(
                    format!("  {} ", super::icons::COIN),
                    Style::default().fg(theme.fg_secondary),
                ),
                Span::styled(
                    if relation.coin > 0 {
                        format!("{} (已投{})", info.stat.format_coin(), relation.coin)
                    } else {
                        info.stat.format_coin()
                    },
                    marked(relation.coin > 0),
                ),
                Span::styled(
                    format!("  {} ", super::icons::STAR),
                    Style::default().fg(theme.fg_secondary),
                ),
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

    /// Comments on the left, related videos on the right. The right column
    /// takes 40% of the page but no more than one home feed card, so a wide
    /// screen gives the extra room to comments instead of stretching cards.
    fn content_columns(area: Rect) -> [Rect; 2] {
        let related_width = (area.width.saturating_mul(2) / 5)
            .min(super::video_card::list_card_width(area.width).saturating_add(2));
        Layout::horizontal([Constraint::Min(0), Constraint::Length(related_width)]).areas(area)
    }

    fn render_comments(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let focused = self.focus == DetailFocus::Comments;
        self.comments.render(frame, area, focused, theme);
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
                " 相关推荐 ",
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
                format!(" 选集 ({}) ", pages.len()),
                Style::default().fg(if is_focused {
                    theme.bilibili_pink
                } else {
                    theme.fg_muted
                }),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let visible_count = inner.height as usize;
        self.episode_visible_rows.set(visible_count);
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
                    Constraint::Length(1), // Help
                ])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6), // Video info
                    Constraint::Min(10),   // Comments + Related
                    Constraint::Length(1), // Help
                ])
                .split(area)
        };

        // Video info
        self.render_video_info(frame, chunks[0], theme);

        if self.loading {
            let loading = Paragraph::new("加载中...")
                .style(Style::default().fg(theme.warning))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded),
                );
            frame.render_widget(loading, chunks[1]);
        } else if let Some(error) = &self.error_message {
            let error_widget = Paragraph::new(error.to_string())
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
                let content = Self::content_columns(chunks[1]);
                self.render_comments(frame, content[0], theme);
                self.render_episodes(frame, content[1], theme);
            } else {
                self.render_comments(frame, chunks[1], theme);
            }
        } else {
            // Comments and Related split
            let content_chunks = Self::content_columns(chunks[1]);

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
                    match (&self.input_target, &self.reply_to) {
                        (InputTarget::Danmaku, _) => " 发送弹幕 ".to_string(),
                        (InputTarget::Comment, Some((_, _, to))) => format!(" 回复 @{to} "),
                        (InputTarget::Comment, None) => " 发表评论 ".to_string(),
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
                    (
                        "Enter".to_string(),
                        match self.input_target {
                            InputTarget::Comment => "发送评论",
                            InputTarget::Danmaku => "发送弹幕",
                        }
                        .into(),
                        theme.success,
                    ),
                    ("Esc".to_string(), "收起（保留草稿）".into(), theme.info),
                    ("Ctrl+U".to_string(), "清空".into(), theme.fg_muted),
                ],
            )
        } else {
            super::hint_footer(
                theme,
                [
                    (keys.play.clone(), "播放".into(), theme.success),
                    (keys.nav_next_page.clone(), "切换面板".into(), theme.info),
                    (keys.comment.clone(), "评论".into(), theme.info),
                    (
                        keys.video_triple.clone(),
                        "三连".into(),
                        theme.bilibili_pink,
                    ),
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
                KeyCode::Enter if self.input_target == InputTarget::Danmaku => {
                    let message = self.input_buffer.trim().to_string();
                    if let (false, Some(aid), Some(cid)) = (
                        message.is_empty(),
                        self.video_info.as_ref().map(|info| info.aid),
                        self.current_cid(),
                    ) {
                        self.clear_input();
                        return Some(AppAction::VideoAction(VideoActionRequest::SendDanmaku {
                            aid,
                            cid,
                            message,
                            progress_ms: 0,
                        }));
                    }
                    return Some(AppAction::None);
                }
                KeyCode::Enter => {
                    let message = self.input_buffer.trim().to_string();
                    if !message.is_empty() {
                        self.clear_input();
                        // The reply target stays so a failed post reopens
                        // as the same reply.
                        let reply_to = self.reply_to.clone();
                        return Some(AppAction::AddComment {
                            oid: self.aid,
                            comment_type: 1, // Video comment type
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

        // Esc first leaves an open reply thread.
        if keys.matches_back(key)
            && self.focus == DetailFocus::Comments
            && self.comments.close_thread()
        {
            return Some(AppAction::None);
        }
        if keys.matches_quit(key) || keys.matches_back(key) {
            return Some(AppAction::BackToList);
        }
        if keys.matches_comment_sort(key) {
            if self.loading || self.video_info.is_none() {
                return Some(AppAction::None);
            }
            let request = self.comments.begin_sort_toggle();
            return Some(self.comment_action(request));
        }
        if self.focus == DetailFocus::Comments
            && let Some(handled) = self.comments.handle_key(key, keys)
        {
            return Some(handled.map_or(AppAction::None, |request| self.comment_action(request)));
        }
        if keys.matches_open_up(key)
            && let Some(info) = &self.video_info
        {
            return Some(AppAction::OpenUpPage(info.owner.mid));
        }
        if let Some(request) = self.video_action_for(key, keys) {
            if self.action_pending {
                self.notice = Some("上一个操作还在进行，请稍候".to_string());
                return Some(AppAction::None);
            }
            return Some(AppAction::VideoAction(request));
        }
        if keys.matches_send_danmaku(key) && self.video_info.is_some() {
            self.open_input(InputTarget::Danmaku, None);
            return Some(AppAction::None);
        }
        if keys.matches_play(key) {
            return Some(self.play_action());
        }
        if keys.matches_comment(key) {
            self.open_input(InputTarget::Comment, None);
            return Some(AppAction::None);
        }
        // Tab/BackTab cycle through the panes on screen.
        if keys.matches_nav_next(key) || keys.matches_nav_prev(key) {
            self.cycle_focus(keys.matches_nav_next(key));
            return Some(AppAction::None);
        }
        if let Some(handled) = self.handle_list_key(key, keys) {
            return Some(handled);
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
                DetailFocus::Comments => {}
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
                        return self
                            .comments
                            .handle_mouse(event)
                            .map(|request| self.comment_action(request));
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
                        return self
                            .comments
                            .handle_mouse(event)
                            .map(|request| self.comment_action(request));
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

                let content_chunks = Self::content_columns(chunks[1]);

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

    fn batch(ids: &[i64]) -> crate::api::comment::CommentBatch {
        crate::api::comment::CommentBatch {
            comments: ids
                .iter()
                .map(|id| {
                    serde_json::from_value(serde_json::json!({
                        "rpid": id, "oid": 1, "mid": 1, "parent": 0, "rcount": 2
                    }))
                    .unwrap()
                })
                .collect(),
            next_offset: None,
        }
    }

    #[test]
    fn comment_keys_go_to_the_panel_and_carry_the_video_target() {
        let keys = Keybindings::default();
        let mut page = loaded_page();
        page.comments.set_comments(batch(&[7, 8]));
        let action = page.handle_input(KeyCode::Enter, &keys);
        assert!(matches!(
            action,
            Some(AppAction::Comment(
                CommentTarget {
                    oid: 1,
                    comment_type: 1
                },
                PanelRequest::Like {
                    rpid: 7,
                    like: true
                }
            ))
        ));
        let action = page.handle_input(KeyCode::Char('r'), &keys);
        assert!(matches!(
            action,
            Some(AppAction::Comment(
                _,
                PanelRequest::Replies { root: 7, page: 1 }
            ))
        ));
        // Esc leaves the reply thread before leaving the page.
        assert!(matches!(
            page.handle_input(KeyCode::Esc, &keys),
            Some(AppAction::None)
        ));
        assert!(matches!(
            page.handle_input(KeyCode::Esc, &keys),
            Some(AppAction::BackToList)
        ));
    }

    #[test]
    fn replying_in_a_thread_posts_with_root_and_parent() {
        let keys = Keybindings::default();
        let mut page = loaded_page();
        page.comments.set_comments(batch(&[7]));
        page.handle_input(KeyCode::Char('r'), &keys);
        page.comments
            .apply_replies(7, 1, batch(&[70]).comments, false);
        page.handle_input(KeyCode::Char('c'), &keys);
        assert!(page.input_mode);
        page.handle_input(KeyCode::Char('x'), &keys);
        let action = page.handle_input(KeyCode::Enter, &keys);
        assert!(matches!(
            action,
            Some(AppAction::AddComment {
                root: Some(7),
                parent: Some(70),
                ..
            })
        ));
    }

    #[test]
    fn sort_cannot_change_while_the_page_loads() {
        let keys = Keybindings::default();
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        assert!(matches!(
            page.handle_input(KeyCode::Char('o'), &keys),
            Some(AppAction::None)
        ));
        assert_eq!(page.comments.sort, crate::api::comment::CommentSort::Hot);
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
        page.loading = false;
        let keys = Keybindings::default();
        page.handle_input(KeyCode::Tab, &keys);
        assert!(matches!(page.focus, DetailFocus::Episodes));
        page.handle_input(KeyCode::Tab, &keys);
        assert!(matches!(page.focus, DetailFocus::Comments));
        page.handle_input(KeyCode::BackTab, &keys);
        assert!(matches!(page.focus, DetailFocus::Episodes));
        page.handle_input(KeyCode::End, &keys);
        assert_eq!(page.episode_scroll, 1);
        page.handle_input(KeyCode::Char('g'), &keys);
        assert_eq!(page.episode_scroll, 0);
    }

    #[test]
    fn tab_skips_panes_hidden_while_loading() {
        let mut page = VideoDetailPage::new("BV1test".into(), 1);
        let keys = Keybindings::default();
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn related_column_is_no_wider_than_a_home_feed_card_on_wide_screens() {
        let wide = Rect::new(0, 0, 240, 50);
        let [comments, related] = VideoDetailPage::content_columns(wide);
        assert_eq!(
            related.width,
            super::super::video_card::list_card_width(240) + 2
        );
        assert!(related.width < 240 * 2 / 5);
        assert_eq!(comments.width + related.width, 240);

        let narrow = Rect::new(0, 0, 120, 40);
        let [_, related] = VideoDetailPage::content_columns(narrow);
        assert_eq!(related.width, 48, "narrow screens keep the 40% column");
    }
}
