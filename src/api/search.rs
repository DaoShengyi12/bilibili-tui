//! Search API types and functions

use serde::Deserialize;

/// Individual video search result
#[derive(Debug, Clone, Deserialize)]
pub struct SearchVideoItem {
    pub aid: Option<i64>,
    pub bvid: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub pic: Option<String>,
    pub play: Option<i64>,
    pub duration: Option<String>,
    pub mid: Option<i64>,
}

impl SearchVideoItem {
    pub fn display_title(&self) -> String {
        // Remove HTML tags like <em class="keyword">
        self.title
            .as_deref()
            .unwrap_or("无标题")
            .replace("<em class=\"keyword\">", "")
            .replace("</em>", "")
    }

    pub fn author_name(&self) -> &str {
        self.author.as_deref().unwrap_or("未知")
    }

    pub fn format_play(&self) -> String {
        match self.play {
            Some(n) if n >= 10000 => format!("{:.1}万", n as f64 / 10000.0),
            Some(n) => format!("{}", n),
            None => "-".to_string(),
        }
    }

    pub fn cover_url(&self) -> Option<String> {
        self.pic.as_ref().map(|url| {
            if url.starts_with("//") {
                format!("https:{}", url)
            } else {
                url.clone()
            }
        })
    }
}

/// Hot search item from web endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct HotwordItem {
    pub keyword: Option<String>,
    pub show_name: Option<String>,
    pub icon: Option<String>,
    pub pos: Option<i32>,
    pub word_type: Option<i32>,
}

impl HotwordItem {
    /// Display name prefers show_name over keyword
    pub fn display_text(&self) -> String {
        self.show_name
            .as_ref()
            .or(self.keyword.as_ref())
            .cloned()
            .unwrap_or_else(|| "-".to_string())
    }

    /// Keyword to trigger search
    pub fn keyword_text(&self) -> Option<String> {
        self.keyword
            .clone()
            .or_else(|| self.show_name.clone())
            .filter(|s| !s.is_empty())
    }

    /// Optional badge based on word_type
    pub fn badge(&self) -> Option<&'static str> {
        match self.word_type.unwrap_or_default() {
            4 => Some("新"),
            5 => Some("热"),
            7 => Some("直播"),
            9 => Some("梗"),
            11 => Some("话题"),
            12 => Some("独家"),
            _ => None,
        }
    }
}

/// Response for hot search list (web)
#[derive(Debug, Deserialize)]
pub struct HotwordResponse {
    pub code: Option<i32>,
    pub message: Option<String>,
    pub list: Option<Vec<HotwordItem>>, // Top 10 hot words
}

/// What a search looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchKind {
    #[default]
    Video,
    User,
    Bangumi,
    Live,
}

impl SearchKind {
    pub const ALL: [Self; 4] = [Self::Video, Self::User, Self::Bangumi, Self::Live];

    /// The `search_type` of `/x/web-interface/wbi/search/type`.
    pub const fn search_type(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::User => "bili_user",
            Self::Bangumi => "media_bangumi",
            Self::Live => "live_room",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Video => "视频",
            Self::User => "用户",
            Self::Bangumi => "番剧",
            Self::Live => "直播",
        }
    }

    pub fn cycle(self, step: isize) -> Self {
        let index = Self::ALL.iter().position(|kind| *kind == self).unwrap_or(0);
        let len = Self::ALL.len() as isize;
        Self::ALL[(index as isize + step).rem_euclid(len) as usize]
    }
}

/// One search result of any kind, reduced to what a result card shows.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResultItem {
    pub target: SearchTarget,
    pub title: String,
    pub subtitle: String,
    /// Short stats, such as plays or followers.
    pub stats: String,
    /// Duration, episode count or live state.
    pub badge: String,
    pub cover: Option<String>,
    pub uploader_mid: Option<i64>,
}

/// What opening a search result shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchTarget {
    Video { bvid: String, aid: i64 },
    User { mid: i64 },
    Bangumi { season_id: i64 },
    Live { room_id: i64 },
}

/// Remove the `<em class="keyword">` highlighting from titles.
pub fn strip_highlight(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    output.push_str(rest);
    output
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn https(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url.to_string()
    }
}

fn count(value: i64) -> String {
    if value >= 10000 {
        format!("{:.1}万", value as f64 / 10000.0)
    } else {
        value.to_string()
    }
}

fn int(item: &serde_json::Value, key: &str) -> Option<i64> {
    let value = item.get(key)?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|value| value as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn text(item: &serde_json::Value, key: &str) -> String {
    item.get(key)
        .and_then(serde_json::Value::as_str)
        .map(strip_highlight)
        .unwrap_or_default()
}

impl SearchResultItem {
    /// Read one entry of `data.result` for `kind`; entries without an ID
    /// are skipped.
    pub fn from_value(kind: SearchKind, item: &serde_json::Value) -> Option<Self> {
        Some(match kind {
            SearchKind::Video => {
                let video: SearchVideoItem = serde_json::from_value(item.clone()).ok()?;
                Self {
                    target: SearchTarget::Video {
                        bvid: video.bvid.clone()?,
                        aid: video.aid.unwrap_or_default(),
                    },
                    title: strip_highlight(&video.display_title()),
                    subtitle: video.author_name().to_string(),
                    stats: video.format_play(),
                    badge: video.duration.clone().unwrap_or_default(),
                    cover: video.cover_url(),
                    uploader_mid: video.mid,
                }
            }
            SearchKind::User => {
                let mid = int(item, "mid").filter(|mid| *mid > 0)?;
                Self {
                    target: SearchTarget::User { mid },
                    title: text(item, "uname"),
                    subtitle: text(item, "usign"),
                    stats: format!("粉丝 {}", count(int(item, "fans").unwrap_or(0))),
                    badge: format!("视频 {}", int(item, "videos").unwrap_or(0)),
                    cover: item.get("upic").and_then(|v| v.as_str()).map(https),
                    uploader_mid: Some(mid),
                }
            }
            SearchKind::Bangumi => {
                let season_id = int(item, "season_id").filter(|id| *id > 0)?;
                let score = item
                    .pointer("/media_score/score")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|score| *score > 0.0)
                    .map(|score| format!("★{score:.1}"))
                    .unwrap_or_default();
                Self {
                    target: SearchTarget::Bangumi { season_id },
                    title: text(item, "title"),
                    subtitle: text(item, "styles"),
                    stats: score,
                    badge: text(item, "index_show"),
                    cover: item.get("cover").and_then(|v| v.as_str()).map(https),
                    uploader_mid: None,
                }
            }
            SearchKind::Live => {
                let room_id = int(item, "roomid").filter(|id| *id > 0)?;
                let live = int(item, "live_status") == Some(1);
                Self {
                    target: SearchTarget::Live { room_id },
                    title: text(item, "title"),
                    subtitle: text(item, "uname"),
                    stats: format!("人气 {}", count(int(item, "online").unwrap_or(0))),
                    badge: if live { "直播中" } else { "未开播" }.to_string(),
                    cover: item
                        .get("user_cover")
                        .or_else(|| item.get("cover"))
                        .and_then(|v| v.as_str())
                        .filter(|url| !url.is_empty())
                        .map(https),
                    uploader_mid: int(item, "uid"),
                }
            }
        })
    }
}

/// Results and total count from the `data` of a typed search.
pub fn parse_search_data(
    kind: SearchKind,
    data: &serde_json::Value,
) -> anyhow::Result<(Vec<SearchResultItem>, i32)> {
    // Risk control answers with only a captcha voucher instead of results.
    if data.get("v_voucher").is_some() && data.get("result").is_none() {
        anyhow::bail!("搜索触发了 B 站风控验证，请稍后重试或登录后再搜索");
    }
    let total = data
        .get("numResults")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as i32;
    let items = data
        .get("result")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| SearchResultItem::from_value(kind, item))
                .collect()
        })
        .unwrap_or_default();
    Ok((items, total))
}

/// Suggested completions from `s.search.bilibili.com/main/suggest`, which
/// sends `result.tag` as a list or, in older replies, `result` as a map.
pub fn parse_suggestions(value: &serde_json::Value) -> Vec<String> {
    let result = value.get("result");
    let entries: Vec<&serde_json::Value> = match result {
        Some(serde_json::Value::Object(map)) => match map.get("tag") {
            Some(serde_json::Value::Array(tags)) => tags.iter().collect(),
            _ => map.values().collect(),
        },
        Some(serde_json::Value::Array(items)) => items.iter().collect(),
        _ => Vec::new(),
    };
    let mut suggestions: Vec<String> = Vec::new();
    for entry in entries {
        let Some(value) = entry
            .get("value")
            .or_else(|| entry.get("term"))
            .and_then(serde_json::Value::as_str)
            .map(strip_highlight)
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        if !suggestions.contains(&value) {
            suggestions.push(value);
        }
    }
    suggestions.truncate(10);
    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_tags_and_entities_are_removed() {
        assert_eq!(
            strip_highlight(r#"<em class="keyword">Rust</em> &amp; 入门"#),
            "Rust & 入门"
        );
        assert_eq!(strip_highlight("a < b"), "a < b");
    }

    #[test]
    fn user_bangumi_and_live_results_map_to_targets() {
        let user = SearchResultItem::from_value(
            SearchKind::User,
            &serde_json::json!({"mid": "7", "uname": "<em class=\"keyword\">UP</em>", "fans": 12345, "videos": 3, "upic": "//a.jpg"}),
        )
        .unwrap();
        assert_eq!(user.target, SearchTarget::User { mid: 7 });
        assert_eq!(user.title, "UP");
        assert_eq!(user.stats, "粉丝 1.2万");
        assert_eq!(user.cover.as_deref(), Some("https://a.jpg"));

        let bangumi = SearchResultItem::from_value(
            SearchKind::Bangumi,
            &serde_json::json!({"season_id": 33, "title": "番", "media_score": {"score": 9.7}, "index_show": "全12话"}),
        )
        .unwrap();
        assert_eq!(bangumi.target, SearchTarget::Bangumi { season_id: 33 });
        assert_eq!(bangumi.stats, "★9.7");

        let live = SearchResultItem::from_value(
            SearchKind::Live,
            &serde_json::json!({"roomid": 99, "title": "t", "uname": "u", "online": 5, "live_status": 1, "user_cover": ""}),
        )
        .unwrap();
        assert_eq!(live.target, SearchTarget::Live { room_id: 99 });
        assert_eq!(live.badge, "直播中");
        assert!(live.cover.is_none());

        assert!(
            SearchResultItem::from_value(SearchKind::User, &serde_json::json!({"uname": "x"}))
                .is_none()
        );
    }

    #[test]
    fn suggestions_accept_list_and_map_shapes() {
        let list = serde_json::json!({"result": {"tag": [
            {"value": "rust", "name": "<em>rust</em>"}, {"value": "rust"}, {"value": "rust 教程"}
        ]}});
        assert_eq!(parse_suggestions(&list), ["rust", "rust 教程"]);
        let map = serde_json::json!({"result": {"0": {"value": "a"}, "1": {"term": "b"}}});
        assert_eq!(parse_suggestions(&map), ["a", "b"]);
        assert!(parse_suggestions(&serde_json::json!({"result": null})).is_empty());
    }

    #[test]
    fn risk_control_voucher_is_an_error_and_no_results_are_empty() {
        assert!(
            parse_search_data(SearchKind::User, &serde_json::json!({"v_voucher": "x"})).is_err()
        );
        let (items, total) =
            parse_search_data(SearchKind::Bangumi, &serde_json::json!({"numResults": 0})).unwrap();
        assert!(items.is_empty() && total == 0);
    }

    #[test]
    fn search_kind_cycles_both_ways() {
        assert_eq!(SearchKind::Video.cycle(-1), SearchKind::Live);
        assert_eq!(SearchKind::Live.cycle(1), SearchKind::Video);
    }
}
