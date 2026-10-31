use crate::api::favorite::{FavoriteOrder, FavoriteSource};
use crate::api::history::HistoryKey;
use crate::api::recommend::HomeFeed;
use crate::api::space::SpaceVideoOrder;
use crate::api::video::VideoPage;
use crate::domain::playback::{PlayOrder, PlaylistItem, PlaylistSource};
use crate::infrastructure::persistence::{Credentials, DanmakuConfig, Keybindings, VideoQuality};
use crate::presentation::tui::DynamicTab;

/// Actions that can be triggered from UI components
#[derive(Debug, Clone)]
pub enum AppAction {
    /// Quit the application
    Quit,
    /// Switch to home page
    SwitchToHome,
    /// Refresh home page recommendations (force reload)
    RefreshHome,
    SwitchHomeFeed(HomeFeed),
    /// Switch to login page
    SwitchToLogin,
    /// Leave the login page for the page selected in the sidebar.
    CloseLogin,
    /// Switch to settings page
    SwitchToSettings,
    /// Switch to history page
    SwitchToHistory,
    /// Reload history from the newest entry.
    RefreshHistory,
    /// Login was successful with credentials
    LoginSuccess(Credentials),
    /// Play a video with metadata (bvid, aid, cid, duration)
    PlayVideo {
        bvid: String,
        aid: i64,
        cid: i64,
        duration: i64,
    },
    /// Play a video with page info for auto-play next episode
    PlayVideoWithPages {
        bvid: String,
        aid: i64,
        pages: Vec<VideoPage>,
        current_index: usize,
    },
    PlayPlaylist {
        items: Vec<PlaylistItem>,
        source: PlaylistSource,
        start_index: usize,
        order: PlayOrder,
    },
    PlayUpAll {
        mid: i64,
        name: String,
        video_order: SpaceVideoOrder,
        play_order: PlayOrder,
    },
    PlayFavoriteAll {
        media_id: i64,
        title: String,
        favorite_order: FavoriteOrder,
        play_order: PlayOrder,
    },
    /// Navigate to next sidebar item
    NavNext,
    /// Navigate to previous sidebar item
    NavPrev,
    CancelPendingLoads,
    /// Search for the keyword with the page's current result kind
    Search(String),
    /// Show search results of another kind, searching again if needed.
    SwitchSearchKind(crate::api::search::SearchKind),
    /// Refresh dynamic feed
    RefreshDynamic,
    /// Open video detail page (bvid, aid)
    OpenVideoDetail(String, i64),
    /// Open an uploader's public space by member ID.
    OpenUpPage(i64),
    RefreshUpPage,
    SwitchUpVideoOrder(SpaceVideoOrder),
    LoadMoreUpVideos,
    OpenFavoriteFolder(i64),
    SwitchFavoriteOrder(FavoriteOrder),
    LoadMoreFavoriteResources,
    SelectFavoriteSource(FavoriteSource),
    LoadMoreFavorites,
    /// Open dynamic detail page for image/text dynamics (dynamic_id)
    OpenDynamicDetail(String),
    /// Go back to previous page
    BackToList,
    /// Load more recommendations
    LoadMoreRecommendations,
    /// Load more search results
    LoadMoreSearch,
    /// Load more dynamic items
    LoadMoreDynamic,
    /// Load more history items
    LoadMoreHistory,
    DeleteHistoryItems(Vec<HistoryKey>),
    OpenArticle(i64),
    OpenHistoryBangumi {
        season_id: i64,
        ep_id: i64,
    },
    /// Something a comment panel asked for: loading, a reply thread or a
    /// like. `Compose` requests are handled by the page itself.
    Comment(
        crate::presentation::tui::CommentTarget,
        crate::presentation::tui::PanelRequest,
    ),
    /// Switch dynamic tab
    SwitchDynamicTab(DynamicTab),
    /// Select UP master (0 = all, 1+ = specific UP)
    SelectUpMaster(usize),
    /// Switch to next theme variant
    NextTheme,
    /// Set a specific theme by Opaline theme ID
    SetTheme(String),
    /// Save keybindings to config
    SaveKeybindings(Box<Keybindings>),
    /// Save live/video danmaku rendering settings.
    SaveDanmakuConfig(Box<DanmakuConfig>),
    /// Save the auto-play-on-video-open preference.
    SaveAutoPlay(bool),
    SaveMinimalMode(bool),
    /// Switch between Nerd Font icons and text labels.
    SaveNerdFontIcons(bool),
    SaveVideoQuality(VideoQuality),
    /// Logout and return to login page
    Logout,
    /// Post a comment, or a reply when `root` is set; `parent` is the
    /// comment replied to inside the thread.
    AddComment {
        oid: i64,
        comment_type: i32,
        message: String,
        root: Option<i64>,
        parent: Option<i64>,
    },
    /// Switch to live page
    SwitchToLive,
    /// Open live room detail
    OpenLiveDetail(i64),
    /// Refresh live recommendations
    RefreshLive,
    /// Load more live rooms
    LoadMoreLive,
    /// Play live stream
    PlayLive {
        room_id: i64,
        title: String,
        /// A quality the user picked; `None` plays the best within the cap.
        quality: Option<i64>,
    },
    /// Fetch the qualities a live room offers for the quality picker.
    LoadLiveQualities(i64),
    /// Send a danmaku to a live room.
    SendLiveDanmaku {
        room_id: i64,
        message: String,
    },
    /// Switch to bangumi page
    SwitchToBangumi,
    /// Refresh bangumi timeline
    RefreshBangumi,
    /// Switch bangumi tab
    SwitchBangumiTab(BangumiTab),
    /// Open bangumi detail page
    OpenBangumiDetail(i64),
    /// Load more bangumi index items
    LoadMoreBangumi,
    /// Play a bangumi episode
    PlayBangumiEpisode {
        ep_id: i64,
        season_id: i64,
        /// Season `type`, reported as the heartbeat `sub_type`.
        season_type: i32,
        aid: i64,
        cid: i64,
        bvid: String,
        title: String,
    },
    /// Stop the running player
    StopPlayback,
    /// Like, coin, favourite, follow or send danmaku on the open video.
    VideoAction(VideoActionRequest),
    /// No action
    None,
}

/// An account action on a video, sent to the server as one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoActionRequest {
    Like {
        aid: i64,
        like: bool,
    },
    Coin {
        aid: i64,
    },
    Favorite {
        aid: i64,
        favorite: bool,
    },
    Triple {
        aid: i64,
    },
    WatchLater {
        aid: i64,
    },
    Follow {
        aid: i64,
        mid: i64,
        follow: bool,
    },
    /// `progress_ms` is filled in by the app from the running player.
    SendDanmaku {
        aid: i64,
        cid: i64,
        message: String,
        progress_ms: i64,
    },
}

impl VideoActionRequest {
    pub fn aid(&self) -> i64 {
        match self {
            Self::Like { aid, .. }
            | Self::Coin { aid }
            | Self::Favorite { aid, .. }
            | Self::Triple { aid }
            | Self::WatchLater { aid }
            | Self::Follow { aid, .. }
            | Self::SendDanmaku { aid, .. } => *aid,
        }
    }
}

/// Bangumi page tabs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BangumiTab {
    Timeline,
    Index,
}
