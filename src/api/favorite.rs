//! Favorite-folder and favorite-resource API models.

use super::serde_util::null_as_default;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FavoriteSource {
    WatchLater,
    Created {
        media_id: i64,
        title: String,
    },
    Collected {
        season_id: i64,
        mid: i64,
        title: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FavoriteOrder {
    RecentlyFavorited,
    MostPlayed,
    RecentlyPublished,
}

impl FavoriteOrder {
    pub fn api_value(self) -> &'static str {
        match self {
            Self::RecentlyFavorited => "mtime",
            Self::MostPlayed => "view",
            Self::RecentlyPublished => "pubtime",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::RecentlyFavorited => Self::MostPlayed,
            Self::MostPlayed => Self::RecentlyPublished,
            Self::RecentlyPublished => Self::RecentlyFavorited,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FavoriteOrder;

    #[test]
    fn favorite_sort_matches_web_query_values() {
        assert_eq!(FavoriteOrder::RecentlyFavorited.api_value(), "mtime");
        assert_eq!(FavoriteOrder::MostPlayed.api_value(), "view");
        assert_eq!(FavoriteOrder::RecentlyPublished.api_value(), "pubtime");
    }

    #[test]
    fn empty_folders_with_null_lists_decode_as_empty() {
        let resources: super::FavoriteResourceData = serde_json::from_value(serde_json::json!({
            "info": null,
            "medias": null,
            "has_more": false
        }))
        .unwrap();
        assert!(resources.medias.is_empty());

        let created: super::FavoriteFolderData =
            serde_json::from_value(serde_json::json!({"count": 0, "list": null})).unwrap();
        assert!(created.list.is_empty());

        let collected: super::CollectedFolderData = serde_json::from_value(
            serde_json::json!({"count": 0, "list": null, "has_more": false}),
        )
        .unwrap();
        assert!(collected.list.is_empty());

        let watch_later: super::WatchLaterData =
            serde_json::from_value(serde_json::json!({"count": 0, "list": null})).unwrap();
        assert!(watch_later.list.is_empty());

        let archives: super::SeasonArchivesData = serde_json::from_value(serde_json::json!({
            "archives": null,
            "page": {"page_num": 1, "total": 0}
        }))
        .unwrap();
        assert!(archives.archives.is_empty());
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FavoriteFolderData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub list: Vec<FavoriteFolder>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FavoriteFolder {
    pub id: i64,
    pub fid: Option<i64>,
    pub mid: i64,
    pub title: String,
    pub media_count: Option<i32>,
    pub fav_state: Option<i32>,
    pub attr: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FavoriteResourceData {
    pub info: Option<FavoriteInfo>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub medias: Vec<FavoriteMedia>,
    pub has_more: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FavoriteInfo {
    pub id: i64,
    pub title: String,
    pub media_count: Option<i32>,
    pub upper: Option<FavoriteUpper>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FavoriteMedia {
    pub id: i64,
    pub bvid: Option<String>,
    pub title: String,
    pub cover: Option<String>,
    pub duration: Option<i64>,
    pub upper: Option<FavoriteUpper>,
    pub cnt_info: Option<FavoriteCountInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FavoriteUpper {
    pub mid: i64,
    pub name: String,
    pub face: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FavoriteCountInfo {
    pub play: Option<i64>,
    pub danmaku: Option<i64>,
    pub collect: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchLaterData {
    pub count: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub list: Vec<WatchLaterItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchLaterItem {
    pub aid: i64,
    pub bvid: String,
    pub title: String,
    pub pic: Option<String>,
    pub duration: Option<i64>,
    pub owner: Option<FavoriteUpper>,
    pub stat: Option<WatchLaterStat>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchLaterStat {
    pub view: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectedFolderData {
    pub count: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub list: Vec<CollectedFolder>,
    pub has_more: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectedFolder {
    pub id: i64,
    pub mid: i64,
    pub title: String,
    pub media_count: Option<i32>,
    pub state: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeasonArchivesData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub archives: Vec<SeasonArchive>,
    pub page: SeasonPage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeasonArchive {
    pub aid: i64,
    pub bvid: String,
    pub title: String,
    pub pic: Option<String>,
    pub duration: Option<i64>,
    pub stat: Option<SeasonStat>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeasonStat {
    pub view: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeasonPage {
    pub page_num: i32,
    pub total: i64,
}
