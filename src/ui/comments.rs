//! Comment panel shared by video, dynamic and article pages: full comment
//! text, pictures, likes, replies and a reply thread view.
//!
//! The panel owns its list state and returns `CommentRequest`s for the page
//! to turn into actions; it never talks to the network itself.

use super::Theme;
use super::text::wrap_to_width;
use crate::api::comment::{CommentBatch, CommentItem, CommentSort};
use crate::storage::Keybindings;
use image::DynamicImage;
use ratatui::{
    crossterm::event::{KeyCode, MouseEvent, MouseEventKind},
    prelude::*,
    widgets::*,
};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Rows a comment picture takes in the list.
const PICTURE_ROWS: u16 = 8;
/// Pictures shown per comment; the rest are counted.
const MAX_PICTURES: usize = 3;

/// Where a comment area lives: `oid` and comment `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentTarget {
    pub oid: i64,
    pub comment_type: i32,
}

/// Something the panel needs the page to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelRequest {
    /// Load top-level comments; `offset: None` replaces the list.
    Comments {
        sort: CommentSort,
        offset: Option<String>,
    },
    /// Load page `page` of the replies under `root`.
    Replies {
        root: i64,
        page: i32,
    },
    Like {
        rpid: i64,
        like: bool,
    },
    /// Open the reply box: `root` is the thread, `parent` the comment
    /// replied to; both `None` for a new top-level comment.
    Compose {
        root: Option<i64>,
        parent: Option<i64>,
        to: Option<String>,
    },
}

/// Replies of one thread, shown instead of the top-level list.
struct Thread {
    root: CommentItem,
    replies: Vec<CommentItem>,
    page: i32,
    has_more: bool,
    loading: bool,
    selected: usize,
    scroll: usize,
}

struct PictureResult {
    url: String,
    protocol: Option<StatefulProtocol>,
}

pub struct CommentPanel {
    pub comments: Vec<CommentItem>,
    pub sort: CommentSort,
    offset: Option<String>,
    pub has_more: bool,
    pub loading: bool,
    selected: usize,
    /// First list row drawn last frame.
    scroll: usize,
    thread: Option<Thread>,
    pub liked: HashSet<i64>,
    /// Likes counted by this session, applied on top of the server count.
    like_delta: HashMap<i64, i64>,
    pictures: HashMap<String, Option<StatefulProtocol>>,
    picture_tx: mpsc::Sender<PictureResult>,
    picture_rx: mpsc::Receiver<PictureResult>,
    picker: Arc<Picker>,
    /// Inner height of the panel last frame, for paging.
    visible_rows: usize,
}

/// One rendered row of a comment, with the picture it starts, if any.
enum Row {
    Text(Line<'static>),
    Picture(String),
}

impl Default for CommentPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl CommentPanel {
    pub fn new() -> Self {
        let (picture_tx, picture_rx) = mpsc::channel(16);
        Self {
            comments: Vec::new(),
            sort: CommentSort::default(),
            offset: None,
            has_more: false,
            loading: false,
            selected: 0,
            scroll: 0,
            thread: None,
            liked: HashSet::new(),
            like_delta: HashMap::new(),
            pictures: HashMap::new(),
            picture_tx,
            picture_rx,
            picker: super::image_picker(),
            visible_rows: 10,
        }
    }

    // ----- loading -----

    /// Replace the list with the first page of a load.
    pub fn set_comments(&mut self, batch: CommentBatch) {
        self.comments = batch.comments;
        self.has_more = batch.next_offset.is_some();
        self.offset = batch.next_offset;
        self.selected = 0;
        self.scroll = 0;
        self.thread = None;
        self.loading = false;
    }

    /// A page for `sort`; pages of a sort no longer selected are dropped.
    pub fn apply_batch(&mut self, sort: CommentSort, append: bool, batch: CommentBatch) {
        if sort != self.sort {
            return;
        }
        if !append {
            self.set_comments(batch);
            return;
        }
        self.comments.extend(batch.comments);
        self.has_more = batch.next_offset.is_some();
        self.offset = batch.next_offset;
        self.loading = false;
    }

    pub fn apply_error(&mut self) {
        self.loading = false;
        if let Some(thread) = self.thread.as_mut() {
            thread.loading = false;
        }
    }

    /// The next top-level page, unless one is loading or none is left.
    pub fn begin_load_more(&mut self) -> Option<PanelRequest> {
        if self.thread.is_some() || self.loading || !self.has_more {
            return None;
        }
        let offset = self.offset.clone()?;
        self.loading = true;
        Some(PanelRequest::Comments {
            sort: self.sort,
            offset: Some(offset),
        })
    }

    /// Switch between hot and newest first and request the first page.
    pub fn begin_sort_toggle(&mut self) -> PanelRequest {
        self.sort = self.sort.toggled();
        self.set_comments(CommentBatch::default());
        self.loading = true;
        PanelRequest::Comments {
            sort: self.sort,
            offset: None,
        }
    }

    /// Request the first page again in the current sort, keeping the list
    /// until it arrives (after posting a comment).
    pub fn begin_reload(&mut self) -> PanelRequest {
        self.loading = true;
        PanelRequest::Comments {
            sort: self.sort,
            offset: None,
        }
    }

    pub fn reset_sort(&mut self) {
        self.sort = CommentSort::default();
    }

    /// The thread whose replies are open, if any.
    pub fn open_thread_root(&self) -> Option<i64> {
        self.thread.as_ref().map(|thread| thread.root.rpid)
    }

    pub fn thread_loading(&self) -> bool {
        self.thread.as_ref().is_some_and(|thread| thread.loading)
    }

    /// Replies for the open thread; replies of a closed thread are dropped.
    pub fn apply_replies(
        &mut self,
        root: i64,
        page: i32,
        replies: Vec<CommentItem>,
        has_more: bool,
    ) {
        let Some(thread) = self.thread.as_mut().filter(|t| t.root.rpid == root) else {
            return;
        };
        if page <= 1 {
            thread.replies = replies;
            thread.selected = 0;
            thread.scroll = 0;
        } else {
            thread.replies.extend(replies);
        }
        thread.page = page;
        thread.has_more = has_more;
        thread.loading = false;
    }

    pub fn apply_like(&mut self, rpid: i64, liked: bool) {
        let was = self.liked.contains(&rpid);
        if liked == was {
            return;
        }
        let delta = self.like_delta.entry(rpid).or_default();
        if liked {
            self.liked.insert(rpid);
            *delta += 1;
        } else {
            self.liked.remove(&rpid);
            *delta -= 1;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.comments.is_empty()
    }

    pub fn len(&self) -> usize {
        self.comments.len()
    }

    // ----- input -----

    fn current_list(&self) -> (&[CommentItem], usize) {
        match &self.thread {
            Some(thread) => (&thread.replies, thread.selected),
            None => (&self.comments, self.selected),
        }
    }

    /// The comment the cursor is on.
    pub fn selected_comment(&self) -> Option<&CommentItem> {
        let (list, selected) = self.current_list();
        list.get(selected)
    }

    fn set_selected(&mut self, index: usize) {
        match self.thread.as_mut() {
            Some(thread) => thread.selected = index,
            None => self.selected = index,
        }
    }

    fn move_selection(&mut self, delta: isize) -> Option<PanelRequest> {
        let (list, selected) = self.current_list();
        let len = list.len();
        if len == 0 {
            return None;
        }
        let next = selected.saturating_add_signed(delta).min(len - 1);
        self.set_selected(next);
        // Load ahead when the cursor nears the end of what is loaded.
        if next + 3 < len {
            return None;
        }
        match self.thread.as_mut() {
            Some(thread) if thread.has_more && !thread.loading => {
                thread.loading = true;
                Some(PanelRequest::Replies {
                    root: thread.root.rpid,
                    page: thread.page + 1,
                })
            }
            Some(_) => None,
            None => self.begin_load_more(),
        }
    }

    /// Open the replies of the selected top-level comment.
    fn open_thread(&mut self) -> Option<PanelRequest> {
        if self.thread.is_some() {
            return None;
        }
        let root = self.comments.get(self.selected)?.clone();
        if root.reply_count() == 0 {
            return None;
        }
        let rpid = root.rpid;
        self.thread = Some(Thread {
            replies: root.replies.clone().unwrap_or_default(),
            root,
            page: 0,
            has_more: true,
            loading: true,
            selected: 0,
            scroll: 0,
        });
        Some(PanelRequest::Replies {
            root: rpid,
            page: 1,
        })
    }

    /// Close the thread view; returns whether one was open.
    pub fn close_thread(&mut self) -> bool {
        self.thread.take().is_some()
    }

    /// Keys the panel handles while the page focuses it. `None` means the
    /// key was not for the panel; `Some(None)` that it was handled here.
    pub fn handle_key(&mut self, key: KeyCode, keys: &Keybindings) -> Option<Option<PanelRequest>> {
        if keys.matches_down(key) {
            return Some(self.move_selection(1));
        }
        if keys.matches_up(key) {
            return Some(self.move_selection(-1));
        }
        if keys.matches_page_down(key) {
            return Some(self.move_selection(self.page_step()));
        }
        if keys.matches_page_up(key) {
            return Some(self.move_selection(-self.page_step()));
        }
        if keys.matches_toggle_replies(key) || keys.matches_right(key) {
            return Some(self.open_thread());
        }
        if self.thread.is_some() && (keys.matches_left(key) || keys.matches_back(key)) {
            self.close_thread();
            return Some(None);
        }
        if keys.matches_confirm(key) {
            let comment = self.selected_comment()?;
            let rpid = comment.rpid;
            return Some(Some(PanelRequest::Like {
                rpid,
                like: !self.liked.contains(&rpid),
            }));
        }
        if keys.matches_comment(key) {
            // In a thread, `c` replies to the selected reply (or the root);
            // in the list it writes a new top-level comment.
            return Some(Some(match &self.thread {
                Some(thread) => {
                    let target = thread.replies.get(thread.selected).unwrap_or(&thread.root);
                    PanelRequest::Compose {
                        root: Some(thread.root.rpid),
                        parent: Some(target.rpid),
                        to: Some(target.author_name().to_string()),
                    }
                }
                None => PanelRequest::Compose {
                    root: None,
                    parent: None,
                    to: None,
                },
            }));
        }
        // `jump_bottom` doubles as "load more": it lands on the last loaded
        // comment, which asks for the next page.
        if keys.matches_load_more(key) || keys.matches_jump_bottom(key) {
            return Some(self.move_selection(isize::MAX / 2));
        }
        if keys.matches_jump_top(key) {
            self.jump_to_top();
            return Some(None);
        }
        None
    }

    /// Select the first comment and show the top of the list, including a
    /// thread's root comment, which is not selectable.
    pub fn jump_to_top(&mut self) {
        match self.thread.as_mut() {
            Some(thread) => {
                thread.selected = 0;
                thread.scroll = 0;
            }
            None => {
                self.selected = 0;
                self.scroll = 0;
            }
        }
    }

    /// Reply to the selected top-level comment from the list.
    pub fn reply_to_selected(&self) -> Option<PanelRequest> {
        let comment = match &self.thread {
            Some(thread) => thread.replies.get(thread.selected).unwrap_or(&thread.root),
            None => self.comments.get(self.selected)?,
        };
        let root = self
            .thread
            .as_ref()
            .map_or(comment.rpid, |thread| thread.root.rpid);
        Some(PanelRequest::Compose {
            root: Some(root),
            parent: Some(comment.rpid),
            to: Some(comment.author_name().to_string()),
        })
    }

    fn page_step(&self) -> isize {
        // About one screen of comments, counting a comment as four rows.
        (self.visible_rows / 4).max(1) as isize
    }

    pub fn handle_mouse(&mut self, event: MouseEvent) -> Option<PanelRequest> {
        match event.kind {
            MouseEventKind::ScrollDown => self.move_selection(1),
            MouseEventKind::ScrollUp => self.move_selection(-1),
            _ => None,
        }
    }

    // ----- pictures -----

    /// Download pictures of the comments near the cursor.
    pub fn start_picture_downloads(&mut self) {
        let (list, selected) = self.current_list();
        let start = selected.saturating_sub(2);
        let urls: Vec<String> = list
            .iter()
            .skip(start)
            .take(8)
            .flat_map(|comment| comment.picture_urls().into_iter().take(MAX_PICTURES))
            .filter(|url| !self.pictures.contains_key(url))
            .collect();
        for url in urls {
            self.pictures.insert(url.clone(), None);
            let tx = self.picture_tx.clone();
            let picker = Arc::clone(&self.picker);
            tokio::spawn(async move {
                let protocol = download_image(&url)
                    .await
                    .map(|image| picker.new_resize_protocol(image));
                let _ = tx.send(PictureResult { url, protocol }).await;
            });
        }
    }

    pub fn poll_picture_results(&mut self) {
        while let Ok(result) = self.picture_rx.try_recv() {
            if let Some(slot) = self.pictures.get_mut(&result.url) {
                *slot = result.protocol;
            }
        }
    }

    // ----- drawing -----

    fn like_count(&self, comment: &CommentItem) -> String {
        let count = i64::from(comment.like.unwrap_or(0))
            + self.like_delta.get(&comment.rpid).copied().unwrap_or(0);
        if count >= 10_000 {
            format!("{:.1}万", count as f64 / 10_000.0)
        } else {
            count.max(0).to_string()
        }
    }

    /// Rows of one comment at `width`: header, full text, pictures, footer.
    fn comment_rows(
        &self,
        comment: &CommentItem,
        width: usize,
        selected: bool,
        indent: usize,
        show_replies: bool,
        theme: &Theme,
    ) -> Vec<Row> {
        let pad = " ".repeat(indent);
        let marker = if selected { "▌" } else { " " };
        let marker_style = Style::default().fg(theme.bilibili_pink);
        let text_width = width.saturating_sub(indent + 2).max(4);
        let level = comment
            .member
            .as_ref()
            .and_then(|member| member.level_info.as_ref())
            .and_then(|level| level.current_level)
            .unwrap_or(0);
        let mut rows = vec![Row::Text(Line::from(vec![
            Span::styled(marker, marker_style),
            Span::raw(pad.clone()),
            Span::styled(
                comment.author_name().to_string(),
                Style::default()
                    .fg(theme.bilibili_pink)
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                format!("  Lv{level}  {}", comment.format_time()),
                Style::default().fg(theme.fg_muted),
            ),
        ]))];
        for line in wrap_to_width(comment.message(), text_width) {
            rows.push(Row::Text(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::raw(format!("{pad} ")),
                Span::styled(line, Style::default().fg(theme.fg_primary)),
            ])));
        }
        let urls = comment.picture_urls();
        for url in urls.iter().take(MAX_PICTURES) {
            rows.push(Row::Picture(url.clone()));
        }
        let liked = self.liked.contains(&comment.rpid);
        let mut footer = vec![
            Span::styled(marker, marker_style),
            Span::raw(format!("{pad} ")),
            Span::styled(
                format!("{} {}", super::icons::LIKE, self.like_count(comment)),
                Style::default().fg(if liked {
                    theme.bilibili_pink
                } else {
                    theme.fg_muted
                }),
            ),
        ];
        if urls.len() > MAX_PICTURES {
            footer.push(Span::styled(
                format!("  +{} 张图", urls.len() - MAX_PICTURES),
                Style::default().fg(theme.fg_muted),
            ));
        }
        if show_replies && comment.reply_count() > 0 {
            footer.push(Span::styled(
                format!("  {} {} 条回复", super::icons::REPLY, comment.reply_count()),
                Style::default().fg(theme.fg_accent),
            ));
        }
        rows.push(Row::Text(Line::from(footer)));
        // Inline preview of the first replies under a top-level comment.
        if show_replies {
            for reply in comment.replies.iter().flatten().take(2) {
                let text = format!("{}：{}", reply.author_name(), reply.message());
                for line in super::video_card::clamp_lines(&text, text_width.saturating_sub(2), 2) {
                    rows.push(Row::Text(Line::from(vec![
                        Span::styled(marker, marker_style),
                        Span::raw(format!("{pad}   ")),
                        Span::styled(line, Style::default().fg(theme.fg_secondary)),
                    ])));
                }
            }
        }
        rows.push(Row::Text(Line::raw("")));
        rows
    }

    fn row_height(row: &Row) -> u16 {
        match row {
            Row::Text(_) => 1,
            Row::Picture(_) => PICTURE_ROWS,
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let title = match &self.thread {
            Some(thread) => format!(
                " {} 的回复 ({}) ",
                thread.root.author_name(),
                thread.root.reply_count()
            ),
            None => format!(" 评论 ({}) · {} ", self.comments.len(), self.sort.label()),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if focused {
                theme.border_focused
            } else {
                theme.border_unfocused
            }))
            .title(Span::styled(
                title,
                Style::default().fg(if focused {
                    theme.bilibili_pink
                } else {
                    theme.fg_muted
                }),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.visible_rows = usize::from(inner.height);
        let width = usize::from(inner.width);

        // Build the comments as groups of rows; the selected group is kept
        // fully on screen when it fits.
        let mut groups: Vec<Vec<Row>> = Vec::new();

        let selected_group = match &self.thread {
            Some(thread) => {
                groups.push(self.comment_rows(&thread.root, width, false, 0, false, theme));
                for (index, reply) in thread.replies.iter().enumerate() {
                    groups.push(self.comment_rows(
                        reply,
                        width,
                        index == thread.selected,
                        2,
                        false,
                        theme,
                    ));
                }
                thread.selected + 1
            }
            None => {
                for (index, comment) in self.comments.iter().enumerate() {
                    groups.push(self.comment_rows(
                        comment,
                        width,
                        index == self.selected,
                        0,
                        true,
                        theme,
                    ));
                }
                self.selected
            }
        };
        let status = if self.thread_loading() || self.loading {
            Some("加载中...")
        } else if groups.is_empty() || (self.thread.is_some() && groups.len() == 1) {
            Some(if self.thread.is_some() {
                "暂无回复"
            } else {
                "暂无评论"
            })
        } else if self.thread.as_ref().is_some_and(|t| !t.has_more) || !self.has_more {
            Some("没有更多了")
        } else {
            None
        };
        if let Some(status) = status {
            groups.push(vec![Row::Text(Line::styled(
                status,
                Style::default().fg(theme.fg_muted),
            ))]);
        }

        let heights: Vec<u16> = groups
            .iter()
            .map(|group| group.iter().map(Self::row_height).sum())
            .collect();
        let scroll = match &self.thread {
            Some(thread) => thread.scroll,
            None => self.scroll,
        };
        let scroll = fit_scroll(&heights, scroll, selected_group, inner.height);
        match self.thread.as_mut() {
            Some(thread) => thread.scroll = scroll,
            None => self.scroll = scroll,
        }

        let mut y = inner.y;
        'groups: for group in groups.into_iter().skip(scroll) {
            for row in group {
                let height = Self::row_height(&row);
                if y + height > inner.bottom() {
                    break 'groups;
                }
                let row_area = Rect::new(inner.x, y, inner.width, height);
                match row {
                    Row::Text(line) => frame.render_widget(Paragraph::new(line), row_area),
                    Row::Picture(url) => self.render_picture(frame, row_area, &url, theme),
                }
                y += height;
            }
        }
    }

    fn render_picture(&mut self, frame: &mut Frame, area: Rect, url: &str, theme: &Theme) {
        // Leave the selection gutter and indent, then keep a 16:9-ish box.
        let width = (area.height * 4).min(area.width.saturating_sub(3));
        let picture_area = Rect::new(area.x + 3, area.y, width, area.height.saturating_sub(1));
        match self.pictures.get_mut(url) {
            Some(Some(protocol)) => {
                frame.render_stateful_widget(StatefulImage::new(), picture_area, protocol);
            }
            _ => frame.render_widget(
                Paragraph::new(format!("{} 图片加载中", super::icons::PICTURE))
                    .style(Style::default().fg(theme.fg_muted)),
                picture_area,
            ),
        }
    }
}

/// First group to draw so that `selected` stays on screen: keep `scroll`
/// when the selection is visible, otherwise start at the selection or
/// scroll just far enough for it to end at the bottom.
fn fit_scroll(heights: &[u16], scroll: usize, selected: usize, height: u16) -> usize {
    let selected = selected.min(heights.len().saturating_sub(1));
    if selected < scroll {
        return selected;
    }
    let mut scroll = scroll;
    loop {
        let used: u32 = heights[scroll..=selected]
            .iter()
            .map(|h| u32::from(*h))
            .sum();
        if used <= u32::from(height) || scroll == selected {
            return scroll;
        }
        scroll += 1;
    }
}

async fn download_image(url: &str) -> Option<DynamicImage> {
    let bytes = reqwest::get(url).await.ok()?.bytes().await.ok()?;
    image::load_from_memory(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(rpid: i64, rcount: i32) -> CommentItem {
        serde_json::from_value(serde_json::json!({
            "rpid": rpid, "oid": 1, "mid": 1, "parent": 0, "rcount": rcount, "like": 10,
            "member": {"uname": format!("u{rpid}")},
            "content": {"message": "很长的评论".repeat(30)}
        }))
        .unwrap()
    }

    fn panel(ids: &[i64], next: Option<&str>) -> CommentPanel {
        let mut panel = CommentPanel::new();
        panel.set_comments(CommentBatch {
            comments: ids.iter().map(|id| comment(*id, 3)).collect(),
            next_offset: next.map(str::to_string),
        });
        panel
    }

    #[test]
    fn scroll_keeps_the_selected_comment_on_screen() {
        assert_eq!(fit_scroll(&[5, 5, 5, 5], 0, 0, 10), 0);
        assert_eq!(fit_scroll(&[5, 5, 5, 5], 0, 2, 10), 1);
        assert_eq!(fit_scroll(&[5, 5, 5, 5], 3, 1, 10), 1);
        // A comment taller than the panel starts at the top.
        assert_eq!(fit_scroll(&[5, 40], 0, 1, 10), 1);
    }

    #[test]
    fn moving_near_the_end_loads_the_next_page_once() {
        let keys = Keybindings::default();
        let mut panel = panel(&[1, 2, 3, 4, 5, 6], Some("p2"));
        assert_eq!(panel.handle_key(KeyCode::Char('j'), &keys), Some(None));
        assert_eq!(panel.handle_key(KeyCode::Char('j'), &keys), Some(None));
        // The third move reaches the last three comments.
        let request = panel.handle_key(KeyCode::Char('j'), &keys);
        assert_eq!(
            request,
            Some(Some(PanelRequest::Comments {
                sort: CommentSort::Hot,
                offset: Some("p2".into())
            }))
        );
        assert_eq!(panel.handle_key(KeyCode::Char('j'), &keys), Some(None));
    }

    #[test]
    fn threads_page_their_replies_and_close_with_back() {
        let keys = Keybindings::default();
        let mut panel = panel(&[1, 2], None);
        assert_eq!(
            panel.handle_key(KeyCode::Char('r'), &keys),
            Some(Some(PanelRequest::Replies { root: 1, page: 1 }))
        );
        assert!(panel.thread_loading());
        panel.apply_replies(1, 1, (10..16).map(|id| comment(id, 0)).collect(), true);
        assert_eq!(panel.selected_comment().unwrap().rpid, 10);
        panel.handle_key(KeyCode::Char('j'), &keys);
        panel.handle_key(KeyCode::Char('j'), &keys);
        let request = panel.handle_key(KeyCode::Char('j'), &keys);
        assert_eq!(
            request,
            Some(Some(PanelRequest::Replies { root: 1, page: 2 }))
        );
        panel.apply_replies(1, 2, vec![comment(20, 0)], false);
        assert_eq!(panel.thread.as_ref().unwrap().replies.len(), 7);
        // Replies for another thread are dropped.
        panel.apply_replies(2, 1, vec![comment(30, 0)], false);
        assert_eq!(panel.thread.as_ref().unwrap().replies.len(), 7);

        assert_eq!(panel.handle_key(KeyCode::Esc, &keys), Some(None));
        assert!(panel.open_thread_root().is_none());
        assert_eq!(panel.selected_comment().unwrap().rpid, 1);
    }

    #[test]
    fn like_toggles_and_adjusts_the_count() {
        let keys = Keybindings::default();
        let mut panel = panel(&[1], None);
        assert_eq!(
            panel.handle_key(KeyCode::Enter, &keys),
            Some(Some(PanelRequest::Like {
                rpid: 1,
                like: true
            }))
        );
        panel.apply_like(1, true);
        assert_eq!(panel.like_count(&panel.comments[0].clone()), "11");
        assert_eq!(
            panel.handle_key(KeyCode::Enter, &keys),
            Some(Some(PanelRequest::Like {
                rpid: 1,
                like: false
            }))
        );
        panel.apply_like(1, false);
        assert_eq!(panel.like_count(&panel.comments[0].clone()), "10");
    }

    #[test]
    fn compose_in_a_thread_replies_to_the_selected_reply() {
        let keys = Keybindings::default();
        let mut panel = panel(&[1], None);
        assert_eq!(
            panel.handle_key(KeyCode::Char('c'), &keys),
            Some(Some(PanelRequest::Compose {
                root: None,
                parent: None,
                to: None
            }))
        );
        panel.handle_key(KeyCode::Char('r'), &keys);
        panel.apply_replies(1, 1, vec![comment(10, 0)], false);
        assert_eq!(
            panel.handle_key(KeyCode::Char('c'), &keys),
            Some(Some(PanelRequest::Compose {
                root: Some(1),
                parent: Some(10),
                to: Some("u10".into())
            }))
        );
    }

    #[test]
    fn full_text_is_wrapped_instead_of_truncated() {
        let panel = panel(&[1], None);
        let rows = panel.comment_rows(&panel.comments[0], 40, false, 0, true, &Theme::default());
        let text_rows = rows
            .iter()
            .filter(|row| matches!(row, Row::Text(line) if line.to_string().contains("评论")))
            .count();
        assert!(text_rows > 5, "{text_rows} rows");
    }

    #[test]
    fn panel_draws_in_a_small_area() {
        let mut panel = panel(&[1, 2], None);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 4)).unwrap();
        terminal
            .draw(|frame| panel.render(frame, frame.area(), true, &Theme::default()))
            .unwrap();
    }

    #[test]
    fn home_and_end_jump_to_the_first_and_last_loaded_comment() {
        let keys = Keybindings::default();
        let mut panel = panel(&[1, 2, 3, 4, 5, 6], Some("p2"));
        let request = panel.handle_key(KeyCode::End, &keys);
        assert_eq!(panel.selected_comment().map(|c| c.rpid), Some(6));
        assert!(matches!(request, Some(Some(PanelRequest::Comments { .. }))));
        assert_eq!(panel.handle_key(KeyCode::Char('g'), &keys), Some(None));
        assert_eq!(panel.selected_comment().map(|c| c.rpid), Some(1));
    }
}
