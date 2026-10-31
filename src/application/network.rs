use crate::api::{
    ApiClient, LiveDanmakuHub,
    article::ArticleData,
    auth::{QrcodeData, QrcodePollStatus},
    bangumi::{SeasonRankItem, SeasonResult},
    comment::{CommentBatch, CommentItem, CommentSort, CommentType},
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
    search::{SearchKind, SearchResultItem},
    space::{RelationStat, SpaceInfo, SpaceVideoData, SpaceVideoOrder},
    video::RelatedVideoItem,
    video::VideoInfo,
};
use crate::domain::playback::{PlayOrder, PlaylistItem, PlaylistSource};
pub use crate::presentation::tui::CommentTarget;
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
        kind: SearchKind,
        page: i32,
    },
    LoadSearchSuggestions {
        req_id: u64,
        term: String,
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
    /// Top-level comments of a comment area. The first page
    /// (`offset: None`) replaces the list; later pages are appended.
    LoadComments {
        req_id: u64,
        target: CommentTarget,
        sort: CommentSort,
        offset: Option<String>,
    },
    LoadCommentReplies {
        req_id: u64,
        target: CommentTarget,
        root: i64,
        page: i32,
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
        parent: Option<i64>,
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
    LoadLiveQualities {
        req_id: u64,
        room_id: i64,
        quality: crate::storage::VideoQuality,
    },
    SendLiveDanmaku {
        room_id: i64,
        message: String,
    },
    VideoAction {
        request: crate::application::VideoActionRequest,
        user_mid: i64,
    },
    /// Refresh the login cookies. Without `force`, only when the server
    /// asks for it; with `force`, after a request reported `-101`.
    RefreshSession {
        credentials: Credentials,
        force: bool,
    },
    /// End the session on the server after the app forgot it locally.
    Logout {
        credentials: Credentials,
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
        kind: SearchKind,
        page: i32,
        results: Vec<SearchResultItem>,
        total: i32,
    },
    SearchSuggestionsLoaded {
        req_id: u64,
        term: String,
        suggestions: Vec<String>,
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
        comments: CommentBatch,
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
        /// The account's like/coin/favourite/follow state; `None` for guests.
        relation: Option<crate::api::client::ArchiveRelation>,
        comments: CommentBatch,
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
        comments: CommentBatch,
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
        /// The server answered `-101`: the saved login no longer works.
        login_expired: bool,
    },
    CommentsLoaded {
        req_id: u64,
        target: CommentTarget,
        sort: CommentSort,
        append: bool,
        batch: CommentBatch,
    },
    CommentRepliesLoaded {
        req_id: u64,
        target: CommentTarget,
        root: i64,
        page: i32,
        replies: Vec<CommentItem>,
        has_more: bool,
    },
    CommentLiked {
        oid: i64,
        rpid: i64,
        liked: bool,
    },
    CommentPosted {
        oid: i64,
    },
    CommentPostFailed {
        oid: i64,
        message: String,
        error: String,
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
    LiveQualitiesLoaded {
        req_id: u64,
        room_id: i64,
        qualities: Vec<crate::api::live::LiveQuality>,
    },
    LiveDanmakuSent {
        room_id: i64,
    },
    VideoActionDone {
        request: crate::application::VideoActionRequest,
        /// What a triple did, for `Triple`.
        result: Result<Option<crate::api::client::TripleResult>, String>,
    },
    LiveDanmakuSendFailed {
        room_id: i64,
        message: String,
        error: String,
    },
    SessionRefreshed {
        credentials: Credentials,
    },
    SessionRefreshFailed {
        error: String,
        /// The refresh was a reaction to `-101`, so the login is gone.
        forced: bool,
    },
    ServerLogoutFailed {
        error: String,
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

            // Device cookies make guest requests look like a browser; every
            // request still works while they are being fetched.
            {
                let api_client = api_client.clone();
                runtime.spawn(async move {
                    let _ = api_client.ensure_device_cookies().await;
                });
            }
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
            Self::CancelPending
            | Self::LikeComment { .. }
            | Self::PostComment { .. }
            | Self::RefreshSession { .. }
            | Self::Logout { .. }
            | Self::SendLiveDanmaku { .. }
            | Self::VideoAction { .. } => {
                return None;
            }
            Self::LoadLiveQualities { .. } => "live_qualities",
            Self::LoadHome { .. } => "home",
            Self::LoadHomeMore { .. } => "home_more",
            Self::LoadHotwords { .. } => "hotwords",
            Self::Search { .. } => "search",
            Self::LoadSearchSuggestions { .. } => "search_suggest",
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
            Self::LoadComments { .. } => "comments",
            Self::LoadCommentReplies { .. } => "comment_replies",
            Self::LoadLiveDetail { .. } => "live_detail",
            Self::ConnectLiveDanmaku { .. } => "live_danmaku",
            Self::LoadQrcode { .. } => "login_qrcode",
            Self::PollQrcode { .. } => "login_poll",
        })
    }

    fn req_id(&self) -> Option<u64> {
        Some(match self {
            Self::CancelPending
            | Self::LikeComment { .. }
            | Self::PostComment { .. }
            | Self::RefreshSession { .. }
            | Self::Logout { .. }
            | Self::SendLiveDanmaku { .. }
            | Self::VideoAction { .. } => {
                return None;
            }
            Self::LoadLiveQualities { req_id, .. } => *req_id,
            Self::LoadHome { req_id, .. }
            | Self::LoadHomeMore { req_id, .. }
            | Self::LoadHotwords { req_id }
            | Self::Search { req_id, .. }
            | Self::LoadSearchSuggestions { req_id, .. }
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
            | Self::LoadComments { req_id, .. }
            | Self::LoadCommentReplies { req_id, .. }
            | Self::LoadLiveDetail { req_id, .. }
            | Self::ConnectLiveDanmaku { req_id, .. }
            | Self::LoadQrcode { req_id }
            | Self::PollQrcode { req_id, .. } => *req_id,
        })
    }
}

/// Upper bound on pages fetched while building one playlist.
const MAX_PLAYLIST_PAGES: i32 = 250;

/// Stop paging on an empty page even when the API still claims more results,
/// and after `MAX_PLAYLIST_PAGES`, so a misbehaving endpoint cannot loop forever.
fn playlist_has_next_page(page: i32, page_len: usize, has_more: bool) -> bool {
    has_more && page_len > 0 && page < MAX_PLAYLIST_PAGES
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
            parent,
        } => Some(
            match api_client
                .add_comment(oid, comment_type, &message, root, parent.or(root))
                .await
            {
                Ok(_) => NetworkEvent::CommentPosted { oid },
                Err(error) => NetworkEvent::CommentPostFailed {
                    oid,
                    message,
                    error: redact_url_queries(&error.to_string()),
                },
            },
        ),
        NetworkCommand::VideoAction { request, user_mid } => {
            let result = run_video_action(&api_client, &request, user_mid)
                .await
                .map_err(|error| redact_url_queries(&format!("{error:#}")));
            Some(NetworkEvent::VideoActionDone { request, result })
        }
        NetworkCommand::SendLiveDanmaku { room_id, message } => Some(
            match api_client.send_live_danmaku(room_id, &message).await {
                Ok(()) => NetworkEvent::LiveDanmakuSent { room_id },
                Err(error) => NetworkEvent::LiveDanmakuSendFailed {
                    room_id,
                    message,
                    error: redact_url_queries(&format!("{error:#}")),
                },
            },
        ),
        NetworkCommand::RefreshSession { credentials, force } => {
            refresh_session(&api_client, &credentials, force).await
        }
        NetworkCommand::Logout { credentials } => {
            api_client.logout(&credentials).await.err().map(|error| {
                NetworkEvent::ServerLogoutFailed {
                    error: redact_url_queries(&format!("{error:#}")),
                }
            })
        }
        command => Some(handle_tracked_command(api_client, command).await),
    }
}

async fn run_video_action(
    api_client: &ApiClient,
    request: &crate::application::VideoActionRequest,
    user_mid: i64,
) -> anyhow::Result<Option<crate::api::client::TripleResult>> {
    use crate::application::VideoActionRequest as Request;
    match request {
        Request::Like { aid, like } => api_client.like_video(*aid, *like).await?,
        Request::Coin { aid } => api_client.coin_video(*aid).await?,
        Request::Favorite { aid, favorite } => {
            api_client
                .set_video_favorite(*aid, user_mid, *favorite)
                .await?;
        }
        Request::Triple { aid } => return Ok(Some(api_client.triple_video(*aid).await?)),
        Request::WatchLater { aid } => api_client.add_to_watch_later(*aid).await?,
        Request::Follow { mid, follow, .. } => api_client.set_following(*mid, *follow).await?,
        Request::SendDanmaku {
            aid,
            cid,
            message,
            progress_ms,
        } => {
            api_client
                .send_video_danmaku(*aid, *cid, *progress_ms, message)
                .await?
        }
    }
    Ok(None)
}

/// `None` when the cookies are still fresh and nothing had to change.
async fn refresh_session(
    api_client: &ApiClient,
    credentials: &Credentials,
    force: bool,
) -> Option<NetworkEvent> {
    let timestamp = if force {
        chrono::Utc::now().timestamp_millis()
    } else {
        match api_client.get_cookie_refresh_info().await {
            Ok(info) if !info.refresh => return None,
            Ok(info) => info.timestamp,
            Err(error) => {
                return crate::api::client::is_login_expired(&error).then(|| {
                    NetworkEvent::SessionRefreshFailed {
                        error: redact_url_queries(&format!("{error:#}")),
                        forced: true,
                    }
                });
            }
        }
    };
    Some(
        match api_client.refresh_cookies(credentials, timestamp).await {
            Ok(credentials) => NetworkEvent::SessionRefreshed { credentials },
            Err(error) => NetworkEvent::SessionRefreshFailed {
                error: redact_url_queries(&format!("{error:#}")),
                forced: force,
            },
        },
    )
}

async fn handle_tracked_command(
    api_client: Arc<ApiClient>,
    command: NetworkCommand,
) -> NetworkEvent {
    match command {
        NetworkCommand::CancelPending
        | NetworkCommand::LikeComment { .. }
        | NetworkCommand::PostComment { .. }
        | NetworkCommand::RefreshSession { .. }
        | NetworkCommand::Logout { .. }
        | NetworkCommand::SendLiveDanmaku { .. }
        | NetworkCommand::VideoAction { .. } => unreachable!("handled by handle_command"),
        NetworkCommand::LoadLiveQualities {
            req_id,
            room_id,
            quality,
        } => match api_client.get_live_qualities(room_id, quality).await {
            Ok(qualities) => NetworkEvent::LiveQualitiesLoaded {
                req_id,
                room_id,
                qualities,
            },
            Err(error) => failed(req_id, "live_qualities", error),
        },
        NetworkCommand::LoadComments {
            req_id,
            target,
            sort,
            offset,
        } => match api_client
            .get_main_comments(target.oid, target.comment_type, sort, offset.as_deref())
            .await
        {
            Ok(batch) => NetworkEvent::CommentsLoaded {
                req_id,
                target,
                sort,
                append: offset.is_some(),
                batch,
            },
            Err(error) => failed(req_id, "comments", error),
        },
        NetworkCommand::LoadCommentReplies {
            req_id,
            target,
            root,
            page,
        } => match api_client
            .get_comment_replies(target.oid, target.comment_type, root, page)
            .await
        {
            Ok((replies, has_more)) => NetworkEvent::CommentRepliesLoaded {
                req_id,
                target,
                root,
                page,
                replies,
                has_more,
            },
            Err(error) => failed(req_id, "comment_replies", error),
        },
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
            kind,
            page,
        } => match api_client.search(&keyword, kind, page).await {
            Ok((results, total)) => NetworkEvent::SearchLoaded {
                req_id,
                keyword,
                kind,
                page,
                results,
                total,
            },
            Err(e) => failed(req_id, "search", e),
        },
        NetworkCommand::LoadSearchSuggestions { req_id, term } => {
            match api_client.get_search_suggestions(&term).await {
                Ok(suggestions) => NetworkEvent::SearchSuggestionsLoaded {
                    req_id,
                    term,
                    suggestions,
                },
                Err(e) => failed(req_id, "search_suggest", e),
            }
        }
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
                let page_len = data.list.vlist.len();
                items.extend(data.list.vlist.into_iter().map(|video| PlaylistItem {
                    bvid: video.bvid,
                    aid: video.aid,
                    cid: None,
                    title: video.title,
                    uploader_mid: Some(video.mid.unwrap_or(mid)),
                    duration: video.duration,
                    page: None,
                }));
                if !playlist_has_next_page(page, page_len, items.len() < total) {
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
                let page_len = data.medias.len();
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
                if !playlist_has_next_page(page, page_len, has_more) {
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
                    .get_main_comments(
                        comment_oid,
                        CommentType::Article.as_i32(),
                        CommentSort::Hot,
                        None,
                    )
                    .await
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
            let comments = api_client
                .get_main_comments(aid, CommentType::Video.as_i32(), CommentSort::Hot, None)
                .await
                .unwrap_or_default();
            let relation = if api_client.is_logged_in() {
                api_client.get_archive_relation(video_info.aid).await.ok()
            } else {
                None
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
                relation,
                comments,
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
            let comments = match comment_oid {
                Some(oid) => api_client
                    .get_main_comments(oid, comment_type, CommentSort::Hot, None)
                    .await
                    .unwrap_or_default(),
                None => CommentBatch::default(),
            };
            let image_urls = dynamic_image_urls(&dynamic_item);
            NetworkEvent::DynamicDetailLoaded {
                req_id,
                dynamic_id,
                dynamic_item,
                comments,
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

/// Pictures of a dynamic, or of the dynamic it forwards.
fn dynamic_image_urls(item: &DynamicItem) -> Vec<String> {
    let source = match item.kind() {
        crate::api::dynamic::DynamicKind::Forward => match item.forwarded() {
            Some(orig) => orig,
            None => return Vec::new(),
        },
        _ => item,
    };
    source
        .draw_images()
        .into_iter()
        .chain(source.opus_images())
        .map(str::to_string)
        .collect()
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
    let login_expired = crate::api::client::is_login_expired(&error);
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
        login_expired,
    }
}

pub(crate) use crate::api::redact::redact_url_queries;

#[cfg(test)]
mod tests {
    use super::{
        MAX_PLAYLIST_PAGES, NetworkCommand, playlist_has_next_page, survives_cancel_pending,
    };

    #[test]
    fn playlist_paging_stops_on_empty_page_despite_has_more() {
        assert!(playlist_has_next_page(1, 40, true));
        assert!(!playlist_has_next_page(2, 0, true));
        assert!(!playlist_has_next_page(1, 40, false));
        assert!(!playlist_has_next_page(MAX_PLAYLIST_PAGES, 40, true));
    }

    #[test]
    fn unrelated_requests_do_not_share_a_cancel_key() {
        let key = |command: NetworkCommand| command.request_key();
        assert_ne!(
            key(NetworkCommand::LoadHotwords { req_id: 1 }),
            key(NetworkCommand::Search {
                req_id: 2,
                keyword: "rust".into(),
                kind: crate::api::search::SearchKind::Video,
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
                parent: None,
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
