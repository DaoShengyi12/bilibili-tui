use crate::app::App;
use crate::application::network;
use crate::presentation::tui::{BangumiPage, HomePage, Page, SearchPage, VideoCard};

/// Error text shown when the worker dropped a request before it finished.
const REQUEST_CANCELLED: &str = "请求已取消";

/// Finds the newest page of a variant whether it is on screen or waiting in
/// the back stack, so results still reach pages the user navigated away from.
macro_rules! find_page {
    ($app:expr, $variant:ident($page:ident) $(if $guard:expr)?) => {
        std::iter::once(&mut $app.current_page)
            .chain($app.navigation_stack.iter_mut().rev())
            .find_map(|page| match page {
                Page::$variant($page) $(if $guard)? => Some($page),
                _ => None,
            })
    };
}

impl App {
    fn home_page_mut(&mut self) -> Option<&mut HomePage> {
        find_page!(self, Home(page)).or(self.cached_home.as_mut())
    }

    /// The standalone search page or the search pane inside Home.
    fn search_page_mut(&mut self) -> Option<&mut SearchPage> {
        std::iter::once(&mut self.current_page)
            .chain(self.navigation_stack.iter_mut().rev())
            .find_map(|page| match page {
                Page::Search(page) => Some(page),
                Page::Home(home) => Some(home.search_mut()),
                _ => None,
            })
            .or(self.cached_home.as_mut().map(HomePage::search_mut))
    }

    fn bangumi_page_mut(&mut self) -> Option<&mut BangumiPage> {
        find_page!(self, Bangumi(page))
            .map(|page| &mut **page)
            .or(self.cached_bangumi.as_mut())
    }

    pub(super) fn drain_network_events(&mut self) {
        while let Ok(event) = self.network_event_rx.try_recv() {
            self.handle_network_event(event);
        }
    }

    fn handle_network_event(&mut self, event: network::NetworkEvent) {
        match event {
            network::NetworkEvent::HomeLoaded {
                req_id,
                feed,
                videos,
            } => {
                if !self.is_latest_request("home", req_id) {
                    return;
                }
                if let Some(page) = self.home_page_mut() {
                    page.apply_recommendations(feed, videos);
                }
            }
            network::NetworkEvent::HomeMoreLoaded {
                req_id,
                feed,
                videos,
            } => {
                if !self.is_latest_request("home_more", req_id) {
                    return;
                }
                if let Some(page) = self.home_page_mut() {
                    page.apply_load_more(feed, videos);
                }
            }
            network::NetworkEvent::HotwordsLoaded { req_id, hotwords } => {
                if self.config.minimal_mode || !self.is_latest_request("hotwords", req_id) {
                    return;
                }
                if let Some(page) = self.search_page_mut() {
                    page.set_hotwords(hotwords);
                }
            }
            network::NetworkEvent::SearchLoaded {
                req_id,
                keyword,
                page,
                results,
                total,
            } => {
                if !self.is_latest_request("search", req_id) {
                    return;
                }
                let Some(search_page) = self.search_page_mut() else {
                    return;
                };
                if search_page.query != keyword {
                    return;
                }
                if page <= 1 {
                    search_page.page = 1;
                    search_page.set_results(results, total);
                } else {
                    search_page.page = page;
                    search_page.total_results = total;
                    search_page.append_results(results);
                }
            }
            network::NetworkEvent::DynamicLoaded {
                req_id,
                append,
                up_list,
                items,
                offset,
                has_more,
            } => {
                let key = if append {
                    "dynamic_more"
                } else {
                    "dynamic_feed"
                };
                if !self.is_latest_request(key, req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, Dynamic(page)) {
                    if let Some(up_list) = up_list {
                        page.set_up_list(up_list);
                    }
                    if append {
                        page.append_feed(items, offset, has_more);
                        page.loading_more = false;
                    } else {
                        page.set_feed(items, offset, has_more);
                    }
                }
            }
            network::NetworkEvent::HistoryLoaded {
                req_id,
                append,
                data,
            } => {
                let key = if append {
                    "history_more"
                } else {
                    "history_init"
                };
                if !self.is_latest_request(key, req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, History(page)) {
                    if append {
                        page.apply_history_more(data);
                    } else {
                        page.apply_history_init(data);
                    }
                }
            }
            network::NetworkEvent::HistoryDeleted {
                req_id,
                successful,
                failed,
            } => {
                if !self.is_latest_request("history_delete", req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, History(page)) {
                    page.apply_delete_result(successful, failed);
                }
            }
            network::NetworkEvent::ArticleLoaded {
                req_id,
                cvid,
                article,
                comments,
            } => {
                if !self.is_latest_request("article_detail", req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, ArticleDetail(page) if page.cvid == cvid) {
                    page.set_article(article, comments);
                }
            }
            network::NetworkEvent::LiveLoaded {
                req_id,
                append,
                rooms,
            } => {
                let key = if append { "live_more" } else { "live_init" };
                if !self.is_latest_request(key, req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, Live(page)) {
                    if append {
                        page.apply_live_more(rooms);
                    } else {
                        page.apply_live_init(rooms);
                    }
                }
            }
            network::NetworkEvent::VideoDetailLoaded {
                req_id,
                bvid,
                video_info,
                comments,
                has_more_comments,
                mut related_videos,
            } => {
                if self.config.minimal_mode {
                    related_videos.clear();
                }
                if !self.is_latest_request("video_detail", req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, VideoDetail(page) if page.bvid == bvid) {
                    page.video_info = Some(video_info);
                    page.comments = comments;
                    page.comment_page = 1;
                    page.has_more_comments = has_more_comments;
                    page.related_videos = related_videos.clone();
                    page.related_card_grid.clear();
                    for video in &related_videos {
                        let card = VideoCard::new(
                            video.bvid.clone(),
                            video.aid,
                            video.title.clone().unwrap_or_else(|| "无标题".to_string()),
                            video.author_name().to_string(),
                            video.format_views(),
                            video.format_duration(),
                            video.cover_url(),
                        )
                        .with_uploader_mid(video.owner.as_ref().and_then(|owner| owner.mid));
                        page.related_card_grid.add_card(card);
                    }
                    page.loading = false;
                    page.error_message = None;
                }
            }
            network::NetworkEvent::UpPageLoaded {
                req_id,
                mid,
                order,
                profile,
                relation,
                videos,
                folders,
            } => {
                if !self.is_latest_request("up_page", req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, Up(page) if page.mid == mid) {
                    if page.video_order == order {
                        page.apply_initial(profile, relation, videos, folders);
                    } else {
                        // The order changed while loading; the video list
                        // comes from the newer LoadUpVideos request.
                        page.apply_profile(profile, relation, folders);
                    }
                }
            }
            network::NetworkEvent::UpVideosLoaded {
                req_id,
                mid,
                page: loaded_page,
                order,
                videos,
            } => {
                if !self.is_latest_request("up_videos", req_id) {
                    return;
                }
                if let Some(page) =
                    find_page!(self, Up(page) if page.mid == mid && page.video_order == order)
                {
                    if loaded_page == 1 {
                        page.videos.clear();
                    }
                    page.apply_more_videos(loaded_page, videos);
                    page.loading = false;
                }
            }
            network::NetworkEvent::FavoriteResourcesLoaded {
                req_id,
                owner_mid,
                media_id,
                page: loaded_page,
                order,
                resources,
            } => {
                if !self.is_latest_request("favorite_resources", req_id) {
                    return;
                }
                if let Some(page) = find_page!(
                    self,
                    Up(page) if page.mid == owner_mid
                        && page.favorite_order == order
                        && (page.pending_folder == Some(media_id)
                            || page.active_folder == Some(media_id))
                ) {
                    page.apply_favorite_resources(media_id, loaded_page, resources);
                }
            }
            network::NetworkEvent::PlaylistLoaded {
                req_id,
                items,
                source,
                start_index,
                order,
            } => {
                if self.is_latest_request("playlist_build", req_id) {
                    self.pending_playlist = Some((items, source, start_index, order));
                }
            }
            network::NetworkEvent::FavoritesInitLoaded {
                req_id,
                mid,
                watch_later,
                created,
                collected,
            } => {
                if !self.is_latest_request("favorites_init", req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, Favorites(page) if page.mid == mid) {
                    page.apply_initial(watch_later, created, collected);
                }
            }
            network::NetworkEvent::FavoritesWatchLaterLoaded { req_id, page, data } => {
                if !self.is_latest_request("favorites_content", req_id) {
                    return;
                }
                if let Some(favorites) = find_page!(
                    self,
                    Favorites(page) if matches!(
                        page.active_source,
                        crate::api::favorite::FavoriteSource::WatchLater
                    )
                ) {
                    favorites.apply_watch_later(page, data);
                }
            }
            network::NetworkEvent::FavoritesCreatedLoaded {
                req_id,
                media_id,
                page,
                data,
            } => {
                if !self.is_latest_request("favorites_content", req_id) {
                    return;
                }
                if let Some(favorites) = find_page!(
                    self,
                    Favorites(page) if matches!(
                        page.active_source,
                        crate::api::favorite::FavoriteSource::Created {
                            media_id: active_id,
                            ..
                        } if active_id == media_id
                    )
                ) {
                    favorites.apply_created(page, data);
                }
            }
            network::NetworkEvent::FavoritesCollectedLoaded {
                req_id,
                season_id,
                page,
                data,
            } => {
                if !self.is_latest_request("favorites_content", req_id) {
                    return;
                }
                if let Some(favorites) = find_page!(
                    self,
                    Favorites(page) if matches!(
                        page.active_source,
                        crate::api::favorite::FavoriteSource::Collected {
                            season_id: active_id,
                            ..
                        } if active_id == season_id
                    )
                ) {
                    favorites.apply_collected(page, data);
                }
            }
            network::NetworkEvent::DynamicDetailLoaded {
                req_id,
                dynamic_id,
                dynamic_item,
                comments,
                has_more_comments,
                image_urls,
            } => {
                if !self.is_latest_request("dynamic_detail", req_id) {
                    return;
                }
                if let Some(page) =
                    find_page!(self, DynamicDetail(page) if page.dynamic_id == dynamic_id)
                {
                    page.dynamic_item = Some(dynamic_item);
                    page.comments = comments;
                    page.comment_page = 1;
                    page.has_more_comments = has_more_comments;
                    page.image_urls = image_urls;
                    page.image_protocols = (0..page.image_urls.len()).map(|_| None).collect();
                    page.loading = false;
                    page.error_message = None;
                }
            }
            network::NetworkEvent::BangumiIndexLoaded { req_id, items } => {
                if !self.is_latest_request("bangumi_index", req_id) {
                    return;
                }
                if let Some(page) = self.bangumi_page_mut() {
                    page.set_index_items(items);
                }
            }
            network::NetworkEvent::BangumiDetailLoaded {
                req_id,
                season_id,
                season,
            } => {
                if !self.is_latest_request("bangumi_detail", req_id) {
                    return;
                }
                if let Some(page) =
                    find_page!(self, BangumiDetail(page) if page.season_id == season_id)
                {
                    page.set_season(season);
                }
            }
            network::NetworkEvent::RequestFailed {
                req_id,
                target,
                error,
            } => {
                if self.is_latest_request(target, req_id) {
                    self.apply_request_failure(target, &error);
                }
            }
            network::NetworkEvent::RequestCancelled { req_id, target } => {
                if self.is_latest_request(target, req_id) {
                    self.apply_request_failure(target, REQUEST_CANCELLED);
                }
            }
        }
    }

    /// Takes the page that owns `target` out of its loading state, wherever
    /// that page currently lives.
    fn apply_request_failure(&mut self, target: &'static str, error: &str) {
        match target {
            "home" => {
                if let Some(page) = self.home_page_mut() {
                    page.apply_recommendations_error(format!("加载推荐视频失败: {error}"));
                }
            }
            "home_more" => {
                if let Some(page) = self.home_page_mut() {
                    page.apply_load_more_error();
                }
            }
            "hotwords" => {
                if let Some(page) = self.search_page_mut() {
                    page.set_hotword_error(format!("加载热搜失败: {error}"));
                }
            }
            "search" => {
                if let Some(page) = self.search_page_mut() {
                    page.set_error(format!("搜索失败: {error}"));
                }
            }
            "dynamic_feed" | "dynamic_more" => {
                if let Some(page) = find_page!(self, Dynamic(page)) {
                    page.loading_up_list = false;
                    page.set_error(format!("加载动态失败: {error}"));
                }
            }
            "history_init" | "history_more" => {
                if let Some(page) = find_page!(self, History(page)) {
                    let prefix = if target == "history_init" {
                        "加载历史记录失败"
                    } else {
                        "加载更多失败"
                    };
                    page.apply_load_more_error(format!("{prefix}: {error}"));
                }
            }
            "live_init" => {
                if let Some(page) = find_page!(self, Live(page)) {
                    page.apply_live_init_error(format!("加载直播推荐失败: {error}"));
                }
            }
            "live_more" => {
                if let Some(page) = find_page!(self, Live(page)) {
                    page.apply_live_more_error();
                }
            }
            "video_detail" => {
                if let Some(page) = find_page!(self, VideoDetail(page) if page.loading) {
                    page.error_message = Some(format!("加载视频信息失败: {error}"));
                    page.loading = false;
                }
            }
            "up_page" | "up_videos" => {
                if let Some(page) = find_page!(self, Up(page)) {
                    page.set_error(format!("加载UP主空间失败: {error}"));
                }
            }
            "favorite_resources" => {
                if let Some(page) = find_page!(self, Up(page)) {
                    page.set_error(format!("加载收藏夹失败: {error}"));
                }
            }
            "playlist_build" => {
                self.playback.status = crate::domain::playback::PlaybackStatus::Failed;
                self.playback.last_error = Some(format!("加载播放列表失败: {error}"));
            }
            "favorites_init" | "favorites_content" => {
                if let Some(page) = find_page!(self, Favorites(page)) {
                    page.set_error(format!("加载收藏失败: {error}"));
                }
            }
            "dynamic_detail" => {
                if let Some(page) = find_page!(self, DynamicDetail(page) if page.loading) {
                    page.error_message = Some(format!("加载动态详情失败: {error}"));
                    page.loading = false;
                }
            }
            "article_detail" => {
                if let Some(page) = find_page!(self, ArticleDetail(page)) {
                    page.set_error(format!("加载专栏失败: {error}"));
                }
            }
            "bangumi_index" => {
                if let Some(page) = self.bangumi_page_mut() {
                    page.set_error(format!("加载番剧索引失败: {error}"));
                }
            }
            "bangumi_detail" => {
                if let Some(page) = find_page!(self, BangumiDetail(page)) {
                    page.set_error(format!("加载番剧详情失败: {error}"));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::application::{AppAction, network::NetworkCommand, network::NetworkEvent};
    use crate::presentation::tui::{DynamicPage, DynamicTab, HistoryPage, HomePage, Page};
    use std::sync::mpsc;

    fn app() -> (App, mpsc::Receiver<NetworkCommand>) {
        let mut app = App::new();
        app.config.minimal_mode = false;
        app.sidebar.set_minimal_mode(false);
        app.credentials = Some(crate::infrastructure::persistence::Credentials {
            sessdata: String::new(),
            bili_jct: String::new(),
            dede_user_id: "1".into(),
            dede_user_id_ckmd5: None,
            refresh_token: None,
        });
        let (tx, rx) = mpsc::channel();
        app.network_command_tx = tx;
        (app, rx)
    }

    fn loaded_home() -> HomePage {
        let mut page = HomePage::new();
        page.apply_recommendations(page.feed(), Vec::new());
        page
    }

    #[tokio::test]
    async fn load_more_result_reaches_home_page_behind_video_detail() {
        let (mut app, _rx) = app();
        let mut home = loaded_home();
        home.begin_load_more().unwrap();
        let feed = home.feed();
        app.current_page = Page::Home(home);
        let req_id = app.next_request_id("home_more");
        app.handle_action(AppAction::OpenVideoDetail("BV1test".into(), 1))
            .await;

        app.handle_network_event(NetworkEvent::HomeMoreLoaded {
            req_id,
            feed,
            videos: Vec::new(),
        });
        app.handle_action(AppAction::BackToList).await;

        assert!(matches!(&app.current_page, Page::Home(page) if !page.is_loading()));
    }

    #[tokio::test]
    async fn failed_history_page_in_back_stack_leaves_loading() {
        let (mut app, _rx) = app();
        let mut history = HistoryPage::new();
        history.begin_loading();
        app.navigation_stack.push(Page::History(history));
        let req_id = app.next_request_id("history_more");

        app.handle_network_event(NetworkEvent::RequestFailed {
            req_id,
            target: "history_more",
            error: "timeout".into(),
        });
        app.handle_action(AppAction::BackToList).await;

        assert!(matches!(&app.current_page, Page::History(page) if !page.is_loading()));
    }

    #[test]
    fn cancelled_hotwords_leave_loading_on_cached_home() {
        let (mut app, _rx) = app();
        let mut home = loaded_home();
        home.search_mut().start_hotword_loading();
        app.cached_home = Some(home);
        app.current_page = Page::Dynamic(DynamicPage::new());
        let req_id = app.next_request_id("hotwords");

        app.handle_network_event(NetworkEvent::RequestCancelled {
            req_id,
            target: "hotwords",
        });

        let search = app.cached_home.as_mut().unwrap().search_mut();
        assert!(!search.hotword_loading);
        assert!(search.hotword_error.is_some());
    }

    #[test]
    fn cancellation_of_a_superseded_request_is_ignored() {
        let (mut app, _rx) = app();
        let mut home = loaded_home();
        let stale = app.next_request_id("home_more");
        home.begin_load_more().unwrap();
        app.current_page = Page::Home(home);
        app.next_request_id("home_more");

        app.handle_network_event(NetworkEvent::RequestCancelled {
            req_id: stale,
            target: "home_more",
        });

        assert!(matches!(&app.current_page, Page::Home(page) if page.is_loading()));
    }

    #[tokio::test]
    async fn cached_home_without_videos_reloads_when_shown() {
        let (mut app, rx) = app();
        let mut home = HomePage::new();
        home.apply_recommendations_error("请求已取消".into());
        app.cached_home = Some(home);

        app.handle_action(AppAction::SwitchToHome).await;

        assert!(matches!(&app.current_page, Page::Home(page) if page.is_loading()));
        assert!(matches!(rx.try_recv(), Ok(NetworkCommand::LoadHome { .. })));
    }

    #[tokio::test]
    async fn switching_dynamic_tab_drops_pending_load_more() {
        let (mut app, rx) = app();
        let mut page = DynamicPage::new();
        page.loading = false;
        page.loading_more = true;
        app.current_page = Page::Dynamic(page);
        let more = app.next_request_id("dynamic_more");

        app.handle_action(AppAction::SwitchDynamicTab(DynamicTab::Videos))
            .await;

        assert!(!app.is_latest_request("dynamic_more", more));
        assert!(matches!(&app.current_page, Page::Dynamic(page) if !page.loading_more));
        assert!(matches!(
            rx.try_recv(),
            Ok(NetworkCommand::LoadDynamicRefresh {
                tab: DynamicTab::Videos,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn dynamic_refresh_before_up_list_arrives_supersedes_init() {
        let (mut app, rx) = app();
        let mut page = DynamicPage::new();
        page.loading_up_list = true;
        app.current_page = Page::Dynamic(page);
        let init = app.next_request_id("dynamic_feed");

        app.handle_action(AppAction::RefreshDynamic).await;

        assert!(!app.is_latest_request("dynamic_feed", init));
        assert!(matches!(
            rx.try_recv(),
            Ok(NetworkCommand::LoadDynamicInit { .. })
        ));
    }

    #[tokio::test]
    async fn live_load_more_waits_for_refresh() {
        let (mut app, rx) = app();
        let mut page = crate::presentation::tui::LivePage::new();
        page.apply_live_init(Vec::new());
        app.current_page = Page::Live(page);

        app.handle_action(AppAction::LoadMoreLive).await;
        let more = app.next_request_id("live_more") - 1;
        assert!(matches!(
            rx.try_recv(),
            Ok(NetworkCommand::LoadLiveMore { .. })
        ));
        app.handle_action(AppAction::RefreshLive).await;
        app.handle_action(AppAction::LoadMoreLive).await;

        assert!(!app.is_latest_request("live_more", more));
        assert!(matches!(
            rx.try_recv(),
            Ok(NetworkCommand::LoadLiveInit { .. })
        ));
        assert!(rx.try_recv().is_err());
    }
}
