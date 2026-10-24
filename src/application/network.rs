use crate::api::{
    ApiClient, LiveDanmakuHub,
    article::ArticleData,
    auth::{QrcodeData, QrcodePollStatus},
    bangumi::{SeasonRankItem, SeasonResult},
    comment::{CommentItem, CommentType},
    dynamic::DynamicItem,
    dynamic::UpListItem,
    favorite::{
        CollectedFolder, FavoriteFolder, FavoriteOrder, FavoriteResourceData, FavoriteSource,
        SeasonArchivesData, WatchLaterData,
    },
    history::{HistoryCursor, HistoryData, HistoryKey},
    live::{LiveRoom, LiveRoomInfo},
    live_ws::HistoryDanmakuData,
    recommend::{HomeFeed, VideoItem},
    search::HotwordItem,
    search::SearchVideoItem,
    space::{RelationStat, SpaceInfo, SpaceVideoData, SpaceVideoOrder},
    video::RelatedVideoItem,
    video::VideoInfo,
};
use crate::domain::playback::{PlayOrder, PlaylistItem, PlaylistSource};
use crate::presentation::tui::DynamicTab;
use crate::storage::Credentials;
use futures_util::{StreamExt, stream};
use std::collections::HashMap;
use std::sync::{Arc, mpsc};

#[derive(Debug)]
pub enum NetworkCommand {
    CancelPending,
    LoadHome {
        req_id: u64,
        feed: HomeFeed,
        use_guest_feed: bool,
    },
    LoadHomeMore {
        req_id: u64,
        fresh_idx: i32,
        feed: HomeFeed,
        use_guest_feed: bool,
    },
    LoadHotwords {
        req_id: u64,
    },
    Search {
        req_id: u64,
        keyword: String,
        page: i32,
    },
    LoadDynamicInit {
        req_id: u64,
        tab: DynamicTab,
        host_mid: Option<i64>,
    },
    LoadDynamicRefresh {
        req_id: u64,
        tab: DynamicTab,
        host_mid: Option<i64>,
    },
    LoadDynamicMore {
        req_id: u64,
        offset: String,
        tab: DynamicTab,
        host_mid: Option<i64>,
    },
    LoadHistoryInit {
        req_id: u64,
    },
    LoadHistoryMore {
        req_id: u64,
        cursor: HistoryCursor,
    },
    DeleteHistory {
        req_id: u64,
        keys: Vec<HistoryKey>,
    },
    LoadArticle {
        req_id: u64,
        cvid: i64,
    },
    LoadLiveInit {
        req_id: u64,
    },
    LoadLiveMore {
        req_id: u64,
    },
    LoadVideoDetail {
        req_id: u64,
        bvid: String,
        aid: i64,
        include_related: bool,
    },
    LoadUpPage {
        req_id: u64,
        mid: i64,
        order: SpaceVideoOrder,
    },
    LoadUpVideos {
        req_id: u64,
        mid: i64,
        page: i32,
        order: SpaceVideoOrder,
    },
    LoadFavoriteResources {
        req_id: u64,
        owner_mid: i64,
        media_id: i64,
        page: i32,
        order: FavoriteOrder,
    },
    BuildUpPlaylist {
        req_id: u64,
        mid: i64,
        name: String,
        video_order: SpaceVideoOrder,
        play_order: PlayOrder,
    },
    BuildFavoritePlaylist {
        req_id: u64,
        media_id: i64,
        title: String,
        favorite_order: FavoriteOrder,
        play_order: PlayOrder,
    },
    LoadFavoritesInit {
        req_id: u64,
        mid: i64,
    },
    LoadFavoritesContent {
        req_id: u64,
        source: FavoriteSource,
        page: i32,
    },
    LoadDynamicDetail {
        req_id: u64,
        dynamic_id: String,
    },
    LoadBangumiIndex {
        req_id: u64,
    },
    LoadBangumiDetail {
        req_id: u64,
        season_id: i64,
    },
    LoadVideoComments {
        req_id: u64,
        aid: i64,
        page: i32,
    },
    LoadDynamicComments {
        req_id: u64,
        dynamic_id: String,
        oid: i64,
        comment_type: i32,
        page: i32,
    },
    LoadCommentReplies {
        req_id: u64,
        aid: i64,
        root: i64,
    },
    LikeComment {
        oid: i64,
        rpid: i64,
        comment_type: i32,
        like: bool,
    },
    PostComment {
        oid: i64,
        comment_type: i32,
        message: String,
        root: Option<i64>,
    },
    LoadLiveDetail {
        req_id: u64,
        room_id: i64,
    },
    ConnectLiveDanmaku {
        req_id: u64,
        room_id: i64,
        uid: i64,
    },
    LoadQrcode {
        req_id: u64,
    },
    PollQrcode {
        req_id: u64,
        qrcode_key: String,
    },
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum NetworkEvent {
    HomeLoaded {
        req_id: u64,
        feed: HomeFeed,
        videos: Vec<VideoItem>,
    },
    HomeMoreLoaded {
        req_id: u64,
        feed: HomeFeed,
        videos: Vec<VideoItem>,
    },
    HotwordsLoaded {
        req_id: u64,
        hotwords: Vec<HotwordItem>,
    },
    SearchLoaded {
        req_id: u64,
        keyword: String,
        page: i32,
        results: Vec<SearchVideoItem>,
        total: i32,
    },
    DynamicLoaded {
        req_id: u64,
        append: bool,
        up_list: Option<Vec<UpListItem>>,
        items: Vec<crate::api::dynamic::DynamicItem>,
        offset: Option<String>,
        has_more: bool,
    },
    HistoryLoaded {
        req_id: u64,
        append: bool,
        data: HistoryData,
    },
    HistoryDeleted {
        req_id: u64,
        successful: Vec<HistoryKey>,
        failed: Vec<(HistoryKey, String)>,
    },
    ArticleLoaded {
        req_id: u64,
        cvid: i64,
        article: ArticleData,
        comments: Vec<CommentItem>,
    },
    LiveLoaded {
        req_id: u64,
        append: bool,
        rooms: Vec<LiveRoom>,
    },
    VideoDetailLoaded {
        req_id: u64,
        bvid: String,
        video_info: VideoInfo,
        comments: Vec<CommentItem>,
        has_more_comments: bool,
        related_videos: Vec<RelatedVideoItem>,
    },
    UpPageLoaded {
        req_id: u64,
        mid: i64,
        order: SpaceVideoOrder,
        profile: SpaceInfo,
        relation: Option<RelationStat>,
        videos: SpaceVideoData,
        folders: Vec<FavoriteFolder>,
    },
    UpVideosLoaded {
        req_id: u64,
        mid: i64,
        page: i32,
        order: SpaceVideoOrder,
        videos: SpaceVideoData,
    },
    FavoriteResourcesLoaded {
        req_id: u64,
        owner_mid: i64,
        media_id: i64,
        page: i32,
        order: FavoriteOrder,
        resources: FavoriteResourceData,
    },
    PlaylistLoaded {
        req_id: u64,
        items: Vec<PlaylistItem>,
        source: PlaylistSource,
        start_index: usize,
        order: PlayOrder,
    },
    FavoritesInitLoaded {
        req_id: u64,
        mid: i64,
        watch_later: WatchLaterData,
        created: Vec<FavoriteFolder>,
        collected: Vec<CollectedFolder>,
    },
    FavoritesWatchLaterLoaded {
        req_id: u64,
        page: i32,
        data: WatchLaterData,
    },
    FavoritesCreatedLoaded {
        req_id: u64,
        media_id: i64,
        page: i32,
        data: FavoriteResourceData,
    },
    FavoritesCollectedLoaded {
        req_id: u64,
        season_id: i64,
        page: i32,
        data: SeasonArchivesData,
    },
    DynamicDetailLoaded {
        req_id: u64,
        dynamic_id: String,
        dynamic_item: DynamicItem,
        comments: Vec<CommentItem>,
        has_more_comments: bool,
        image_urls: Vec<String>,
    },
    BangumiIndexLoaded {
        req_id: u64,
        items: Vec<SeasonRankItem>,
    },
    BangumiDetailLoaded {
        req_id: u64,
        season_id: i64,
        season: SeasonResult,
    },
    RequestFailed {
        req_id: u64,
        target: &'static str,
        error: String,
    },
    VideoCommentsLoaded {
        req_id: u64,
        aid: i64,
        page: i32,
        replies: Option<Vec<CommentItem>>,
    },
    DynamicCommentsLoaded {
        req_id: u64,
        dynamic_id: String,
        page: i32,
        replies: Option<Vec<CommentItem>>,
    },
    CommentRepliesLoaded {
        req_id: u64,
        root: i64,
        replies: Vec<CommentItem>,
    },
    CommentLiked {
        oid: i64,
        rpid: i64,
        liked: bool,
    },
    CommentPosted {
        oid: i64,
    },
    LiveDetailLoaded {
        req_id: u64,
        room_id: i64,
        room_info: LiveRoomInfo,
        history: Option<HistoryDanmakuData>,
    },
    LiveDanmakuConnected {
        req_id: u64,
        room_id: i64,
        hub: Arc<LiveDanmakuHub>,
    },
    QrcodeLoaded {
        req_id: u64,
        data: QrcodeData,
    },
    QrcodePolled {
        req_id: u64,
        status: Option<QrcodePollStatus>,
        credentials: Option<Credentials>,
    },
    /// The worker dropped the request before it finished, either because a
    /// newer request with the same key replaced it or because of
    /// `CancelPending`. Pages must leave their loading state.
    RequestCancelled {
        req_id: u64,
        target: &'static str,
    },
}

pub struct NetworkBridge {
    pub command_tx: mpsc::Sender<NetworkCommand>,
    pub event_rx: mpsc::Receiver<NetworkEvent>,
}

pub fn start_network_worker(api_client: Arc<ApiClient>) -> NetworkBridge {
    let (command_tx, command_rx) = mpsc::channel::<NetworkCommand>();
    let (event_tx, event_rx) = mpsc::channel::<NetworkEvent>();

    std::thread::Builder::new()
        .name("bilibili-network-worker".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return,
            };

            let mut cancellations =
                HashMap::<&'static str, tokio_util::sync::CancellationToken>::new();
            while let Ok(command) = command_rx.recv() {
                if matches!(command, NetworkCommand::CancelPending) {
                    cancellations.retain(|key, token| {
                        if survives_cancel_pending(key) {
                            return true;
                        }
                        token.cancel();
                        false
                    });
                    continue;
                }
                let api_client = api_client.clone();
                let event_tx = event_tx.clone();
                let (Some(key), Some(req_id)) = (command.request_key(), command.req_id()) else {
                    // Untracked mutations always run to completion.
                    runtime.spawn(async move {
                        if let Some(event) = handle_command(api_client, command).await {
                            let _ = event_tx.send(event);
                        }
                    });
                    continue;
                };
                let token = tokio_util::sync::CancellationToken::new();
                if let Some(previous) = cancellations.insert(key, token.clone()) {
                    previous.cancel();
                }
                runtime.spawn(async move {
                    let event = tokio::select! {
                        _ = token.cancelled() => Some(NetworkEvent::RequestCancelled { req_id, target: key }),
                        event = handle_command(api_client, command) => event,
                    };
                    if let Some(event) = event {
                        let _ = event_tx.send(event);
                    }
                });
            }
        })
        .expect("failed to spawn network worker");

    NetworkBridge {
        command_tx,
        event_rx,
    }
}

/// User-started background work that must finish even when the user leaves
/// the page: a playlist the user asked to play and a deletion the server may
/// already have applied.
fn survives_cancel_pending(key: &str) -> bool {
    matches!(key, "playlist_build" | "history_delete")
}

impl NetworkCommand {
    /// The request-tracking key shared with the app. A newer command with the
    /// same key supersedes the older one, so unrelated requests must not
    /// share a key. `None` for `CancelPending` and for untracked mutations,
    /// which are never superseded or cancelled.
    pub fn request_key(&self) -> Option<&'static str> {
        Some(match self {
            Self::CancelPending | Self::LikeComment { .. } | Self::PostComment { .. } => {
                return None;
            }
            Self::LoadHome { .. } => "home",
            Self::LoadHomeMore { .. } => "home_more",
            Self::LoadHotwords { .. } => "hotwords",
            Self::Search { .. } => "search",
            Self::LoadDynamicInit { .. } | Self::LoadDynamicRefresh { .. } => "dynamic_feed",
            Self::LoadDynamicMore { .. } => "dynamic_more",
            Self::LoadHistoryInit { .. } => "history_init",
            Self::LoadHistoryMore { .. } => "history_more",
            Self::DeleteHistory { .. } => "history_delete",
            Self::LoadArticle { .. } => "article_detail",
            Self::LoadLiveInit { .. } => "live_init",
            Self::LoadLiveMore { .. } => "live_more",
            Self::LoadVideoDetail { .. } => "video_detail",
            Self::LoadUpPage { .. } => "up_page",
            Self::LoadUpVideos { .. } => "up_videos",
            Self::LoadFavoriteResources { .. } => "favorite_resources",
            Self::BuildUpPlaylist { .. } | Self::BuildFavoritePlaylist { .. } => "playlist_build",
            Self::LoadFavoritesInit { .. } => "favorites_init",
            Self::LoadFavoritesContent { .. } => "favorites_content",
            Self::LoadDynamicDetail { .. } => "dynamic_detail",
            Self::LoadBangumiIndex { .. } => "bangumi_index",
            Self::LoadBangumiDetail { .. } => "bangumi_detail",
            Self::LoadVideoComments { .. } => "video_comments",
            Self::LoadDynamicComments { .. } => "dynamic_comments",
            Self::LoadCommentReplies { .. } => "comment_replies",
            Self::LoadLiveDetail { .. } => "live_detail",
            Self::ConnectLiveDanmaku { .. } => "live_danmaku",
            Self::LoadQrcode { .. } => "login_qrcode",
            Self::PollQrcode { .. } => "login_poll",
        })
    }

    fn req_id(&self) -> Option<u64> {
        Some(match self {
            Self::CancelPending | Self::LikeComment { .. } | Self::PostComment { .. } => {
                return None;
            }
            Self::LoadHome { req_id, .. }
            | Self::LoadHomeMore { req_id, .. }
            | Self::LoadHotwords { req_id }
            | Self::Search { req_id, .. }
            | Self::LoadDynamicInit { req_id, .. }
            | Self::LoadDynamicRefresh { req_id, .. }
            | Self::LoadDynamicMore { req_id, .. }
            | Self::LoadHistoryInit { req_id }
            | Self::LoadHistoryMore { req_id, .. }
            | Self::DeleteHistory { req_id, .. }
            | Self::LoadArticle { req_id, .. }
            | Self::LoadLiveInit { req_id }
            | Self::LoadLiveMore { req_id }
            | Self::LoadVideoDetail { req_id, .. }
            | Self::LoadUpPage { req_id, .. }
            | Self::LoadUpVideos { req_id, .. }
            | Self::LoadFavoriteResources { req_id, .. }
            | Self::BuildUpPlaylist { req_id, .. }
            | Self::BuildFavoritePlaylist { req_id, .. }
            | Self::LoadFavoritesInit { req_id, .. }
            | Self::LoadFavoritesContent { req_id, .. }
            | Self::LoadDynamicDetail { req_id, .. }
            | Self::LoadBangumiIndex { req_id }
            | Self::LoadBangumiDetail { req_id, .. }
            | Self::LoadVideoComments { req_id, .. }
            | Self::LoadDynamicComments { req_id, .. }
            | Self::LoadCommentReplies { req_id, .. }
            | Self::LoadLiveDetail { req_id, .. }
            | Self::ConnectLiveDanmaku { req_id, .. }
            | Self::LoadQrcode { req_id }
            | Self::PollQrcode { req_id, .. } => *req_id,
        })
    }
}

async fn handle_command(
    api_client: Arc<ApiClient>,
    command: NetworkCommand,
) -> Option<NetworkEvent> {
    match command {
        NetworkCommand::CancelPending => None,
        NetworkCommand::LikeComment {
            oid,
            rpid,
            comment_type,
            like,
        } => api_client
            .like_comment(oid, rpid, comment_type, like)
            .await
            .ok()
            .map(|()| NetworkEvent::CommentLiked {
                oid,
                rpid,
                liked: like,
            }),
        NetworkCommand::PostComment {
            oid,
            comment_type,
            message,
            root,
        } => api_client
            .add_comment(oid, comment_type, &message, root, root)
            .await
            .ok()
            .map(|_| NetworkEvent::CommentPosted { oid }),
        command => Some(handle_tracked_command(api_client, command).await),
    }
}

async fn handle_tracked_command(
    api_client: Arc<ApiClient>,
    command: NetworkCommand,
) -> NetworkEvent {
    match command {
        NetworkCommand::CancelPending
        | NetworkCommand::LikeComment { .. }
        | NetworkCommand::PostComment { .. } => unreachable!("handled by handle_command"),
        NetworkCommand::LoadVideoComments { req_id, aid, page } => {
            match api_client.get_comments(aid, page).await {
                Ok(data) => NetworkEvent::VideoCommentsLoaded {
                    req_id,
                    aid,
                    page,
                    replies: data.replies,
                },
                Err(error) => failed(req_id, "video_comments", error),
            }
        }
        NetworkCommand::LoadDynamicComments {
            req_id,
            dynamic_id,
            oid,
            comment_type,
            page,
        } => match api_client
            .get_dynamic_comments(oid, comment_type, page)
            .await
        {
            Ok(data) => NetworkEvent::DynamicCommentsLoaded {
                req_id,
                dynamic_id,
                page,
                replies: data.replies,
            },
            Err(error) => failed(req_id, "dynamic_comments", error),
        },
        NetworkCommand::LoadCommentReplies { req_id, aid, root } => {
            match api_client.get_comment_replies(aid, root, 1).await {
                Ok(data) => NetworkEvent::CommentRepliesLoaded {
                    req_id,
                    root,
                    replies: data.replies.unwrap_or_default(),
                },
                Err(error) => failed(req_id, "comment_replies", error),
            }
        }
        NetworkCommand::LoadLiveDetail { req_id, room_id } => {
            let (room_info, history) = tokio::join!(
                api_client.get_live_room_info(room_id),
                api_client.get_history_danmaku(room_id)
            );
            match room_info {
                Ok(room_info) => NetworkEvent::LiveDetailLoaded {
                    req_id,
                    room_id,
                    room_info,
                    history: history.ok(),
                },
                Err(error) => failed(req_id, "live_detail", error),
            }
        }
        NetworkCommand::ConnectLiveDanmaku {
            req_id,
            room_id,
            uid,
        } => match LiveDanmakuHub::connect(Arc::clone(&api_client), room_id, uid).await {
            Ok(hub) => NetworkEvent::LiveDanmakuConnected {
                req_id,
                room_id,
                hub,
            },
            Err(error) => failed(req_id, "live_danmaku", error),
        },
        NetworkCommand::LoadQrcode { req_id } => match api_client.get_qrcode_data().await {
            Ok(data) => NetworkEvent::QrcodeLoaded { req_id, data },
            Err(error) => failed(req_id, "login_qrcode", error),
        },
        NetworkCommand::PollQrcode { req_id, qrcode_key } => {
            match api_client.poll_qrcode(&qrcode_key).await {
                Ok(result) => {
                    let status = result
                        .data
                        .as_ref()
                        .map(|data| QrcodePollStatus::from(data.code));
                    let credentials = match (status, result.data) {
                        (Some(QrcodePollStatus::Success), Some(data)) => {
                            Credentials::from_cookies(&result.cookies, Some(data.refresh_token))
                        }
                        _ => None,
                    };
                    NetworkEvent::QrcodePolled {
                        req_id,
                        status,
                        credentials,
                    }
                }
                Err(error) => failed(req_id, "login_poll", error),
            }
        }
        NetworkCommand::LoadHome {
            req_id,
            feed,
            use_guest_feed,
        } => {
            let result = match (feed, use_guest_feed) {
                (HomeFeed::Recommended, false) => api_client.get_recommendations().await,
                (HomeFeed::Recommended, true) | (HomeFeed::Popular, _) => {
                    api_client.get_popular_videos(1, 20).await
                }
                _ => api_client.get_home_feed(feed, 1, 20).await,
            };
            match result {
                Ok(mut videos) => {
                    enrich_followers(&api_client, &mut videos).await;
                    NetworkEvent::HomeLoaded {
                        req_id,
                        feed,
                        videos,
                    }
                }
                Err(e) => failed(req_id, "home", e),
            }
        }
        NetworkCommand::LoadHomeMore {
            req_id,
            fresh_idx,
            feed,
            use_guest_feed,
        } => {
            let result = match (feed, use_guest_feed) {
                (HomeFeed::Recommended, false) => {
                    api_client.get_recommendations_paged(fresh_idx).await
                }
                (HomeFeed::Recommended, true) | (HomeFeed::Popular, _) => {
                    api_client.get_popular_videos(fresh_idx, 20).await
                }
                _ => api_client.get_home_feed(feed, fresh_idx, 20).await,
            };
            match result {
                Ok(mut videos) => {
                    enrich_followers(&api_client, &mut videos).await;
                    NetworkEvent::HomeMoreLoaded {
                        req_id,
                        feed,
                        videos,
                    }
                }
                Err(e) => failed(req_id, "home_more", e),
            }
        }
        NetworkCommand::LoadHotwords { req_id } => match api_client.get_hot_search().await {
            Ok(hotwords) => NetworkEvent::HotwordsLoaded { req_id, hotwords },
            Err(e) => failed(req_id, "hotwords", e),
        },
        NetworkCommand::Search {
            req_id,
            keyword,
            page,
        } => match api_client.search_videos(&keyword, page).await {
            Ok(data) => NetworkEvent::SearchLoaded {
                req_id,
                keyword,
                page,
                results: data.result.unwrap_or_default(),
                total: data.num_results.unwrap_or(0),
            },
            Err(e) => failed(req_id, "search", e),
        },
        NetworkCommand::LoadUpPage { req_id, mid, order } => {
            match api_client.get_space_info(mid).await {
                Ok(profile) => {
                    let relation = api_client.get_relation_stat(mid).await.ok();
                    let folders = api_client
                        .get_favorite_folders(mid)
                        .await
                        .unwrap_or_default();
                    match api_client.get_space_videos(mid, 1, 40, order).await {
                        Ok(videos) => NetworkEvent::UpPageLoaded {
                            req_id,
                            mid,
                            order,
                            profile,
                            relation,
                            videos,
                            folders,
                        },
                        Err(error) => failed(req_id, "up_page", error),
                    }
                }
                Err(error) => failed(req_id, "up_page", error),
            }
        }
        NetworkCommand::LoadUpVideos {
            req_id,
            mid,
            page,
            order,
        } => match api_client.get_space_videos(mid, page, 40, order).await {
            Ok(videos) => NetworkEvent::UpVideosLoaded {
                req_id,
                mid,
                page,
                order,
                videos,
            },
            Err(error) => failed(req_id, "up_videos", error),
        },
        NetworkCommand::LoadFavoriteResources {
            req_id,
            owner_mid,
            media_id,
            page,
            order,
        } => match api_client
            .get_favorite_resources(media_id, page, 40, order)
            .await
        {
            Ok(resources) => NetworkEvent::FavoriteResourcesLoaded {
                req_id,
                owner_mid,
                media_id,
                page,
                order,
                resources,
            },
            Err(error) => failed(req_id, "favorite_resources", error),
        },
        NetworkCommand::BuildUpPlaylist {
            req_id,
            mid,
            name,
            video_order,
            play_order,
        } => {
            let mut items = Vec::new();
            let mut page = 1;
            loop {
                let data = match api_client
                    .get_space_videos(mid, page, 40, video_order)
                    .await
                {
                    Ok(data) => data,
                    Err(error) => return failed(req_id, "playlist_build", error),
                };
                let total = data.page.count as usize;
                let empty = data.list.vlist.is_empty();
                items.extend(data.list.vlist.into_iter().map(|video| PlaylistItem {
                    bvid: video.bvid,
                    aid: video.aid,
                    cid: None,
                    title: video.title,
                    uploader_mid: Some(video.mid.unwrap_or(mid)),
                    duration: video.duration,
                    page: None,
                }));
                if empty || items.len() >= total {
                    break;
                }
                page += 1;
            }
            let start_index = if play_order == PlayOrder::Reverse {
                items.len().saturating_sub(1)
            } else {
                0
            };
            NetworkEvent::PlaylistLoaded {
                req_id,
                items,
                source: PlaylistSource::Uploader { mid, name },
                start_index,
                order: play_order,
            }
        }
        NetworkCommand::BuildFavoritePlaylist {
            req_id,
            media_id,
            title,
            favorite_order,
            play_order,
        } => {
            let mut items = Vec::new();
            let mut page = 1;
            loop {
                let data = match api_client
                    .get_favorite_resources(media_id, page, 40, favorite_order)
                    .await
                {
                    Ok(data) => data,
                    Err(error) => return failed(req_id, "playlist_build", error),
                };
                let has_more = data.has_more.unwrap_or(false);
                items.extend(data.medias.into_iter().filter_map(|media| {
                    Some(PlaylistItem {
                        bvid: media.bvid?,
                        aid: media.id,
                        cid: None,
                        title: media.title,
                        uploader_mid: media.upper.as_ref().map(|upper| upper.mid),
                        duration: media.duration,
                        page: None,
                    })
                }));
                if !has_more {
                    break;
                }
                page += 1;
            }
            let start_index = if play_order == PlayOrder::Reverse {
                items.len().saturating_sub(1)
            } else {
                0
            };
            NetworkEvent::PlaylistLoaded {
                req_id,
                items,
                source: PlaylistSource::Favorites { media_id, title },
                start_index,
                order: play_order,
            }
        }
        NetworkCommand::LoadFavoritesInit { req_id, mid } => {
            match api_client.get_watch_later(1, 20).await {
                Ok(watch_later) => {
                    let created = api_client
                        .get_favorite_folders(mid)
                        .await
                        .unwrap_or_default();
                    match api_client.get_collected_folders(mid, 1, 50).await {
                        Ok(collected) => NetworkEvent::FavoritesInitLoaded {
                            req_id,
                            mid,
                            watch_later,
                            created,
                            collected: collected.list,
                        },
                        Err(error) => failed(req_id, "favorites_init", error),
                    }
                }
                Err(error) => failed(req_id, "favorites_init", error),
            }
        }
        NetworkCommand::LoadFavoritesContent {
            req_id,
            source,
            page,
        } => match source {
            FavoriteSource::WatchLater => match api_client.get_watch_later(page, 20).await {
                Ok(data) => NetworkEvent::FavoritesWatchLaterLoaded { req_id, page, data },
                Err(error) => failed(req_id, "favorites_content", error),
            },
            FavoriteSource::Created { media_id, .. } => match api_client
                .get_favorite_resources(media_id, page, 40, FavoriteOrder::RecentlyFavorited)
                .await
            {
                Ok(data) => NetworkEvent::FavoritesCreatedLoaded {
                    req_id,
                    media_id,
                    page,
                    data,
                },
                Err(error) => failed(req_id, "favorites_content", error),
            },
            FavoriteSource::Collected { season_id, mid, .. } => match api_client
                .get_collected_season_videos(mid, season_id, page, 30)
                .await
            {
                Ok(data) => NetworkEvent::FavoritesCollectedLoaded {
                    req_id,
                    season_id,
                    page,
                    data,
                },
                Err(error) => failed(req_id, "favorites_content", error),
            },
        },
        NetworkCommand::LoadDynamicInit {
            req_id,
            tab,
            host_mid,
        } => {
            let up_list = match api_client.get_dynamic_portal().await {
                Ok(portal) => portal.up_list,
                Err(_) => None,
            };
            let feed_type = tab.get_feed_type();
            match api_client.get_dynamic_feed(None, feed_type, host_mid).await {
                Ok(data) => NetworkEvent::DynamicLoaded {
                    req_id,
                    append: false,
                    up_list,
                    items: data.items.unwrap_or_default(),
                    offset: data.offset,
                    has_more: data.has_more.unwrap_or(false),
                },
                Err(e) => failed(req_id, "dynamic_feed", e),
            }
        }
        NetworkCommand::LoadDynamicRefresh {
            req_id,
            tab,
            host_mid,
        } => {
            let feed_type = tab.get_feed_type();
            match api_client.get_dynamic_feed(None, feed_type, host_mid).await {
                Ok(data) => NetworkEvent::DynamicLoaded {
                    req_id,
                    append: false,
                    up_list: None,
                    items: data.items.unwrap_or_default(),
                    offset: data.offset,
                    has_more: data.has_more.unwrap_or(false),
                },
                Err(e) => failed(req_id, "dynamic_feed", e),
            }
        }
        NetworkCommand::LoadDynamicMore {
            req_id,
            offset,
            tab,
            host_mid,
        } => {
            let feed_type = tab.get_feed_type();
            match api_client
                .get_dynamic_feed(Some(&offset), feed_type, host_mid)
                .await
            {
                Ok(data) => NetworkEvent::DynamicLoaded {
                    req_id,
                    append: true,
                    up_list: None,
                    items: data.items.unwrap_or_default(),
                    offset: data.offset,
                    has_more: data.has_more.unwrap_or(false),
                },
                Err(e) => failed(req_id, "dynamic_more", e),
            }
        }
        NetworkCommand::LoadHistoryInit { req_id } => {
            match api_client.get_history(None, None, None).await {
                Ok(data) => NetworkEvent::HistoryLoaded {
                    req_id,
                    append: false,
                    data,
                },
                Err(e) => failed(req_id, "history_init", e),
            }
        }
        NetworkCommand::LoadHistoryMore { req_id, cursor } => match api_client
            .get_history(
                Some(cursor.max),
                Some(cursor.view_at),
                Some(cursor.business.as_str()),
            )
            .await
        {
            Ok(data) => NetworkEvent::HistoryLoaded {
                req_id,
                append: true,
                data,
            },
            Err(e) => failed(req_id, "history_more", e),
        },
        NetworkCommand::DeleteHistory { req_id, keys } => {
            let mut successful = Vec::new();
            let mut failed_keys = Vec::new();
            for key in keys {
                match api_client.delete_history_item(&key).await {
                    Ok(()) => successful.push(key),
                    Err(error) => failed_keys.push((key, error.to_string())),
                }
            }
            NetworkEvent::HistoryDeleted {
                req_id,
                successful,
                failed: failed_keys,
            }
        }
        NetworkCommand::LoadArticle { req_id, cvid } => match api_client.get_article(cvid).await {
            Ok(article) => {
                let comment_oid = if article.id > 0 { article.id } else { cvid };
                let comments = api_client
                    .get_dynamic_comments(comment_oid, CommentType::Article.as_i32(), 1)
                    .await
                    .ok()
                    .map(|data| {
                        let mut comments = data.hots.unwrap_or_default();
                        for comment in data.replies.unwrap_or_default() {
                            if !comments
                                .iter()
                                .any(|existing| existing.rpid == comment.rpid)
                            {
                                comments.push(comment);
                            }
                        }
                        comments
                    })
                    .unwrap_or_default();
                NetworkEvent::ArticleLoaded {
                    req_id,
                    cvid,
                    article,
                    comments,
                }
            }
            Err(error) => failed(req_id, "article_detail", error),
        },
        NetworkCommand::LoadLiveInit { req_id } => {
            let rooms = match api_client.get_live_home_rooms().await {
                Ok(rooms) => Ok(rooms),
                Err(_) => api_client.get_live_recommendations().await,
            };
            match rooms {
                Ok(rooms) => NetworkEvent::LiveLoaded {
                    req_id,
                    append: false,
                    rooms,
                },
                Err(e) => failed(req_id, "live_init", e),
            }
        }
        NetworkCommand::LoadLiveMore { req_id } => {
            match api_client.get_live_recommendations().await {
                Ok(rooms) => NetworkEvent::LiveLoaded {
                    req_id,
                    append: true,
                    rooms,
                },
                Err(e) => failed(req_id, "live_more", e),
            }
        }
        NetworkCommand::LoadVideoDetail {
            req_id,
            bvid,
            aid,
            include_related,
        } => {
            let video_info = match api_client.get_video_info(&bvid).await {
                Ok(info) => info,
                Err(e) => return failed(req_id, "video_detail", e),
            };
            let (comments, has_more_comments) = match api_client.get_comments(aid, 1).await {
                Ok(data) => {
                    let comments = data.replies.unwrap_or_default();
                    let has_more = data
                        .page
                        .map(|p| p.count.unwrap_or(0) > comments.len() as i32)
                        .unwrap_or(false);
                    (comments, has_more)
                }
                Err(_) => (Vec::new(), false),
            };
            let related_videos = if include_related {
                api_client
                    .get_related_videos(&bvid)
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            NetworkEvent::VideoDetailLoaded {
                req_id,
                bvid,
                video_info,
                comments,
                has_more_comments,
                related_videos,
            }
        }
        NetworkCommand::LoadDynamicDetail { req_id, dynamic_id } => {
            let dynamic_item = match api_client.get_dynamic_detail(&dynamic_id).await {
                Ok(item) => item,
                Err(e) => return failed(req_id, "dynamic_detail", e),
            };
            let comment_type = dynamic_item.comment_type();
            let comment_oid = dynamic_item.comment_oid(&dynamic_id);
            let (comments, has_more_comments) = if let Some(oid) = comment_oid {
                match api_client.get_dynamic_comments(oid, comment_type, 1).await {
                    Ok(data) => {
                        let comments = data.replies.unwrap_or_default();
                        let has_more = data
                            .page
                            .map(|p| p.count.unwrap_or(0) > comments.len() as i32)
                            .unwrap_or(false);
                        (comments, has_more)
                    }
                    Err(_) => (Vec::new(), false),
                }
            } else {
                (Vec::new(), false)
            };
            let mut image_urls = Vec::new();
            if dynamic_item.is_draw() {
                image_urls.extend(
                    dynamic_item
                        .draw_images()
                        .into_iter()
                        .map(|s| s.to_string()),
                );
            }
            if dynamic_item.is_opus() {
                image_urls.extend(
                    dynamic_item
                        .opus_images()
                        .into_iter()
                        .map(|s| s.to_string()),
                );
            }
            NetworkEvent::DynamicDetailLoaded {
                req_id,
                dynamic_id,
                dynamic_item,
                comments,
                has_more_comments,
                image_urls,
            }
        }
        NetworkCommand::LoadBangumiIndex { req_id } => match api_client.get_bangumi_rank().await {
            Ok(items) => NetworkEvent::BangumiIndexLoaded { req_id, items },
            Err(e) => failed(req_id, "bangumi_index", e),
        },
        NetworkCommand::LoadBangumiDetail { req_id, season_id } => {
            match api_client.get_bangumi_season(season_id).await {
                Ok(season) => NetworkEvent::BangumiDetailLoaded {
                    req_id,
                    season_id,
                    season,
                },
                Err(e) => failed(req_id, "bangumi_detail", e),
            }
        }
    }
}

async fn enrich_followers(api_client: &Arc<ApiClient>, videos: &mut [VideoItem]) {
    let mids = videos
        .iter()
        .filter_map(|video| video.owner.as_ref().map(|owner| owner.mid))
        .filter(|mid| *mid > 0)
        .collect::<std::collections::BTreeSet<_>>();
    let followers = stream::iter(mids)
        .map(|mid| {
            let client = Arc::clone(api_client);
            async move {
                let follower = client
                    .get_relation_stat(mid)
                    .await
                    .ok()
                    .and_then(|stat| stat.follower);
                (mid, follower)
            }
        })
        .buffer_unordered(6)
        .collect::<HashMap<_, _>>()
        .await;
    for video in videos {
        if let Some(owner) = video.owner.as_mut() {
            owner.follower = followers.get(&owner.mid).copied().flatten();
        }
    }
}

fn failed(req_id: u64, target: &'static str, error: anyhow::Error) -> NetworkEvent {
    let safe_error = redact_url_queries(&error.to_string());
    if let Some(mut dir) = dirs::config_dir() {
        dir.push("bilibili-tui");
        let log_path = dir.join("debug.log");
        if std::fs::create_dir_all(&dir).is_ok()
            && let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
        {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&log_path, std::fs::Permissions::from_mode(0o600));
            }
            use std::io::Write;
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let safe_log_error = redact_url_queries(&format!("{error:#}"));
            let _ = writeln!(
                log,
                "[{timestamp}] Network request failed\nTarget: {target}\nRequest ID: {req_id}\nError: {safe_log_error}\n"
            );
        }
    }

    NetworkEvent::RequestFailed {
        req_id,
        target,
        error: safe_error,
    }
}

pub(crate) fn redact_url_queries(message: &str) -> String {
    let mut output = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();
    while let Some(ch) = chars.next() {
        output.push(ch);
        if ch == '?' {
            output.push_str("<redacted>");
            while chars
                .peek()
                .is_some_and(|next| !next.is_whitespace() && !matches!(next, '"' | '\''))
            {
                chars.next();
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{NetworkCommand, survives_cancel_pending};

    #[test]
    fn unrelated_requests_do_not_share_a_cancel_key() {
        let key = |command: NetworkCommand| command.request_key();
        assert_ne!(
            key(NetworkCommand::LoadHotwords { req_id: 1 }),
            key(NetworkCommand::Search {
                req_id: 2,
                keyword: "rust".into(),
                page: 1,
            })
        );
        assert_ne!(
            key(NetworkCommand::LoadLiveInit { req_id: 1 }),
            key(NetworkCommand::LoadLiveMore { req_id: 2 })
        );
        assert_ne!(
            key(NetworkCommand::LoadUpPage {
                req_id: 1,
                mid: 1,
                order: crate::api::space::SpaceVideoOrder::Latest,
            }),
            key(NetworkCommand::LoadUpVideos {
                req_id: 2,
                mid: 1,
                page: 2,
                order: crate::api::space::SpaceVideoOrder::Latest,
            })
        );
        assert_eq!(key(NetworkCommand::CancelPending), None);
    }

    #[test]
    fn comment_mutations_are_never_superseded() {
        for command in [
            NetworkCommand::LikeComment {
                oid: 1,
                rpid: 2,
                comment_type: 1,
                like: true,
            },
            NetworkCommand::PostComment {
                oid: 1,
                comment_type: 1,
                message: "hi".into(),
                root: None,
            },
        ] {
            assert_eq!(command.request_key(), None);
            assert_eq!(command.req_id(), None);
        }
    }

    #[test]
    fn cancel_pending_keeps_playlist_builds_and_deletions() {
        assert!(survives_cancel_pending("playlist_build"));
        assert!(survives_cancel_pending("history_delete"));
        assert!(!survives_cancel_pending("home"));
    }
}

#[cfg(test)]
mod security_tests {
    use super::redact_url_queries;

    #[test]
    fn network_log_redacts_url_query_parameters() {
        let value =
            redact_url_queries("GET https://cdn.example/video?token=secret&expires=1 failed");
        assert_eq!(value, "GET https://cdn.example/video?<redacted> failed");
        assert!(!value.contains("secret"));
    }
}
