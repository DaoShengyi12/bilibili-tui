use super::{Component, Theme, VideoCard, VideoCardGrid, shortcut_footer};
use crate::api::{
    favorite::{FavoriteFolder, FavoriteOrder, FavoriteResourceData},
    space::{RelationStat, SpaceInfo, SpaceVideoData, SpaceVideoOrder},
};
use crate::application::AppAction;
use crate::domain::playback::PlayOrder;
use crate::storage::Keybindings;
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpTab {
    Videos,
    Favorites,
}

pub struct UpPage {
    pub mid: i64,
    /// Own space opened as a sidebar destination rather than a nested detail.
    pub is_sidebar_page: bool,
    pub profile: Option<SpaceInfo>,
    pub relation: Option<RelationStat>,
    pub tab: UpTab,
    pub video_order: SpaceVideoOrder,
    pub play_order: PlayOrder,
    pub videos: VideoCardGrid,
    pub video_page: i32,
    pub video_total: i64,
    pub folders: Vec<FavoriteFolder>,
    pub folder_selected: usize,
    pub active_folder: Option<i64>,
    pub pending_folder: Option<i64>,
    pub favorite_videos: VideoCardGrid,
    pub favorite_page: i32,
    pub favorite_order: FavoriteOrder,
    pub favorite_has_more: bool,
    pub loading: bool,
    pub loading_more: bool,
    pub error: Option<String>,
}

impl UpPage {
    pub fn new(mid: i64) -> Self {
        Self {
            mid,
            is_sidebar_page: false,
            profile: None,
            relation: None,
            tab: UpTab::Videos,
            video_order: SpaceVideoOrder::Latest,
            play_order: PlayOrder::Forward,
            videos: VideoCardGrid::new(),
            video_page: 1,
            video_total: 0,
            folders: Vec::new(),
            folder_selected: 0,
            active_folder: None,
            pending_folder: None,
            favorite_videos: VideoCardGrid::new(),
            favorite_page: 1,
            favorite_order: FavoriteOrder::RecentlyFavorited,
            favorite_has_more: false,
            loading: true,
            loading_more: false,
            error: None,
        }
    }

    pub fn new_sidebar(mid: i64) -> Self {
        Self {
            is_sidebar_page: true,
            ..Self::new(mid)
        }
    }

    pub fn new_favorites(mid: i64) -> Self {
        let mut page = Self::new(mid);
        page.tab = UpTab::Favorites;
        page
    }

    pub fn apply_initial(
        &mut self,
        profile: SpaceInfo,
        relation: Option<RelationStat>,
        videos: SpaceVideoData,
        folders: Vec<FavoriteFolder>,
    ) {
        self.apply_profile(profile, relation, folders);
        self.videos.clear();
        self.video_page = 1;
        self.video_total = videos.page.count;
        self.append_space_videos(videos);
        self.loading = false;
        self.error = None;
    }

    /// Applies the header and folder list without touching the video list.
    pub fn apply_profile(
        &mut self,
        profile: SpaceInfo,
        relation: Option<RelationStat>,
        folders: Vec<FavoriteFolder>,
    ) {
        self.profile = Some(profile);
        self.relation = relation;
        self.folders = folders;
    }

    pub fn apply_more_videos(&mut self, page: i32, videos: SpaceVideoData) {
        self.video_page = page;
        self.video_total = videos.page.count;
        self.append_space_videos(videos);
        self.loading_more = false;
    }

    fn append_space_videos(&mut self, videos: SpaceVideoData) {
        for video in videos.list.vlist {
            let duration = format_duration(video.duration.unwrap_or_default());
            let views = format_count(video.play.unwrap_or_default());
            let card = VideoCard::new(
                Some(video.bvid),
                Some(video.aid),
                video.title,
                video
                    .author
                    .or_else(|| self.profile.as_ref().map(|p| p.name.clone()))
                    .unwrap_or_else(|| "未知UP".to_string()),
                views,
                duration,
                video.pic,
            )
            .with_uploader_mid(Some(video.mid.unwrap_or(self.mid)));
            self.videos.add_card(card);
        }
    }

    pub fn apply_favorite_resources(
        &mut self,
        media_id: i64,
        page: i32,
        resources: FavoriteResourceData,
    ) {
        if page == 1 || self.active_folder != Some(media_id) {
            self.favorite_videos.clear();
        }
        self.active_folder = Some(media_id);
        self.pending_folder = None;
        self.favorite_page = page;
        self.favorite_has_more = resources.has_more.unwrap_or(false);
        for media in resources.medias {
            let Some(bvid) = media.bvid else { continue };
            let upper_name = media
                .upper
                .as_ref()
                .map(|upper| upper.name.clone())
                .unwrap_or_else(|| "未知UP".to_string());
            let upper_mid = media.upper.as_ref().map(|upper| upper.mid);
            let views = media
                .cnt_info
                .as_ref()
                .and_then(|count| count.play)
                .map(format_count)
                .unwrap_or_else(|| "-".to_string());
            let card = VideoCard::new(
                Some(bvid),
                Some(media.id),
                media.title,
                upper_name,
                views,
                format_duration(media.duration.unwrap_or_default()),
                media.cover,
            )
            .with_uploader_mid(upper_mid);
            self.favorite_videos.add_card(card);
        }
        self.loading = false;
        self.loading_more = false;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.loading = false;
        self.loading_more = false;
        self.pending_folder = None;
        self.error = Some(error);
    }

    fn selected_grid(&mut self) -> &mut VideoCardGrid {
        if self.tab == UpTab::Videos {
            &mut self.videos
        } else {
            &mut self.favorite_videos
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let (name, sign) = self
            .profile
            .as_ref()
            .map(|p| (p.name.as_str(), p.sign.as_deref().unwrap_or("暂无签名")))
            .unwrap_or(("加载中…", ""));
        let stats = self
            .relation
            .as_ref()
            .map(|r| {
                format!(
                    "关注 {}  ·  粉丝 {}",
                    format_count(r.following.unwrap_or_default()),
                    format_count(r.follower.unwrap_or_default())
                )
            })
            .unwrap_or_default();
        let text = vec![
            Line::from(Span::styled(
                name,
                Style::default()
                    .fg(theme.bilibili_pink)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(sign),
            Line::from(Span::styled(stats, Style::default().fg(theme.fg_muted))),
        ];
        frame.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL).title(" UP主空间 ")),
            area,
        );
    }
}

impl Component for UpPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(area);
        self.draw_header(frame, chunks[0], theme);

        let sort = match self.video_order {
            SpaceVideoOrder::Latest => "最新发布",
            SpaceVideoOrder::Popular => "最多播放",
        };
        let favorite_sort = match self.favorite_order {
            FavoriteOrder::RecentlyFavorited => "最近收藏",
            FavoriteOrder::MostPlayed => "最多播放",
            FavoriteOrder::RecentlyPublished => "最近投稿",
        };
        let play_order = match self.play_order {
            PlayOrder::Forward => "顺序播放",
            PlayOrder::Reverse => "倒序播放",
            PlayOrder::Shuffle => "随机播放",
        };
        let tabs = Tabs::new(vec!["1 投稿", "2 收藏夹"])
            .select(if self.tab == UpTab::Videos { 0 } else { 1 })
            .highlight_style(Style::default().fg(theme.bilibili_pink))
            .block(Block::default().borders(Borders::ALL).title(format!(
                " {} · {play_order} ",
                if self.tab == UpTab::Videos {
                    sort
                } else {
                    favorite_sort
                }
            )));
        frame.render_widget(tabs, chunks[1]);

        if self.loading {
            frame.render_widget(Paragraph::new("正在加载…"), chunks[2]);
        } else if let Some(error) = &self.error {
            frame.render_widget(
                Paragraph::new(error.as_str()).style(Style::default().fg(theme.error)),
                chunks[2],
            );
        } else if self.tab == UpTab::Videos {
            self.videos.render(frame, chunks[2], theme);
        } else if self.active_folder.is_some() {
            self.favorite_videos.render(frame, chunks[2], theme);
        } else {
            let items: Vec<ListItem> = self
                .folders
                .iter()
                .map(|folder| {
                    ListItem::new(format!(
                        "{}  ({}个视频)",
                        folder.title,
                        folder.media_count.unwrap_or_default()
                    ))
                })
                .collect();
            let mut state = ListState::default().with_selected(Some(self.folder_selected));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("▶ ")
                    .highlight_style(Style::default().fg(theme.bilibili_cyan)),
                chunks[2],
                &mut state,
            );
        }

        let mut shortcuts = vec![
            ("1/2".into(), "投稿/收藏夹".into(), theme.info),
            (
                format!("{}/{}", keys.page_up, keys.page_down),
                "翻页".into(),
                theme.fg_accent,
            ),
            ("o".into(), "最新/热门".into(), theme.info),
            ("s".into(), "顺序/倒序/随机".into(), theme.info),
            (keys.play.clone(), "连播".into(), theme.success),
            (keys.confirm.clone(), "打开".into(), theme.success),
            (keys.back.clone(), "返回".into(), theme.info),
        ];
        if self.is_sidebar_page {
            shortcuts.insert(
                0,
                (
                    format!("{}/{}", keys.nav_next_page, keys.nav_prev_page),
                    "切换页面".into(),
                    theme.info,
                ),
            );
        }
        frame.render_widget(
            Paragraph::new(shortcut_footer(theme, shortcuts)).alignment(Alignment::Center),
            chunks[3],
        );
    }

    fn handle_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        if self.is_sidebar_page {
            if keys.matches_nav_next(key) {
                return Some(AppAction::NavNext);
            }
            if keys.matches_nav_prev(key) {
                return Some(AppAction::NavPrev);
            }
            if keys.matches_quit(key) {
                return Some(AppAction::Quit);
            }
        }
        if keys.matches_back(key) || keys.matches_quit(key) {
            if self.tab == UpTab::Favorites && self.pending_folder.take().is_some() {
                self.loading = false;
                return Some(AppAction::None);
            }
            if self.tab == UpTab::Favorites && self.active_folder.take().is_some() {
                self.favorite_videos.clear();
                return Some(AppAction::None);
            }
            return Some(if self.is_sidebar_page {
                AppAction::NavPrev
            } else {
                AppAction::BackToList
            });
        }
        match key {
            KeyCode::Char('1') => {
                self.tab = UpTab::Videos;
                return Some(AppAction::None);
            }
            KeyCode::Char('2') => {
                self.tab = UpTab::Favorites;
                return Some(AppAction::None);
            }
            KeyCode::Char('o') if self.tab == UpTab::Videos => {
                self.video_order = if self.video_order == SpaceVideoOrder::Latest {
                    SpaceVideoOrder::Popular
                } else {
                    SpaceVideoOrder::Latest
                };
                return Some(AppAction::SwitchUpVideoOrder(self.video_order));
            }
            KeyCode::Char('o') if self.tab == UpTab::Favorites => {
                self.favorite_order = self.favorite_order.next();
                if let Some(media_id) = self.pending_folder {
                    // Results in the old order are discarded, so request the
                    // pending folder again in the new order.
                    return Some(AppAction::OpenFavoriteFolder(media_id));
                }
                if self.active_folder.is_some() {
                    self.loading = true;
                    return Some(AppAction::SwitchFavoriteOrder(self.favorite_order));
                }
                return Some(AppAction::None);
            }
            KeyCode::Char('s') => {
                self.play_order = match self.play_order {
                    PlayOrder::Forward => PlayOrder::Reverse,
                    PlayOrder::Reverse => PlayOrder::Shuffle,
                    PlayOrder::Shuffle => PlayOrder::Forward,
                };
                return Some(AppAction::None);
            }
            _ => {}
        }

        if self.tab == UpTab::Favorites && self.active_folder.is_none() {
            if keys.matches_down(key) && self.folder_selected + 1 < self.folders.len() {
                self.folder_selected += 1;
            } else if keys.matches_up(key) && self.folder_selected > 0 {
                self.folder_selected -= 1;
            } else if keys.matches_confirm(key)
                && let Some(folder) = self.folders.get(self.folder_selected)
            {
                self.loading = true;
                self.pending_folder = Some(folder.id);
                return Some(AppAction::OpenFavoriteFolder(folder.id));
            }
            return Some(AppAction::None);
        }

        let tab = self.tab;
        let loading_more = self.loading_more;
        let video_total = self.video_total;
        let favorite_has_more = self.favorite_has_more;
        let grid = self.selected_grid();
        if keys.matches_play(key) {
            if tab == UpTab::Videos {
                return Some(AppAction::PlayUpAll {
                    mid: self.mid,
                    name: self
                        .profile
                        .as_ref()
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "UP主投稿".to_string()),
                    video_order: self.video_order,
                    play_order: self.play_order,
                });
            } else {
                let media_id = self.active_folder.unwrap_or_default();
                let title = self
                    .folders
                    .iter()
                    .find(|folder| folder.id == media_id)
                    .map(|folder| folder.title.clone())
                    .unwrap_or_else(|| "收藏夹".to_string());
                return Some(AppAction::PlayFavoriteAll {
                    media_id,
                    title,
                    favorite_order: self.favorite_order,
                    play_order: self.play_order,
                });
            }
        }
        if keys.matches_page_down(key) {
            grid.move_page_down();
            if grid.is_near_bottom(grid.cached_visible_rows) && !loading_more {
                if tab == UpTab::Videos && grid.cards.len() < video_total as usize {
                    return Some(AppAction::LoadMoreUpVideos);
                }
                if tab == UpTab::Favorites && favorite_has_more {
                    return Some(AppAction::LoadMoreFavoriteResources);
                }
            }
            return Some(AppAction::None);
        }
        if keys.matches_page_up(key) {
            grid.move_page_up();
            return Some(AppAction::None);
        }
        if keys.matches_down(key) {
            grid.move_down();
            if grid.is_near_bottom(grid.cached_visible_rows) && !loading_more {
                if tab == UpTab::Videos && grid.cards.len() < video_total as usize {
                    return Some(AppAction::LoadMoreUpVideos);
                }
                if tab == UpTab::Favorites && favorite_has_more {
                    return Some(AppAction::LoadMoreFavoriteResources);
                }
            }
        } else if keys.matches_up(key) {
            grid.move_up();
        } else if keys.matches_right(key) {
            grid.move_right();
        } else if keys.matches_left(key) {
            grid.move_left();
        } else if keys.matches_confirm(key)
            && let Some(card) = grid.selected_card()
            && let (Some(bvid), Some(aid)) = (&card.bvid, card.aid)
        {
            return Some(AppAction::OpenVideoDetail(bvid.clone(), aid));
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
    fn play_order_cycles_through_shuffle() {
        let mut page = UpPage::new(1);
        let keys = Keybindings::default();
        page.handle_input(KeyCode::Char('s'), &keys);
        assert_eq!(page.play_order, PlayOrder::Reverse);
        page.handle_input(KeyCode::Char('s'), &keys);
        assert_eq!(page.play_order, PlayOrder::Shuffle);
        page.handle_input(KeyCode::Char('s'), &keys);
        assert_eq!(page.play_order, PlayOrder::Forward);
    }

    #[test]
    fn back_cancels_a_pending_folder_without_leaving_up_page() {
        let mut page = UpPage::new_favorites(1);
        page.pending_folder = Some(10);
        page.loading = true;
        let action = page.handle_input(KeyCode::Esc, &Keybindings::default());
        assert!(matches!(action, Some(AppAction::None)));
        assert_eq!(page.pending_folder, None);
        assert!(!page.loading);
    }

    #[test]
    fn changing_favorite_order_requests_the_pending_folder_again() {
        let mut page = UpPage::new_favorites(1);
        page.pending_folder = Some(10);
        page.loading = true;
        let action = page.handle_input(KeyCode::Char('o'), &Keybindings::default());
        assert!(matches!(action, Some(AppAction::OpenFavoriteFolder(10))));
        assert_eq!(page.pending_folder, Some(10));
        assert!(page.loading);
    }
}

#[cfg(test)]
mod sidebar_navigation_tests {
    use super::*;

    #[test]
    fn nested_up_page_preserves_detail_tab_and_back_behavior() {
        let keys = Keybindings::default();
        let mut page = UpPage::new(1);
        assert!(matches!(
            page.handle_input(KeyCode::Tab, &keys),
            Some(AppAction::None)
        ));
        assert!(matches!(
            page.handle_input(KeyCode::Esc, &keys),
            Some(AppAction::BackToList)
        ));
    }

    #[test]
    fn own_space_folder_escape_stays_on_sidebar_page_before_leaving() {
        let keys = Keybindings::default();
        let mut page = UpPage::new_sidebar(1);
        page.tab = UpTab::Favorites;
        page.pending_folder = Some(2);
        assert!(matches!(
            page.handle_input(KeyCode::Esc, &keys),
            Some(AppAction::None)
        ));
        assert!(!page.loading);
        assert!(matches!(
            page.handle_input(KeyCode::Esc, &keys),
            Some(AppAction::NavPrev)
        ));
    }
}
