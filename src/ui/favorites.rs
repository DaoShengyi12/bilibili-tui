use super::{Component, Theme, VideoCard, VideoCardGrid, shortcut_footer};
use crate::api::favorite::{
    CollectedFolder, FavoriteFolder, FavoriteResourceData, FavoriteSource, SeasonArchivesData,
    WatchLaterData,
};
use crate::application::AppAction;
use crate::storage::Keybindings;
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind},
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use std::time::Instant;

pub struct FavoritesPage {
    pub mid: i64,
    pub created: Vec<FavoriteFolder>,
    pub collected: Vec<CollectedFolder>,
    pub selected_source: usize,
    pub focus_sources: bool,
    pub active_source: FavoriteSource,
    pub videos: VideoCardGrid,
    pub page: i32,
    pub total: i64,
    /// Items received from the API for the active source, including ones
    /// without a BVID that are not shown. Paging compares this, not the
    /// card count, with `total`.
    fetched: usize,
    /// False once a page came back empty, whatever `total` claims.
    has_more: bool,
    pub loading: bool,
    pub loading_more: bool,
    pub error: Option<String>,
    last_click_time: Option<Instant>,
    last_click_index: Option<usize>,
    /// First source-list row drawn last frame; clicks map through it.
    sources_offset: std::cell::Cell<usize>,
}

impl FavoritesPage {
    pub fn new(mid: i64) -> Self {
        Self {
            mid,
            created: Vec::new(),
            collected: Vec::new(),
            selected_source: 0,
            focus_sources: true,
            active_source: FavoriteSource::WatchLater,
            videos: VideoCardGrid::new_list(),
            page: 1,
            total: 0,
            fetched: 0,
            has_more: true,
            loading: true,
            loading_more: false,
            error: None,
            last_click_time: None,
            last_click_index: None,
            sources_offset: std::cell::Cell::new(0),
        }
    }

    pub fn sources(&self) -> Vec<FavoriteSource> {
        let mut sources = vec![FavoriteSource::WatchLater];
        sources.extend(self.created.iter().map(|folder| FavoriteSource::Created {
            media_id: folder.id,
            title: folder.title.clone(),
        }));
        sources.extend(
            self.collected
                .iter()
                .map(|folder| FavoriteSource::Collected {
                    season_id: folder.id,
                    mid: folder.mid,
                    title: folder.title.clone(),
                }),
        );
        sources
    }

    pub fn apply_initial(
        &mut self,
        watch_later: WatchLaterData,
        created: Vec<FavoriteFolder>,
        collected: Vec<CollectedFolder>,
    ) {
        self.created = created;
        self.collected = collected;
        self.active_source = FavoriteSource::WatchLater;
        self.selected_source = 0;
        self.videos.clear();
        self.page = 1;
        self.total = watch_later.count;
        self.reset_paging();
        self.append_watch_later(watch_later);
        self.loading = false;
        self.error = None;
    }

    pub fn begin_source_load(&mut self, source: FavoriteSource) {
        self.active_source = source;
        self.page = 1;
        self.total = 0;
        self.reset_paging();
        self.videos.clear();
        self.loading = true;
        self.loading_more = false;
        self.error = None;
    }

    pub fn apply_watch_later(&mut self, page: i32, data: WatchLaterData) {
        if page == 1 {
            self.videos.clear();
            self.reset_paging();
        }
        self.page = page;
        self.total = data.count;
        self.append_watch_later(data);
        self.finish_load();
    }

    fn append_watch_later(&mut self, data: WatchLaterData) {
        self.record_page(data.list.len());
        for item in data.list {
            let author = item
                .owner
                .as_ref()
                .map(|owner| owner.name.clone())
                .unwrap_or_else(|| "未知UP".to_string());
            let uploader_mid = item.owner.as_ref().map(|owner| owner.mid);
            let views = item
                .stat
                .and_then(|stat| stat.view)
                .map(format_count)
                .unwrap_or_else(|| "-".to_string());
            self.videos.add_card(
                VideoCard::new(
                    Some(item.bvid),
                    Some(item.aid),
                    item.title,
                    author,
                    views,
                    format_duration(item.duration.unwrap_or_default()),
                    item.pic,
                )
                .with_uploader_mid(uploader_mid),
            );
        }
    }

    pub fn apply_created(&mut self, page: i32, data: FavoriteResourceData) {
        if page == 1 {
            self.videos.clear();
            self.reset_paging();
        }
        self.page = page;
        self.total = data
            .info
            .as_ref()
            .and_then(|info| info.media_count)
            .unwrap_or_default() as i64;
        self.record_page(data.medias.len());
        if data.has_more == Some(false) {
            self.has_more = false;
        }
        for item in data.medias {
            let Some(bvid) = item.bvid else { continue };
            let author = item
                .upper
                .as_ref()
                .map(|upper| upper.name.clone())
                .unwrap_or_else(|| "未知UP".to_string());
            let uploader_mid = item.upper.as_ref().map(|upper| upper.mid);
            let views = item
                .cnt_info
                .and_then(|count| count.play)
                .map(format_count)
                .unwrap_or_else(|| "-".to_string());
            self.videos.add_card(
                VideoCard::new(
                    Some(bvid),
                    Some(item.id),
                    item.title,
                    author,
                    views,
                    format_duration(item.duration.unwrap_or_default()),
                    item.cover,
                )
                .with_uploader_mid(uploader_mid),
            );
        }
        self.finish_load();
    }

    pub fn apply_collected(&mut self, page: i32, data: SeasonArchivesData) {
        if page == 1 {
            self.videos.clear();
            self.reset_paging();
        }
        self.page = data.page.page_num;
        self.total = data.page.total;
        self.record_page(data.archives.len());
        let author = match &self.active_source {
            FavoriteSource::Collected { title, .. } => title.clone(),
            _ => "合集".to_string(),
        };
        for item in data.archives {
            let views = item
                .stat
                .and_then(|stat| stat.view)
                .map(format_count)
                .unwrap_or_else(|| "-".to_string());
            self.videos.add_card(VideoCard::new(
                Some(item.bvid),
                Some(item.aid),
                item.title,
                author.clone(),
                views,
                format_duration(item.duration.unwrap_or_default()),
                item.pic,
            ));
        }
        self.finish_load();
    }

    /// Nothing is shown or loading, e.g. after Left cancelled the load; the
    /// active source can then be selected again to reload it.
    fn needs_reload(&self) -> bool {
        !self.loading && self.videos.cards.is_empty()
    }

    fn reset_paging(&mut self) {
        self.fetched = 0;
        self.has_more = true;
    }

    fn record_page(&mut self, items: usize) {
        self.fetched += items;
        if items == 0 {
            self.has_more = false;
        }
    }

    /// Whether another page exists. Uses the raw item count so items hidden
    /// for lacking a BVID do not make every scroll request an empty page.
    fn can_load_more(&self) -> bool {
        self.has_more && !self.loading_more && self.fetched < self.total.max(0) as usize
    }

    fn finish_load(&mut self) {
        self.loading = false;
        self.loading_more = false;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.loading = false;
        self.loading_more = false;
        self.error = Some(error);
    }

    fn source_label(source: &FavoriteSource) -> &str {
        match source {
            FavoriteSource::WatchLater => "稍后再看",
            FavoriteSource::Created { title, .. } | FavoriteSource::Collected { title, .. } => {
                title
            }
        }
    }
}

impl Component for FavoritesPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(30)])
            .split(area);
        let mut items = vec![
            ListItem::new("其他收藏").style(
                Style::default()
                    .fg(theme.fg_muted)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        items.push(ListItem::new("  稍后再看"));
        items.push(
            ListItem::new("我创建的收藏夹").style(
                Style::default()
                    .fg(theme.fg_muted)
                    .add_modifier(Modifier::BOLD),
            ),
        );
        items.extend(self.created.iter().map(|folder| {
            ListItem::new(format!(
                "  {}  ({})",
                folder.title,
                folder.media_count.unwrap_or_default()
            ))
        }));
        items.push(
            ListItem::new("我追的合集/收藏夹").style(
                Style::default()
                    .fg(theme.fg_muted)
                    .add_modifier(Modifier::BOLD),
            ),
        );
        items.extend(self.collected.iter().map(|folder| {
            ListItem::new(format!(
                "  {}  ({})",
                folder.title,
                folder.media_count.unwrap_or_default()
            ))
        }));
        let state_index = if self.selected_source == 0 {
            1
        } else if self.selected_source <= self.created.len() {
            self.selected_source + 2
        } else {
            self.selected_source + 3
        };
        let mut state = ListState::default()
            .with_offset(self.sources_offset.get())
            .with_selected(Some(state_index));
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" 收藏 "))
                .highlight_symbol("▶ ")
                .highlight_style(if self.focus_sources {
                    Style::default().fg(theme.bilibili_pink)
                } else {
                    Style::default().fg(theme.bilibili_cyan)
                }),
            chunks[0],
            &mut state,
        );
        self.sources_offset.set(state.offset());

        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(chunks[1]);
        frame.render_widget(
            Paragraph::new(format!(
                "{}  ·  {}/{}",
                Self::source_label(&self.active_source),
                self.videos.cards.len(),
                self.total
            ))
            .block(Block::default().borders(Borders::ALL)),
            right[0],
        );
        if self.loading {
            frame.render_widget(Paragraph::new("正在加载…"), right[1]);
        } else if let Some(error) = &self.error {
            frame.render_widget(
                Paragraph::new(error.as_str()).style(Style::default().fg(theme.error)),
                right[1],
            );
        } else {
            self.videos.render(frame, right[1], theme);
        }
        frame.render_widget(
            Paragraph::new(shortcut_footer(
                theme,
                [
                    ("↑/↓".into(), "选择".into(), theme.fg_accent),
                    (
                        format!("{}/{}", keys.page_up, keys.page_down),
                        "翻页".into(),
                        theme.fg_accent,
                    ),
                    ("←/→".into(), "收藏/视频".into(), theme.fg_accent),
                    (keys.confirm.clone(), "打开".into(), theme.success),
                    (keys.nav_next_page.clone(), "下一页面".into(), theme.info),
                    (keys.nav_prev_page.clone(), "上一页面".into(), theme.info),
                ],
            ))
            .alignment(Alignment::Center),
            right[2],
        );
    }

    fn handle_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        if keys.matches_quit(key) {
            return Some(AppAction::Quit);
        }
        if keys.matches_nav_next(key) {
            return Some(AppAction::NavNext);
        }
        if keys.matches_nav_prev(key) {
            return Some(AppAction::NavPrev);
        }
        let sources = self.sources();
        // Pane switching has priority over loading and list navigation.
        if keys.matches_left(key) {
            self.focus_sources = true;
            if self.loading || self.loading_more {
                self.loading = false;
                self.loading_more = false;
                return Some(AppAction::CancelPendingLoads);
            }
            return Some(AppAction::None);
        }
        if keys.matches_right(key) {
            if self.focus_sources
                && let Some(source) = sources.get(self.selected_source)
                && (*source != self.active_source || self.needs_reload())
            {
                self.focus_sources = false;
                return Some(AppAction::SelectFavoriteSource(source.clone()));
            }
            self.focus_sources = false;
            return Some(AppAction::None);
        }
        if self.focus_sources {
            if keys.matches_down(key) && self.selected_source + 1 < sources.len() {
                self.selected_source += 1;
            } else if keys.matches_up(key) && self.selected_source > 0 {
                self.selected_source -= 1;
            } else if keys.matches_confirm(key)
                && let Some(source) = sources.get(self.selected_source)
                && *source != self.active_source
            {
                return Some(AppAction::SelectFavoriteSource(source.clone()));
            }
            return Some(AppAction::None);
        }

        if keys.matches_page_down(key) {
            self.videos.move_page_down();
            if self.videos.is_near_bottom(self.videos.cached_visible_rows) && self.can_load_more() {
                self.loading_more = true;
                return Some(AppAction::LoadMoreFavorites);
            }
            return Some(AppAction::None);
        }
        if keys.matches_page_up(key) {
            self.videos.move_page_up();
            return Some(AppAction::None);
        }

        if keys.matches_down(key) {
            self.videos.move_down();
            if self.videos.is_near_bottom(self.videos.cached_visible_rows) && self.can_load_more() {
                self.loading_more = true;
                return Some(AppAction::LoadMoreFavorites);
            }
        } else if keys.matches_up(key) {
            self.videos.move_up();
        } else if keys.matches_confirm(key)
            && let Some(card) = self.videos.selected_card()
            && let (Some(bvid), Some(aid)) = (&card.bvid, card.aid)
        {
            return Some(AppAction::OpenVideoDetail(bvid.clone(), aid));
        }
        Some(AppAction::None)
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> Option<AppAction> {
        let position = Position::new(event.column, event.row);
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(30)])
            .split(area);
        let video_parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(panes[1]);
        match event.kind {
            MouseEventKind::ScrollDown if panes[1].contains(position) => {
                self.focus_sources = false;
                if self.videos.move_down()
                    && self.videos.is_near_bottom(self.videos.cached_visible_rows)
                    && self.can_load_more()
                {
                    self.loading_more = true;
                    return Some(AppAction::LoadMoreFavorites);
                }
            }
            MouseEventKind::ScrollUp if panes[1].contains(position) => {
                self.focus_sources = false;
                self.videos.move_up();
            }
            MouseEventKind::Down(MouseButton::Left) if panes[0].contains(position) => {
                self.focus_sources = true;
                let item_row =
                    self.sources_offset.get() + event.row.saturating_sub(panes[0].y + 1) as usize;
                let source_index = if item_row == 1 {
                    Some(0)
                } else if (3..3 + self.created.len()).contains(&item_row) {
                    Some(item_row - 2)
                } else {
                    let first_collected = 4 + self.created.len();
                    (item_row >= first_collected)
                        .then_some(1 + self.created.len() + item_row - first_collected)
                };
                let sources = self.sources();
                if let Some(index) = source_index.filter(|index| *index < sources.len()) {
                    self.selected_source = index;
                    if let Some(source) = sources.get(index)
                        && *source != self.active_source
                    {
                        return Some(AppAction::SelectFavoriteSource(source.clone()));
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) if video_parts[1].contains(position) => {
                self.focus_sources = false;
                if self.videos.select_at(event.row, video_parts[1]) {
                    let index = self.videos.selected_index;
                    let now = Instant::now();
                    let double = self.last_click_index == Some(index)
                        && self
                            .last_click_time
                            .is_some_and(|time| now.duration_since(time).as_millis() < 500);
                    self.last_click_index = Some(index);
                    self.last_click_time = Some(now);
                    if double
                        && let Some(card) = self.videos.selected_card()
                        && let (Some(bvid), Some(aid)) = (&card.bvid, card.aid)
                    {
                        return Some(AppAction::OpenVideoDetail(bvid.clone(), aid));
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Middle) if video_parts[1].contains(position) => {
                if let Some(card) = self.videos.selected_card()
                    && let (Some(bvid), Some(aid)) = (&card.bvid, card.aid)
                {
                    return Some(AppAction::OpenVideoDetail(bvid.clone(), aid));
                }
            }
            _ => {}
        }
        Some(AppAction::None)
    }
}

fn format_count(value: i64) -> String {
    if value >= 10_000 {
        format!("{:.1}万", value as f64 / 10_000.0)
    } else {
        value.to_string()
    }
}

fn format_duration(seconds: i64) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_without_bvid_do_not_make_paging_request_empty_pages() {
        let mut page = FavoritesPage::new(1);
        page.begin_source_load(FavoriteSource::Created {
            media_id: 9,
            title: "f".into(),
        });
        let data: FavoriteResourceData = serde_json::from_value(serde_json::json!({
            "info": {"id": 9, "title": "f", "media_count": 2},
            "medias": [
                {"id": 1, "bvid": "BV1", "title": "ok"},
                {"id": 2, "bvid": null, "title": "失效视频"}
            ],
            "has_more": false
        }))
        .unwrap();
        page.apply_created(1, data);
        assert_eq!(page.videos.cards.len(), 1);
        assert!(!page.can_load_more(), "both items were received");

        let mut page = FavoritesPage::new(1);
        page.begin_source_load(FavoriteSource::WatchLater);
        page.apply_watch_later(
            1,
            serde_json::from_value(serde_json::json!({"count": 50, "list": []})).unwrap(),
        );
        assert!(!page.can_load_more(), "an empty page ends paging");
    }

    #[test]
    fn favorites_is_a_normal_sidebar_page() {
        let mut page = FavoritesPage::new(1);
        let keys = Keybindings::default();
        assert!(matches!(
            page.handle_input(KeyCode::Tab, &keys),
            Some(AppAction::NavNext)
        ));
        assert!(matches!(page.active_source, FavoriteSource::WatchLater));
    }

    #[test]
    fn source_list_starts_with_watch_later_then_created_and_collected() {
        let mut page = FavoritesPage::new(1);
        page.created.push(FavoriteFolder {
            id: 2,
            fid: None,
            mid: 1,
            title: "我的收藏".to_string(),
            media_count: Some(1),
            fav_state: None,
            attr: None,
        });
        page.collected.push(CollectedFolder {
            id: 3,
            mid: 4,
            title: "我追的合集".to_string(),
            media_count: Some(2),
            state: Some(0),
        });
        let sources = page.sources();
        assert!(matches!(sources[0], FavoriteSource::WatchLater));
        assert!(matches!(sources[1], FavoriteSource::Created { .. }));
        assert!(matches!(sources[2], FavoriteSource::Collected { .. }));
    }

    #[test]
    fn favorites_video_list_is_single_column() {
        let page = FavoritesPage::new(1);
        assert_eq!(page.videos.columns, 1);
        assert!(page.videos.list_layout);
    }

    #[test]
    fn left_cancels_loading_before_video_navigation() {
        let mut page = FavoritesPage::new(1);
        page.focus_sources = false;
        page.loading = true;
        let keys = Keybindings::default();
        assert!(matches!(
            page.handle_input(KeyCode::Left, &keys),
            Some(AppAction::CancelPendingLoads)
        ));
        assert!(page.focus_sources);
        assert!(!page.loading);
    }

    #[test]
    fn source_whose_load_was_cancelled_can_be_selected_again() {
        let mut page = FavoritesPage::new(1);
        page.begin_source_load(FavoriteSource::WatchLater);
        page.focus_sources = false;
        let keys = Keybindings::default();
        page.handle_input(KeyCode::Left, &keys);
        assert!(page.videos.cards.is_empty());

        assert!(matches!(
            page.handle_input(KeyCode::Right, &keys),
            Some(AppAction::SelectFavoriteSource(FavoriteSource::WatchLater))
        ));
    }
}
