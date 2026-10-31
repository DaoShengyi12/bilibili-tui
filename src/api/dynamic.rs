//! Dynamic feed API types and functions

use serde::{Deserialize, Deserializer};

fn deserialize_optional_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(serde_json::Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| serde::de::Error::custom("integer is out of range"))
            .map(Some),
        Some(serde_json::Value::String(value)) => value
            .parse::<i64>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected integer or numeric string, got {other}"
        ))),
    }
}

fn deserialize_optional_i32<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional_i64(deserializer)?
        .map(|value| i32::try_from(value).map_err(serde::de::Error::custom))
        .transpose()
}

fn deserialize_optional_f32<'de, D>(deserializer: D) -> Result<Option<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(number)) => number
            .as_f64()
            .map(|value| Some(value as f32))
            .ok_or_else(|| serde::de::Error::custom("invalid floating-point number")),
        Some(serde_json::Value::String(value)) => value
            .parse::<f32>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected number or numeric string, got {other}"
        ))),
    }
}

/// Dynamic feed response
#[derive(Debug, Deserialize)]
pub struct DynamicFeedData {
    pub items: Option<Vec<DynamicItem>>,
    pub offset: Option<String>,
    pub has_more: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_i32")]
    pub update_num: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::{DynamicFeedData, DynamicItem, DynamicKind};
    use serde::Deserialize;

    #[test]
    fn video_dynamic_comment_count_tolerates_drift() {
        let item = |stat: serde_json::Value| -> DynamicItem {
            serde_json::from_value(serde_json::json!({
                "id_str": "1",
                "type": "DYNAMIC_TYPE_AV",
                "modules": {"module_stat": stat}
            }))
            .unwrap()
        };
        assert_eq!(
            item(serde_json::json!({"comment": {"count": 12}})).comment_count(),
            Some(12)
        );
        assert_eq!(
            item(serde_json::json!({"comment": {"count": "34"}})).comment_count(),
            Some(34)
        );
        assert_eq!(
            item(serde_json::json!({"comment": {"count": null}})).comment_count(),
            None
        );
        assert_eq!(
            item(serde_json::json!({"comment": {"count": "x"}})).comment_count(),
            None
        );
        assert_eq!(item(serde_json::json!(null)).comment_count(), None);
    }

    #[test]
    fn video_and_opus_dynamics_comment_on_basic_comment_target() {
        let video: DynamicItem = serde_json::from_value(serde_json::json!({
            "id_str": "1262563193572556921",
            "type": "DYNAMIC_TYPE_AV",
            "basic": {"comment_type": 1, "comment_id_str": "117488511615141"},
            "modules": {"module_dynamic": {"major": {"type": "MAJOR_TYPE_ARCHIVE"}}}
        }))
        .unwrap();
        assert_eq!(video.comment_type(), 1);
        assert_eq!(
            video.comment_oid("1262563193572556921"),
            Some(117488511615141)
        );

        let opus: DynamicItem = serde_json::from_value(serde_json::json!({
            "id_str": "1262558061062520855",
            "type": "DYNAMIC_TYPE_DRAW",
            "basic": {"comment_type": 11, "comment_id_str": "413121832"},
            "modules": {"module_dynamic": {"major": {"type": "MAJOR_TYPE_OPUS"}}}
        }))
        .unwrap();
        assert_eq!(opus.comment_type(), 11);
        assert_eq!(opus.comment_oid("1262558061062520855"), Some(413121832));
    }

    #[test]
    fn dynamics_without_basic_keep_previous_comment_target() {
        let forward: DynamicItem = serde_json::from_value(serde_json::json!({
            "id_str": "1262523297597227011",
            "type": "DYNAMIC_TYPE_FORWARD",
            "modules": {"module_dynamic": {"major": null}}
        }))
        .unwrap();
        assert_eq!(forward.comment_type(), 17);
        assert_eq!(
            forward.comment_oid("1262523297597227011"),
            Some(1262523297597227011)
        );
    }

    #[test]
    fn forward_article_and_live_dynamics_are_classified() {
        let forward: DynamicItem = serde_json::from_value(serde_json::json!({
            "id_str": "1",
            "type": "DYNAMIC_TYPE_FORWARD",
            "modules": {"module_dynamic": {"desc": {"text": "转发理由"}, "major": null}},
            "orig": {
                "id_str": "2",
                "type": "DYNAMIC_TYPE_AV",
                "modules": {
                    "module_author": {"name": "原UP"},
                    "module_dynamic": {"major": {
                        "type": "MAJOR_TYPE_ARCHIVE",
                        "archive": {"bvid": "BV1x", "title": "原视频", "cover": "c.jpg"}
                    }}
                }
            }
        }))
        .unwrap();
        assert_eq!(forward.kind(), DynamicKind::Forward);
        let orig = forward.forwarded().unwrap();
        assert_eq!(orig.kind(), DynamicKind::Video);
        assert_eq!(orig.summary(), "原视频");
        assert_eq!(forward.cover().as_deref(), Some("c.jpg"));

        let article: DynamicItem = serde_json::from_value(serde_json::json!({
            "type": "DYNAMIC_TYPE_ARTICLE",
            "modules": {"module_dynamic": {"major": {
                "type": "MAJOR_TYPE_ARTICLE",
                "article": {"id": "123", "title": "专栏标题", "covers": ["a.jpg"]}
            }}}
        }))
        .unwrap();
        assert_eq!(article.kind(), DynamicKind::Article);
        assert_eq!(article.article(), Some((123, "专栏标题")));
        assert_eq!(article.cover().as_deref(), Some("a.jpg"));

        let content = serde_json::json!({"live_play_info": {
            "room_id": 456, "title": "直播中", "cover": "l.jpg", "live_status": 1,
            "watched_show": {"text_large": "1万人看过"}
        }})
        .to_string();
        let live: DynamicItem = serde_json::from_value(serde_json::json!({
            "type": "DYNAMIC_TYPE_LIVE_RCMD",
            "modules": {"module_dynamic": {"major": {
                "type": "MAJOR_TYPE_LIVE_RCMD",
                "live_rcmd": {"content": content}
            }}}
        }))
        .unwrap();
        assert_eq!(live.kind(), DynamicKind::Live);
        let room = live.live().unwrap();
        assert_eq!((room.room_id, room.is_live), (456, true));
        assert_eq!(room.watched.as_deref(), Some("1万人看过"));
    }

    #[test]
    fn empty_numeric_strings_read_as_missing() {
        let item: DynamicItem = serde_json::from_value(serde_json::json!({
            "basic": {"comment_type": "", "comment_id_str": ""},
            "modules": {"module_author": {"pub_ts": ""}}
        }))
        .unwrap();
        assert!(item.basic.unwrap().comment_id.is_none());
    }

    #[test]
    fn unreadable_live_content_is_not_a_live_dynamic() {
        let live: DynamicItem = serde_json::from_value(serde_json::json!({
            "modules": {"module_dynamic": {"major": {
                "type": "MAJOR_TYPE_LIVE_RCMD",
                "live_rcmd": {"content": "not json"}
            }}}
        }))
        .unwrap();
        assert_eq!(live.kind(), DynamicKind::Other);
    }

    #[test]
    fn feed_accepts_string_update_num() {
        let data: DynamicFeedData =
            serde_json::from_str(r#"{"items":[],"offset":"1","has_more":true,"update_num":"0"}"#)
                .expect("string update_num should deserialize");

        assert_eq!(data.update_num, Some(0));
    }

    #[test]
    fn feed_accepts_numeric_fields_as_numbers_strings_and_null() {
        let json = r#"{
          "items": [{
            "id_str": "1",
            "type": "DYNAMIC_TYPE_DRAW",
            "modules": {
              "module_author": {"pub_ts": "1720000000"},
              "module_dynamic": {"major": {
                "type": "MAJOR_TYPE_DRAW",
                "draw": {"id": "42", "items": [
                  {"src": "a", "height": "1080", "width": 1920, "size": "1.25"},
                  {"src": "b", "height": null, "width": null, "size": 2.5}
                ]}
              }}
            }
          }],
          "offset": null,
          "has_more": false,
          "update_num": 0
        }"#;
        let data: DynamicFeedData = serde_json::from_str(json).expect("mixed numeric forms");
        let item = &data.items.expect("items")[0];
        let modules = item.modules.as_ref().expect("modules");
        assert_eq!(
            modules.module_author.as_ref().and_then(|a| a.pub_ts),
            Some(1_720_000_000)
        );
        let draw = modules
            .module_dynamic
            .as_ref()
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.draw.as_ref())
            .expect("draw");
        assert_eq!(draw.id, Some(42));
        let pictures = draw.items.as_ref().expect("pictures");
        assert_eq!(
            (pictures[0].height, pictures[0].width, pictures[0].size),
            (Some(1080), Some(1920), Some(1.25))
        );
        assert_eq!(
            (pictures[1].height, pictures[1].width, pictures[1].size),
            (None, None, Some(2.5))
        );
    }

    #[test]
    fn feed_rejects_invalid_or_out_of_range_numeric_strings() {
        let invalid = r#"{"items":[{"modules":{"module_author":{"pub_ts":"not-a-number"}}}]}"#;
        assert!(serde_json::from_str::<DynamicFeedData>(invalid).is_err());
        let overflow = r#"{"items":[{"modules":{"module_dynamic":{"major":{"draw":{"items":[{"height":"2147483648"}]}}}}}]}"#;
        assert!(serde_json::from_str::<DynamicFeedData>(overflow).is_err());
    }

    #[test]
    fn saved_real_feed_response_deserializes_when_fixture_is_available() {
        let Ok(path) = std::env::var("BILIBILI_DYNAMIC_FIXTURE") else {
            return;
        };
        #[derive(Deserialize)]
        struct Response {
            data: DynamicFeedData,
        }

        let body = std::fs::read(path).expect("read dynamic fixture");
        let response: Response =
            serde_json::from_slice(&body).expect("deserialize dynamic fixture");
        assert!(response.data.items.is_some());
    }
}

/// Portal data response (frequently watched UPs)
#[derive(Debug, Deserialize)]
pub struct PortalData {
    pub up_list: Option<Vec<UpListItem>>,
    // my_info and live_users are available but not needed for now
}

/// UP master in portal up_list
#[derive(Debug, Clone, Deserialize)]
pub struct UpListItem {
    pub mid: i64,
    pub uname: String,
    pub face: String,
    pub has_update: bool,
    #[serde(default)]
    pub is_reserve_recall: bool,
}

/// Individual dynamic item
#[derive(Debug, Clone, Deserialize)]
pub struct DynamicItem {
    pub id_str: Option<String>,
    #[serde(rename = "type")]
    pub dynamic_type: Option<String>,
    pub modules: Option<DynamicModules>,
    #[serde(default)]
    pub basic: Option<DynamicBasic>,
    /// The forwarded dynamic of a `DYNAMIC_TYPE_FORWARD`.
    #[serde(default)]
    pub orig: Option<Box<DynamicItem>>,
}

/// What a dynamic shows, as far as this client can display it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicKind {
    Video,
    Draw,
    Opus,
    Article,
    Live,
    Forward,
    /// Plain text without a major.
    Word,
    Other,
}

/// A live room shared in a dynamic, from `MAJOR_TYPE_LIVE_RCMD` (whose
/// `content` is a JSON string) or the older `MAJOR_TYPE_LIVE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicLive {
    pub room_id: i64,
    pub title: String,
    pub cover: Option<String>,
    pub is_live: bool,
    /// For example "1.2万人看过".
    pub watched: Option<String>,
}

/// Server-provided comment target. Video dynamics comment on the aid and
/// opus/draw dynamics on their rid, so the dynamic id is not always the oid.
#[derive(Debug, Clone, Deserialize)]
pub struct DynamicBasic {
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    pub comment_type: Option<i64>,
    #[serde(
        default,
        rename = "comment_id_str",
        deserialize_with = "deserialize_optional_i64"
    )]
    pub comment_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DynamicModules {
    pub module_author: Option<ModuleAuthor>,
    pub module_dynamic: Option<ModuleDynamic>,
    #[serde(default)]
    pub module_stat: Option<ModuleStat>,
}

/// Interaction counts under a dynamic.
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleStat {
    #[serde(default)]
    pub comment: Option<StatCount>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatCount {
    #[serde(default, deserialize_with = "super::serde_util::lenient_optional_i64")]
    pub count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModuleAuthor {
    pub name: Option<String>,
    pub face: Option<String>,
    pub mid: Option<i64>,
    pub pub_time: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    pub pub_ts: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModuleDynamic {
    pub major: Option<DynamicMajor>,
    pub desc: Option<DynamicDesc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DynamicMajor {
    #[serde(rename = "type")]
    pub major_type: Option<String>,
    pub archive: Option<ArchiveInfo>,
    pub draw: Option<DrawInfo>,
    pub opus: Option<OpusInfo>,
    #[serde(default)]
    pub article: Option<ArticleMajor>,
    #[serde(default)]
    pub live_rcmd: Option<LiveRcmdMajor>,
    #[serde(default)]
    pub live: Option<LiveMajor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArticleMajor {
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    pub id: Option<i64>,
    pub title: Option<String>,
    pub desc: Option<String>,
    #[serde(default)]
    pub covers: Option<Vec<String>>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveRcmdMajor {
    /// JSON text holding `live_play_info`.
    pub content: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveMajor {
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    pub id: Option<i64>,
    pub title: Option<String>,
    pub cover: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    pub live_state: Option<i64>,
    pub desc_first: Option<String>,
}

#[derive(Deserialize)]
struct LiveRcmdContent {
    live_play_info: LivePlayInfo,
}

#[derive(Deserialize)]
struct LivePlayInfo {
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    room_id: Option<i64>,
    title: Option<String>,
    cover: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    live_status: Option<i64>,
    #[serde(default)]
    watched_show: Option<WatchedShow>,
}

#[derive(Deserialize)]
struct WatchedShow {
    text_large: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArchiveInfo {
    pub bvid: Option<String>,
    pub title: Option<String>,
    pub cover: Option<String>,
    pub desc: Option<String>,
    pub duration_text: Option<String>,
    pub stat: Option<ArchiveStat>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArchiveStat {
    pub play: Option<String>,
    pub danmaku: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DynamicDesc {
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DrawInfo {
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    pub id: Option<i64>,
    pub items: Option<Vec<DrawItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DrawItem {
    pub src: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_i32")]
    pub height: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_optional_i32")]
    pub width: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_optional_f32")]
    pub size: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpusInfo {
    pub title: Option<String>,
    pub summary: Option<OpusSummary>,
    pub pics: Option<Vec<OpusPic>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpusSummary {
    pub text: Option<String>,
    pub rich_text_nodes: Option<Vec<RichTextNode>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpusPic {
    pub url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_i32")]
    pub height: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_optional_i32")]
    pub width: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_optional_f32")]
    pub size: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RichTextNode {
    pub text: Option<String>,
    #[serde(rename = "type")]
    pub node_type: Option<String>,
}

impl DynamicItem {
    fn major(&self) -> Option<&DynamicMajor> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
    }

    pub fn kind(&self) -> DynamicKind {
        if self.dynamic_type.as_deref() == Some("DYNAMIC_TYPE_FORWARD") {
            return DynamicKind::Forward;
        }
        match self.major().and_then(|m| m.major_type.as_deref()) {
            Some("MAJOR_TYPE_ARCHIVE") => DynamicKind::Video,
            Some("MAJOR_TYPE_DRAW") => DynamicKind::Draw,
            Some("MAJOR_TYPE_OPUS") => DynamicKind::Opus,
            Some("MAJOR_TYPE_ARTICLE") => DynamicKind::Article,
            Some("MAJOR_TYPE_LIVE_RCMD" | "MAJOR_TYPE_LIVE") if self.live().is_some() => {
                DynamicKind::Live
            }
            None if self.desc_text().is_some_and(|text| !text.is_empty()) => DynamicKind::Word,
            _ => DynamicKind::Other,
        }
    }

    /// The forwarded dynamic, when this is a forward.
    pub fn forwarded(&self) -> Option<&DynamicItem> {
        self.orig.as_deref()
    }

    /// `(cvid, title)` of an article dynamic.
    pub fn article(&self) -> Option<(i64, &str)> {
        let article = self.major()?.article.as_ref()?;
        let cvid = article.id.filter(|id| *id > 0)?;
        Some((cvid, article.title.as_deref().unwrap_or("专栏")))
    }

    pub fn article_desc(&self) -> Option<&str> {
        self.major()?.article.as_ref()?.desc.as_deref()
    }

    pub fn article_cover(&self) -> Option<&str> {
        self.major()?
            .article
            .as_ref()?
            .covers
            .as_ref()?
            .first()
            .map(String::as_str)
    }

    pub fn live(&self) -> Option<DynamicLive> {
        let major = self.major()?;
        if let Some(content) = major.live_rcmd.as_ref().and_then(|l| l.content.as_deref()) {
            let info = serde_json::from_str::<LiveRcmdContent>(content)
                .ok()?
                .live_play_info;
            return Some(DynamicLive {
                room_id: info.room_id.filter(|id| *id > 0)?,
                title: info.title.unwrap_or_default(),
                cover: info.cover,
                is_live: info.live_status == Some(1),
                watched: info.watched_show.and_then(|show| show.text_large),
            });
        }
        let live = major.live.as_ref()?;
        Some(DynamicLive {
            room_id: live.id.filter(|id| *id > 0)?,
            title: live.title.clone().unwrap_or_default(),
            cover: live.cover.clone(),
            is_live: live.live_state == Some(1),
            watched: live.desc_first.clone(),
        })
    }

    /// One-line summary used for cards and forwarded content.
    pub fn summary(&self) -> String {
        match self.kind() {
            DynamicKind::Video => self.video_title().unwrap_or("视频").to_string(),
            DynamicKind::Draw => self.desc_text().unwrap_or("图片动态").to_string(),
            DynamicKind::Opus => self
                .opus_title()
                .or(self.opus_text())
                .unwrap_or("图文动态")
                .to_string(),
            DynamicKind::Article => self
                .article()
                .map_or("专栏", |(_, title)| title)
                .to_string(),
            DynamicKind::Live => self.live().map(|live| live.title).unwrap_or_default(),
            DynamicKind::Forward | DynamicKind::Word | DynamicKind::Other => {
                self.desc_text().unwrap_or("动态").to_string()
            }
        }
    }

    /// The first picture to show for this dynamic.
    pub fn cover(&self) -> Option<String> {
        match self.kind() {
            DynamicKind::Video => self.video_cover().map(str::to_string),
            DynamicKind::Draw => self.draw_images().first().map(|url| url.to_string()),
            DynamicKind::Opus => self.opus_images().first().map(|url| url.to_string()),
            DynamicKind::Article => self.article_cover().map(str::to_string),
            DynamicKind::Live => self.live()?.cover,
            DynamicKind::Forward => self.forwarded()?.cover(),
            DynamicKind::Word | DynamicKind::Other => None,
        }
    }

    pub fn opus_title(&self) -> Option<&str> {
        self.major()?
            .opus
            .as_ref()?
            .title
            .as_deref()
            .filter(|title| !title.is_empty())
    }

    pub fn is_video(&self) -> bool {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.major_type.as_ref())
            .map(|t| t == "MAJOR_TYPE_ARCHIVE")
            .unwrap_or(false)
    }

    pub fn author_name(&self) -> &str {
        self.modules
            .as_ref()
            .and_then(|m| m.module_author.as_ref())
            .and_then(|a| a.name.as_deref())
            .unwrap_or("未知")
    }

    pub fn author_face(&self) -> Option<&str> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_author.as_ref())
            .and_then(|a| a.face.as_deref())
    }

    pub fn author_mid(&self) -> Option<i64> {
        self.modules
            .as_ref()
            .and_then(|modules| modules.module_author.as_ref())
            .and_then(|author| author.mid)
    }

    pub fn pub_time(&self) -> &str {
        self.modules
            .as_ref()
            .and_then(|m| m.module_author.as_ref())
            .and_then(|a| a.pub_time.as_deref())
            .unwrap_or("")
    }

    pub fn video_title(&self) -> Option<&str> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.archive.as_ref())
            .and_then(|a| a.title.as_deref())
    }

    pub fn video_cover(&self) -> Option<&str> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.archive.as_ref())
            .and_then(|a| a.cover.as_deref())
    }

    pub fn video_bvid(&self) -> Option<&str> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.archive.as_ref())
            .and_then(|a| a.bvid.as_deref())
    }

    pub fn video_play(&self) -> &str {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.archive.as_ref())
            .and_then(|a| a.stat.as_ref())
            .and_then(|s| s.play.as_deref())
            .unwrap_or("-")
    }

    pub fn video_danmaku(&self) -> &str {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.archive.as_ref())
            .and_then(|a| a.stat.as_ref())
            .and_then(|s| s.danmaku.as_deref())
            .unwrap_or("-")
    }

    /// Comments under the dynamic; for a video dynamic, the video's replies.
    pub fn comment_count(&self) -> Option<i64> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_stat.as_ref())
            .and_then(|s| s.comment.as_ref())
            .and_then(|c| c.count)
    }

    pub fn video_duration(&self) -> &str {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.archive.as_ref())
            .and_then(|a| a.duration_text.as_deref())
            .unwrap_or("")
    }

    pub fn is_draw(&self) -> bool {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.major_type.as_ref())
            .map(|t| t == "MAJOR_TYPE_DRAW")
            .unwrap_or(false)
    }

    pub fn is_opus(&self) -> bool {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.major_type.as_ref())
            .map(|t| t == "MAJOR_TYPE_OPUS")
            .unwrap_or(false)
    }

    pub fn draw_images(&self) -> Vec<&str> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.draw.as_ref())
            .and_then(|draw| draw.items.as_ref())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.src.as_deref())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn opus_text(&self) -> Option<&str> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.opus.as_ref())
            .and_then(|opus| opus.summary.as_ref())
            .and_then(|summary| summary.text.as_deref())
    }

    pub fn opus_images(&self) -> Vec<&str> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.opus.as_ref())
            .and_then(|opus| opus.pics.as_ref())
            .map(|pics| pics.iter().filter_map(|pic| pic.url.as_deref()).collect())
            .unwrap_or_default()
    }

    pub fn desc_text(&self) -> Option<&str> {
        self.modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.desc.as_ref())
            .and_then(|desc| desc.text.as_deref())
    }

    /// Get the correct comment type code for this dynamic
    /// Type 11: 相簿（图片动态） - image/photo albums (MAJOR_TYPE_DRAW, MAJOR_TYPE_OPUS)
    /// Type 17: 动态（纯文字动态&分享） - text dynamics and shares
    /// Type 1: 视频 - videos (MAJOR_TYPE_ARCHIVE)
    pub fn comment_type(&self) -> i32 {
        if let Some(comment_type) = self
            .basic
            .as_ref()
            .and_then(|basic| basic.comment_type)
            .and_then(|value| i32::try_from(value).ok())
            .filter(|value| *value > 0)
        {
            return comment_type;
        }
        if let Some(major_type) = self
            .modules
            .as_ref()
            .and_then(|m| m.module_dynamic.as_ref())
            .and_then(|d| d.major.as_ref())
            .and_then(|m| m.major_type.as_ref())
        {
            match major_type.as_str() {
                "MAJOR_TYPE_DRAW" | "MAJOR_TYPE_OPUS" => 11, // 相簿（图片动态）
                "MAJOR_TYPE_ARCHIVE" => 1,                   // 视频
                _ => 17,                                     // 其他类型的动态（纯文字、转发等）
            }
        } else {
            17 // 默认使用类型17
        }
    }

    /// Get the correct oid for comments
    /// For image dynamics (MAJOR_TYPE_DRAW), use the draw.id
    /// For other types, use the dynamic id itself
    pub fn comment_oid(&self, dynamic_id: &str) -> Option<i64> {
        if let Some(oid) = self
            .basic
            .as_ref()
            .and_then(|basic| basic.comment_id)
            .filter(|oid| *oid > 0)
        {
            return Some(oid);
        }
        if self.is_draw() {
            // For draw type, use the draw.id as oid
            self.modules
                .as_ref()
                .and_then(|m| m.module_dynamic.as_ref())
                .and_then(|d| d.major.as_ref())
                .and_then(|m| m.draw.as_ref())
                .and_then(|draw| draw.id)
        } else {
            // For other types, use the dynamic_id
            dynamic_id.parse::<i64>().ok()
        }
    }
}

/// Following users response
#[derive(Debug, Deserialize)]
pub struct FollowingsData {
    pub list: Option<Vec<FollowingUser>>,
    pub total: Option<i32>,
}

/// Following user info
#[derive(Debug, Clone, Deserialize)]
pub struct FollowingUser {
    pub mid: Option<i64>,
    pub uname: Option<String>,
    pub face: Option<String>,
    pub sign: Option<String>,
}
