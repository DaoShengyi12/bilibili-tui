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
                kind,
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
                if search_page.query != keyword || search_page.kind != kind {
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
            network::NetworkEvent::SearchSuggestionsLoaded {
                req_id,
                term,
                suggestions,
            } => {
                if !self.is_latest_request("search_suggest", req_id) {
                    return;
                }
                if let Some(page) = self.search_page_mut() {
                    page.apply_suggestions(&term, suggestions);
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
                relation,
                comments,
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
                    page.relation = relation;
                    page.comments.reset_sort();
                    page.comments.set_comments(comments);
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
                image_urls,
            } => {
                if !self.is_latest_request("dynamic_detail", req_id) {
                    return;
                }
                if let Some(page) =
                    find_page!(self, DynamicDetail(page) if page.dynamic_id == dynamic_id)
                {
                    page.dynamic_item = Some(dynamic_item);
                    page.comments.reset_sort();
                    page.comments.set_comments(comments);
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
            network::NetworkEvent::CommentsLoaded {
                req_id,
                target,
                sort,
                append,
                batch,
            } => {
                if !self.is_latest_request("comments", req_id) {
                    return;
                }
                if let Some(panel) = self.comment_panel_mut(target) {
                    panel.apply_batch(sort, append, batch);
                }
            }
            network::NetworkEvent::CommentRepliesLoaded {
                req_id,
                target,
                root,
                page,
                replies,
                has_more,
            } => {
                if !self.is_latest_request("comment_replies", req_id) {
                    return;
                }
                if let Some(panel) = self.comment_panel_mut(target) {
                    panel.apply_replies(root, page, replies, has_more);
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
                login_expired,
            } => {
                if login_expired {
                    self.handle_login_expired();
                }
                if self.is_latest_request(target, req_id) {
                    self.apply_request_failure(target, &error);
                }
            }
            network::NetworkEvent::LiveQualitiesLoaded {
                req_id,
                room_id,
                qualities,
            } => {
                if !self.is_latest_request("live_qualities", req_id) {
                    return;
                }
                if let Some(page) = find_page!(self, LiveDetail(page) if page.room_id() == room_id)
                {
                    page.apply_qualities(qualities);
                }
            }
            network::NetworkEvent::VideoActionDone { request, result } => {
                if let Some(page) = find_page!(self, VideoDetail(page) if page.aid == request.aid())
                {
                    page.apply_video_action(request, result);
                }
            }
            network::NetworkEvent::LiveDanmakuSent { room_id } => {
                if let Page::LiveDetail(page) = &mut self.current_page
                    && page.room_id() == room_id
                {
                    page.apply_danmaku_sent();
                }
            }
            network::NetworkEvent::LiveDanmakuSendFailed {
                room_id,
                message,
                error,
            } => {
                if let Page::LiveDetail(page) = &mut self.current_page
                    && page.room_id() == room_id
                {
                    page.restore_danmaku_draft(message, format!("弹幕发送失败：{error}"));
                } else {
                    self.status_notice = Some(format!("弹幕发送失败：{error}"));
                }
            }
            network::NetworkEvent::SessionRefreshed { credentials } => {
                self.apply_refreshed_session(credentials);
            }
            network::NetworkEvent::SessionRefreshFailed { error, forced } => {
                self.session_refresh_in_flight = false;
                if forced && self.credentials.is_some() {
                    self.login_prompt = Some(format!("登录已失效：{error}"));
                }
            }
            network::NetworkEvent::ServerLogoutFailed { error } => {
                self.status_notice = Some(format!("已在本地退出，但服务端注销失败：{error}"));
            }
            network::NetworkEvent::RequestCancelled { req_id, target } => {
                if self.is_latest_request(target, req_id) {
                    self.apply_request_failure(target, REQUEST_CANCELLED);
                }
            }
        }
    }

    /// A request answered `-101`. Try the refresh token once; when that is
    /// gone or already failed, ask the user to log in again.
    fn handle_login_expired(&mut self) {
        let Some(credentials) = self.credentials.clone() else {
            return;
        };
        if self.session_refresh_in_flight || self.login_prompt.is_some() {
            return;
        }
        if credentials.refresh_token.is_none() || self.session_refresh_forced {
            self.login_prompt = Some("登录已失效".to_string());
            return;
        }
        self.session_refresh_forced = true;
        self.session_refresh_in_flight = true;
        self.send_network_command(network::NetworkCommand::RefreshSession {
            credentials,
            force: true,
        });
    }

    fn apply_refreshed_session(&mut self, credentials: crate::storage::Credentials) {
        self.session_refresh_in_flight = false;
        // A logout or another login while the refresh ran wins.
        let same_account = self
            .credentials
            .as_ref()
            .is_some_and(|current| current.dede_user_id == credentials.dede_user_id);
        if !same_account {
            if let Some(current) = &self.credentials {
                self.api_client.set_credentials(current);
            } else {
                self.api_client.clear_credentials();
            }
            return;
        }
        self.report_save_result(
            "保存登录信息",
            crate::infrastructure::persistence::save_credentials(&credentials),
        );
        self.credentials = Some(credentials);
        self.session_refresh_forced = false;
        if self.status_notice.is_none() {
            self.status_notice = Some("登录状态已自动续期".to_string());
        }
    }

    /// The comment panel showing `target`, on screen or in the back stack.
    fn comment_panel_mut(
        &mut self,
        target: crate::presentation::tui::CommentTarget,
    ) -> Option<&mut crate::presentation::tui::CommentPanel> {
        std::iter::once(&mut self.current_page)
            .chain(self.navigation_stack.iter_mut().rev())
            .find_map(|page| match page {
                Page::VideoDetail(page) if page.comment_target() == target => {
                    Some(&mut page.comments)
                }
                Page::DynamicDetail(page) if page.comment_target() == Some(target) => {
                    Some(&mut page.comments)
                }
                Page::ArticleDetail(page) if page.comment_target() == Some(target) => {
                    Some(&mut page.comments)
                }
                _ => None,
            })
    }

    /// Every panel of the comment area `oid` that holds `rpid`.
    fn apply_comment_like(&mut self, oid: i64, rpid: i64, liked: bool) {
        for page in std::iter::once(&mut self.current_page).chain(self.navigation_stack.iter_mut())
        {
            let panel = match page {
                Page::VideoDetail(page) if page.aid == oid => &mut page.comments,
                Page::DynamicDetail(page)
                    if page
                        .comment_target()
                        .is_some_and(|target| target.oid == oid) =>
                {
                    &mut page.comments
                }
                Page::ArticleDetail(page)
                    if page
                        .comment_target()
                        .is_some_and(|target| target.oid == oid) =>
                {
                    &mut page.comments
                }
                _ => continue,
            };
            panel.apply_like(rpid, liked);
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
                    .comment_target()
                    .is_some_and(|target| target.oid == oid) =>
            {
                page.restore_comment_draft(message, notice)
            }
            Page::ArticleDetail(page)
                if page
                    .comment_target()
                    .is_some_and(|target| target.oid == oid) =>
            {
                page.restore_comment_draft(message, notice)
            }
            // The page is gone, so the draft cannot be shown again; say so
            // instead of dropping it silently.
            _ => self.status_notice = Some(format!("评论未发出（{notice}），已离开该页面")),
        }
    }

    /// Reload the first page of the comment area a comment was posted to,
    /// in the sort it is shown in, so the new comment shows up.
    fn reload_comments_after_post(&mut self, oid: i64) {
        let target = match &self.current_page {
            Page::VideoDetail(page) => Some(page.comment_target()),
            Page::DynamicDetail(page) => page.comment_target(),
            Page::ArticleDetail(page) => page.comment_target(),
            _ => None,
        }
        .filter(|target| target.oid == oid);
        let Some(target) = target else {
            return;
        };
        let Some(panel) = self.comment_panel_mut(target) else {
            return;
        };
        let request = panel.begin_reload();
        self.handle_comment_request(target, request);
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
            "comments" | "comment_replies" => {
                for page in
                    std::iter::once(&mut self.current_page).chain(self.navigation_stack.iter_mut())
                {
                    match page {
                        Page::VideoDetail(page) => page.comments.apply_error(),
                        Page::DynamicDetail(page) => page.comments.apply_error(),
                        Page::ArticleDetail(page) => page.comments.apply_error(),
                        _ => {}
                    }
                }
            }
            "live_detail" => {
                if let Some(page) = find_page!(self, LiveDetail(page)) {
                    page.apply_room_info_error(format!("加载房间信息失败: {error}"));
                }
            }
            "live_qualities" => {
                if let Some(page) = find_page!(self, LiveDetail(page)) {
                    page.apply_qualities_error(format!("获取画质失败: {error}"));
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
            login_expired: false,
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
            parent: None,
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
            login_expired: false,
        });
        assert!(matches!(&app.current_page, Page::LiveDetail(page) if !page.is_loading()));
    }

    #[tokio::test]
    async fn comment_pages_reach_the_panel_and_failures_end_loading() {
        let (mut app, rx) = app();
        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1test".into(), 7);
        page.loading = false;
        let target = page.comment_target();
        app.current_page = Page::VideoDetail(Box::new(page));

        app.handle_action(AppAction::Comment(
            target,
            crate::presentation::tui::PanelRequest::Comments {
                sort: crate::api::comment::CommentSort::Hot,
                offset: Some("next".into()),
            },
        ))
        .await;
        let Ok(NetworkCommand::LoadComments { req_id, .. }) = rx.try_recv() else {
            panic!("comments were not requested");
        };
        if let Page::VideoDetail(page) = &mut app.current_page {
            page.comments.loading = true;
        }
        app.handle_network_event(NetworkEvent::RequestCancelled {
            req_id,
            target: "comments",
        });
        assert!(matches!(&app.current_page, Page::VideoDetail(page) if !page.comments.loading));

        let req_id = app.next_request_id("comments");
        app.handle_network_event(NetworkEvent::CommentsLoaded {
            req_id,
            target,
            sort: crate::api::comment::CommentSort::Hot,
            append: false,
            batch: crate::api::comment::CommentBatch {
                comments: vec![
                    serde_json::from_value(serde_json::json!({
                        "rpid": 5, "oid": 7, "mid": 1, "parent": 0
                    }))
                    .unwrap(),
                ],
                next_offset: None,
            },
        });
        assert!(matches!(&app.current_page, Page::VideoDetail(page) if page.comments.len() == 1));
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

    fn expired(req_id: u64) -> NetworkEvent {
        NetworkEvent::RequestFailed {
            req_id,
            target: "history_init",
            error: "API error (-101): 账号未登录".into(),
            login_expired: true,
        }
    }

    fn with_refresh_token(app: &mut App) {
        if let Some(credentials) = app.credentials.as_mut() {
            credentials.refresh_token = Some("token".into());
        }
    }

    #[tokio::test]
    async fn expired_login_tries_one_forced_refresh_before_prompting() {
        let (mut app, rx) = app();
        with_refresh_token(&mut app);

        app.handle_network_event(expired(1));
        app.handle_network_event(expired(2));

        assert!(matches!(
            rx.try_recv(),
            Ok(NetworkCommand::RefreshSession { force: true, .. })
        ));
        assert!(
            rx.try_recv().is_err(),
            "a refresh in flight is not repeated"
        );
        assert!(app.login_prompt.is_none());

        app.handle_network_event(NetworkEvent::SessionRefreshFailed {
            error: "HTTP 404".into(),
            forced: true,
        });
        assert!(app.login_prompt.as_deref().unwrap().contains("HTTP 404"));
    }

    #[tokio::test]
    async fn expired_login_without_refresh_token_prompts_at_once() {
        let (mut app, rx) = app();
        app.handle_network_event(expired(1));
        assert!(app.login_prompt.is_some());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn guests_are_not_asked_to_log_in_again() {
        let (mut app, _rx) = app();
        app.credentials = None;
        app.handle_network_event(expired(1));
        assert!(app.login_prompt.is_none());
    }

    #[tokio::test]
    async fn unforced_refresh_failure_does_not_prompt() {
        let (mut app, _rx) = app();
        app.handle_network_event(NetworkEvent::SessionRefreshFailed {
            error: "timeout".into(),
            forced: false,
        });
        assert!(app.login_prompt.is_none());
    }

    #[tokio::test]
    async fn refresh_for_a_replaced_account_is_dropped() {
        let (mut app, _rx) = app();
        let mut other = app.credentials.clone().unwrap();
        other.dede_user_id = "2".into();
        other.sessdata = "other".into();

        app.handle_network_event(NetworkEvent::SessionRefreshed { credentials: other });

        assert_eq!(app.credentials.as_ref().unwrap().dede_user_id, "1");
        assert!(app.status_notice.is_none());
    }

    #[tokio::test]
    async fn posted_comment_reloads_the_first_page_in_the_current_sort() {
        let (mut app, rx) = app();
        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1test".into(), 9);
        page.loading = false;
        page.comments.begin_sort_toggle();
        app.current_page = Page::VideoDetail(Box::new(page));

        app.handle_network_event(NetworkEvent::CommentPosted { oid: 9 });

        assert!(matches!(
            rx.try_recv(),
            Ok(NetworkCommand::LoadComments {
                target: crate::presentation::tui::CommentTarget {
                    oid: 9,
                    comment_type: 1
                },
                sort: crate::api::comment::CommentSort::Time,
                offset: None,
                ..
            })
        ));
        assert!(matches!(&app.current_page, Page::VideoDetail(page) if page.comments.loading));
    }

    #[tokio::test]
    async fn sent_danmaku_takes_the_position_of_the_playing_part() {
        let (mut app, rx) = app();
        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1test".into(), 1);
        page.loading = false;
        app.current_page = Page::VideoDetail(Box::new(page));
        *app.playback_position.lock().unwrap() = Some((11, 65.5));

        app.handle_action(AppAction::VideoAction(
            crate::application::VideoActionRequest::SendDanmaku {
                aid: 1,
                cid: 11,
                message: "hi".into(),
                progress_ms: 0,
            },
        ))
        .await;

        assert!(matches!(
            rx.try_recv(),
            Ok(NetworkCommand::VideoAction {
                request: crate::application::VideoActionRequest::SendDanmaku {
                    progress_ms: 65_500,
                    ..
                },
                user_mid: 1,
            })
        ));
    }

    #[tokio::test]
    async fn video_actions_need_login() {
        let (mut app, rx) = app();
        app.credentials = None;
        let mut page = crate::presentation::tui::VideoDetailPage::new("BV1test".into(), 1);
        page.loading = false;
        app.current_page = Page::VideoDetail(Box::new(page));
        app.handle_action(AppAction::VideoAction(
            crate::application::VideoActionRequest::Coin { aid: 1 },
        ))
        .await;
        assert!(rx.try_recv().is_err());
        assert!(
            matches!(&app.current_page, Page::VideoDetail(page) if page.notice.is_some() && !page.action_pending)
        );
    }
}
