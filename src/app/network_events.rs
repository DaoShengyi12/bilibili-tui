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
        loop {
            match self.network_event_rx.try_recv() {
                Ok(event) => self.handle_network_event(event),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.network_worker_lost.set(true);
                    break;
                }
            }
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
                    page.loading_more_comments = false;
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
                    self.start_playlist(items, source, start_index, order);
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
            network::NetworkEvent::VideoCommentsLoaded {
                req_id,
                aid,
                page,
                replies,
            } => {
                if !self.is_latest_request("video_comments", req_id) {
                    return;
                }
                if let Some(detail) = find_page!(self, VideoDetail(detail) if detail.aid == aid) {
                    detail.apply_more_comments(page, replies);
                }
            }
            network::NetworkEvent::DynamicCommentsLoaded {
                req_id,
                dynamic_id,
                page,
                replies,
            } => {
                if !self.is_latest_request("dynamic_comments", req_id) {
                    return;
                }
                if let Some(detail) =
                    find_page!(self, DynamicDetail(detail) if detail.dynamic_id == dynamic_id)
                {
                    detail.apply_more_comments(page, replies);
                }
            }
            network::NetworkEvent::CommentRepliesLoaded {
                req_id,
                root,
                replies,
            } => {
                if !self.is_latest_request("comment_replies", req_id) {
                    return;
                }
                if let Some(page) =
                    find_page!(self, VideoDetail(page) if page.expanded_comment == Some(root))
                {
                    page.apply_comment_replies(root, Some(replies));
                }
            }
            network::NetworkEvent::CommentLiked { oid, rpid, liked } => {
                self.apply_comment_like(oid, rpid, liked);
            }
            network::NetworkEvent::CommentPosted { oid } => self.reload_comments_after_post(oid),
            network::NetworkEvent::CommentPostFailed {
                oid,
                message,
                error,
            } => self.restore_comment_draft(oid, message, format!("评论发送失败：{error}")),
            network::NetworkEvent::LiveDetailLoaded {
                req_id,
                room_id,
                room_info,
                history,
            } => {
                if !self.is_latest_request("live_detail", req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, LiveDetail(page) if page.room_id() == room_id)
                {
                    page.apply_room_info(room_info);
                    if let Some(history) = history {
                        page.apply_history_danmaku(history);
                    }
                }
            }
            network::NetworkEvent::LiveDanmakuConnected {
                req_id,
                room_id,
                hub,
            } => {
                if !self.is_latest_request("live_danmaku", req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, LiveDetail(page) if page.room_id() == room_id)
                {
                    page.attach_danmaku_hub(std::sync::Arc::clone(&hub));
                }
                self.live_danmaku_hub = Some(std::sync::Arc::downgrade(&hub));
            }
            network::NetworkEvent::QrcodeLoaded { req_id, data } => {
                if !self.is_latest_request("login_qrcode", req_id) {
                    return;
                }
                if let Page::Login(page) = &mut self.current_page {
                    page.apply_qrcode(data);
                }
            }
            network::NetworkEvent::QrcodePolled {
                req_id,
                status,
                credentials,
            } => {
                if !self.is_latest_request("login_poll", req_id) {
                    return;
                }
                if let Page::Login(page) = &mut self.current_page
                    && let Some(action) = page.apply_poll(status, credentials)
                {
                    self.deferred_actions.push(action);
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

    fn apply_comment_like(&mut self, oid: i64, rpid: i64, liked: bool) {
        for page in std::iter::once(&mut self.current_page).chain(self.navigation_stack.iter_mut())
        {
            let liked_comments = match page {
                Page::VideoDetail(page) if page.aid == oid => &mut page.liked_comments,
                Page::DynamicDetail(page) if page.comments.iter().any(|c| c.rpid == rpid) => {
                    &mut page.liked_comments
                }
                _ => continue,
            };
            if liked {
                liked_comments.insert(rpid);
            } else {
                liked_comments.remove(&rpid);
            }
        }
    }

    /// Reloads the detail page a comment was posted to so the new comment
    /// shows up.
    /// Give an unsent comment back to the page it was written on.
    pub(super) fn restore_comment_draft(&mut self, oid: i64, message: String, notice: String) {
        match &mut self.current_page {
            Page::VideoDetail(page) if page.aid == oid => {
                page.restore_comment_draft(message, notice)
            }
            Page::DynamicDetail(page)
                if page
                    .dynamic_item
                    .as_ref()
                    .and_then(|item| item.comment_oid(&page.dynamic_id))
                    == Some(oid) =>
            {
                page.restore_comment_draft(message, notice)
            }
            _ => {}
        }
    }

    fn reload_comments_after_post(&mut self, oid: i64) {
        match &self.current_page {
            Page::VideoDetail(page) if page.aid == oid => {
                let (bvid, aid) = (page.bvid.clone(), page.aid);
                // The reload replaces the comment list; drop any page in flight.
                self.next_request_id("video_comments");
                let req_id = self.next_request_id("video_detail");
                self.send_network_command(network::NetworkCommand::LoadVideoDetail {
                    req_id,
                    bvid,
                    aid,
                    include_related: !self.config.minimal_mode,
                });
            }
            Page::DynamicDetail(page)
                if page
                    .dynamic_item
                    .as_ref()
                    .and_then(|item| item.comment_oid(&page.dynamic_id))
                    == Some(oid) =>
            {
                let dynamic_id = page.dynamic_id.clone();
                let req_id = self.next_request_id("dynamic_detail");
                self.send_network_command(network::NetworkCommand::LoadDynamicDetail {
                    req_id,
                    dynamic_id,
                });
            }
            _ => {}
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
            "video_comments" => {
                if let Some(page) =
                    find_page!(self, VideoDetail(page) if page.loading_more_comments)
                {
                    page.apply_more_comments_error();
                }
            }
            "dynamic_comments" => {
                if let Some(page) =
                    find_page!(self, DynamicDetail(page) if page.loading_more_comments)
                {
                    page.apply_more_comments_error();
                }
            }
            "comment_replies" => {
                if let Some(page) = find_page!(self, VideoDetail(page) if page.loading_replies)
                    && let Some(root) = page.expanded_comment
                {
                    page.apply_comment_replies(root, None);
                }
            }
            "live_detail" => {
                if let Some(page) = find_page!(self, LiveDetail(page)) {
                    page.apply_room_info_error(format!("加载房间信息失败: {error}"));
                }
            }
            "live_danmaku" => {
                if let Some(page) = find_page!(self, LiveDetail(page)) {
                    page.set_ws_error(format!("WS连接失败: {error}"));
                }
            }
            "login_qrcode" => {
                if let Page::Login(page) = &mut self.current_page {
                    page.apply_qrcode_error(format!("获取二维码失败: {error}"));
                }
            }
            "login_poll" => {
                if let Page::Login(page) = &mut self.current_page {
                    page.apply_poll_error(format!("轮询失败: {error}"));
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
    async fn failed_comment_post_reopens_the_draft_with_a_notice() {
        let (mut app, _rx) = app();
        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1test".into(), 9);
        page.loading = false;
        app.current_page = Page::VideoDetail(Box::new(page));

        app.handle_network_event(NetworkEvent::CommentPostFailed {
            oid: 9,
            message: "草稿".into(),
            error: "API error (12015): 需要评论验证码".into(),
        });

        let Page::VideoDetail(page) = &app.current_page else {
            panic!("expected video detail");
        };
        assert!(page.input_mode);
        assert_eq!(page.input_buffer, "草稿");
        assert!(page.notice.as_deref().unwrap().contains("12015"));
        assert!(page.error_message.is_none());
    }

    #[tokio::test]
    async fn login_hint_is_a_footer_notice_that_keeps_the_detail_content() {
        let (mut app, _rx) = app();
        app.credentials = None;
        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1test".into(), 9);
        page.loading = false;
        app.current_page = Page::VideoDetail(Box::new(page));

        app.handle_action(AppAction::AddComment {
            oid: 9,
            comment_type: 1,
            message: "草稿".into(),
            root: None,
        })
        .await;

        let Page::VideoDetail(page) = &mut app.current_page else {
            panic!("expected video detail");
        };
        assert!(page.error_message.is_none());
        assert_eq!(page.input_buffer, "草稿");
        assert!(page.notice.is_some());
        let keys = crate::storage::Keybindings::default();
        crate::presentation::tui::Component::handle_input(
            page.as_mut(),
            crossterm::event::KeyCode::Char('x'),
            &keys,
        );
        assert!(page.notice.is_none(), "the next key clears the notice");
    }

    #[tokio::test]
    async fn superseded_video_detail_reloads_when_shown_again() {
        let (mut app, rx) = app();
        app.handle_action(AppAction::OpenVideoDetail("BVa".into(), 1))
            .await;
        app.handle_action(AppAction::OpenVideoDetail("BVb".into(), 2))
            .await;
        // B superseded A's request; the worker reports A as cancelled.
        rx.try_iter().for_each(drop);

        app.handle_action(AppAction::BackToList).await;

        let Page::VideoDetail(page) = &app.current_page else {
            panic!("expected video A");
        };
        assert_eq!(page.bvid, "BVa");
        let reload = rx.try_iter().find_map(|command| match command {
            NetworkCommand::LoadVideoDetail { req_id, bvid, .. } => Some((req_id, bvid)),
            _ => None,
        });
        let (req_id, bvid) = reload.expect("A is requested again");
        assert_eq!(bvid, "BVa");
        assert!(app.is_latest_request("video_detail", req_id));
    }

    #[tokio::test]
    async fn loaded_page_is_not_requested_again_when_shown() {
        let (mut app, rx) = app();
        app.handle_action(AppAction::OpenVideoDetail("BVa".into(), 1))
            .await;
        if let Page::VideoDetail(page) = &mut app.current_page {
            page.loading = false;
        }
        app.handle_action(AppAction::OpenUpPage(5)).await;
        rx.try_iter().for_each(drop);

        app.handle_action(AppAction::BackToList).await;

        assert!(
            rx.try_iter()
                .all(|command| !matches!(command, NetworkCommand::LoadVideoDetail { .. }))
        );
    }

    #[tokio::test]
    async fn back_from_dynamic_detail_restores_the_list_without_reloading() {
        let (mut app, rx) = app();
        app.sidebar
            .select(crate::presentation::tui::NavItem::Dynamic);
        app.current_page = Page::Dynamic(crate::presentation::tui::DynamicPage::new());

        app.handle_action(AppAction::OpenDynamicDetail("1".into()))
            .await;
        assert!(matches!(app.current_page, Page::DynamicDetail(_)));
        rx.try_iter().for_each(drop);

        app.handle_action(AppAction::BackToList).await;

        assert!(matches!(app.current_page, Page::Dynamic(_)));
        assert!(app.navigation_stack.is_empty());
        assert!(
            rx.try_iter()
                .all(|command| !matches!(command, NetworkCommand::LoadDynamicInit { .. })),
            "the restored dynamic page must keep its loaded feed"
        );
    }

    #[tokio::test]
    async fn back_from_live_and_bangumi_details_returns_to_the_list() {
        let (mut app, _rx) = app();
        app.current_page = Page::Live(crate::presentation::tui::LivePage::new());
        app.handle_action(AppAction::OpenLiveDetail(42)).await;
        app.handle_action(AppAction::BackToList).await;
        assert!(matches!(app.current_page, Page::Live(_)));

        app.current_page = Page::Bangumi(Box::default());
        app.handle_action(AppAction::OpenBangumiDetail(7)).await;
        app.handle_action(AppAction::BackToList).await;
        assert!(matches!(app.current_page, Page::Bangumi(_)));
        assert!(app.navigation_stack.is_empty());
    }

    #[tokio::test]
    async fn sidebar_navigation_from_a_detail_drops_the_back_stack() {
        let (mut app, _rx) = app();
        app.sidebar
            .select(crate::presentation::tui::NavItem::Bangumi);
        app.current_page = Page::Bangumi(Box::default());
        app.handle_action(AppAction::OpenBangumiDetail(7)).await;

        app.handle_action(AppAction::NavNext).await;

        assert!(app.navigation_stack.is_empty());
        assert!(
            app.cached_bangumi.is_some(),
            "the list page moves to its tab cache"
        );
        assert!(!matches!(app.current_page, Page::BangumiDetail(_)));
    }

    #[tokio::test]
    async fn account_change_clears_navigation_caches_and_danmaku() {
        let (mut app, rx) = app();
        app.navigation_stack
            .push(Page::Live(crate::presentation::tui::LivePage::new()));
        app.cached_home = Some(crate::presentation::tui::HomePage::new());
        app.cached_bangumi = Some(crate::presentation::tui::BangumiPage::new());
        app.sidebar.select(crate::presentation::tui::NavItem::Live);
        app.live_danmaku_hub = Some(std::sync::Weak::new());

        app.reset_account_state();

        assert!(app.navigation_stack.is_empty());
        assert!(app.cached_home.is_none() && app.cached_bangumi.is_none());
        assert!(app.live_danmaku_hub.is_none());
        assert_eq!(
            app.sidebar.selected,
            crate::presentation::tui::NavItem::Home
        );
        assert!(matches!(rx.try_recv(), Ok(NetworkCommand::CancelPending)));
    }

    #[tokio::test]
    async fn opening_a_live_room_returns_before_the_room_loads() {
        let (mut app, rx) = app();
        app.live_danmaku_hub = None;
        app.current_page = Page::Live(crate::presentation::tui::LivePage::new());

        app.handle_action(AppAction::OpenLiveDetail(42)).await;

        assert!(matches!(&app.current_page, Page::LiveDetail(page) if page.room_id() == 42));
        let commands: Vec<_> = rx.try_iter().collect();
        assert!(commands.iter().any(|command| matches!(
            command,
            NetworkCommand::ConnectLiveDanmaku { room_id: 42, .. }
        )));
        let Some(NetworkCommand::LoadLiveDetail { req_id, .. }) = commands
            .into_iter()
            .find(|command| matches!(command, NetworkCommand::LoadLiveDetail { .. }))
        else {
            panic!("live room details were not requested");
        };

        app.handle_network_event(NetworkEvent::RequestFailed {
            req_id,
            target: "live_detail",
            error: "timeout".into(),
        });
        assert!(matches!(&app.current_page, Page::LiveDetail(page) if !page.is_loading()));
    }

    #[tokio::test]
    async fn more_comments_load_in_the_background() {
        let (mut app, rx) = app();
        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1test".into(), 7);
        page.has_more_comments = true;
        app.current_page = Page::VideoDetail(Box::new(page));

        app.handle_action(AppAction::LoadMoreComments).await;
        let Ok(NetworkCommand::LoadVideoComments {
            req_id,
            aid: 7,
            page: 2,
        }) = rx.try_recv()
        else {
            panic!("comments were not requested");
        };
        assert!(matches!(&app.current_page, Page::VideoDetail(page) if page.loading_more_comments));

        app.handle_network_event(NetworkEvent::RequestCancelled {
            req_id,
            target: "video_comments",
        });
        assert!(
            matches!(&app.current_page, Page::VideoDetail(page) if !page.loading_more_comments)
        );
    }

    #[tokio::test]
    async fn play_requests_are_ignored_while_a_player_is_starting() {
        let (mut app, _rx) = app();
        app.playback.begin_starting(9);

        app.handle_action(AppAction::PlayVideo {
            bvid: "BV1test".into(),
            aid: 1,
            cid: 1,
            duration: 10,
        })
        .await;

        assert_eq!(app.playback.session_id, Some(9));
    }

    #[tokio::test]
    async fn successful_qrcode_poll_logs_in_on_the_next_tick() {
        let (mut app, _rx) = app();
        app.credentials = None;
        app.current_page = Page::Login(crate::presentation::tui::LoginPage::new());
        let req_id = app.next_request_id("login_poll");
        let credentials = crate::infrastructure::persistence::Credentials {
            sessdata: String::new(),
            bili_jct: String::new(),
            dede_user_id: "1".into(),
            dede_user_id_ckmd5: None,
            refresh_token: None,
        };

        app.handle_network_event(NetworkEvent::QrcodePolled {
            req_id,
            status: Some(crate::api::auth::QrcodePollStatus::Success),
            credentials: Some(credentials),
        });

        assert!(matches!(
            app.deferred_actions.as_slice(),
            [AppAction::LoginSuccess(_)]
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
