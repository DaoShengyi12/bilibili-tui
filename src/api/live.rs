//! Bilibili Live Streaming API types and utilities

use serde::Deserialize;

/// Live room recommendation item from getMoreRecList API
#[derive(Debug, Clone, Deserialize)]
pub struct LiveRoom {
    pub roomid: i64,
    pub uid: i64,
    pub title: String,
    pub uname: String,
    pub face: String,
    pub cover: String,
    #[serde(default)]
    pub keyframe: String,
    pub online: i64,
    #[serde(default)]
    pub area_v2_name: String,
    #[serde(default)]
    pub area_v2_parent_name: String,
    #[serde(default)]
    pub watched_show: Option<WatchedShow>,
}

/// Watched count display info
#[derive(Debug, Clone, Deserialize)]
pub struct WatchedShow {
    pub num: i64,
    #[serde(default)]
    pub text_small: String,
}

/// Live recommendations response data
#[derive(Debug, Deserialize)]
pub struct LiveRecommendData {
    #[serde(default)]
    pub recommend_room_list: Vec<LiveRoom>,
}

#[derive(Debug, Deserialize)]
pub struct LiveHomeData {
    #[serde(default)]
    pub room_list: Vec<LiveHomeModule>,
}

#[derive(Debug, Deserialize)]
pub struct LiveHomeModule {
    pub module_info: LiveModuleInfo,
    #[serde(default)]
    pub list: Vec<LiveRoom>,
}

#[derive(Debug, Deserialize)]
pub struct LiveModuleInfo {
    pub title: String,
    #[serde(rename = "type")]
    pub module_type: i32,
}

impl LiveHomeData {
    pub fn followed_then_recommended(self) -> Vec<LiveRoom> {
        let mut followed = Vec::new();
        let mut recommended = Vec::new();
        for module in self.room_list {
            if module.module_info.module_type == 8 || module.module_info.title == "我的关注" {
                followed.extend(module.list);
            } else if module.module_info.module_type == 6 || module.module_info.title == "推荐直播"
            {
                recommended.extend(module.list);
            }
        }
        let mut seen = std::collections::HashSet::new();
        followed.extend(recommended);
        followed.retain(|room| seen.insert(room.roomid));
        followed
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LivePlayInfoData {
    pub live_status: i32,
    pub playurl_info: Option<LivePlayUrlInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LivePlayUrlInfo {
    pub playurl: LivePlayUrl,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LivePlayUrl {
    #[serde(default)]
    pub stream: Vec<LiveStream>,
    /// Names of the quality ladder, for example 10000 = "原画".
    #[serde(default, deserialize_with = "crate::api::serde_util::null_as_default")]
    pub g_qn_desc: Vec<LiveQnDesc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveQnDesc {
    pub qn: i64,
    #[serde(default)]
    pub desc: String,
}

/// A live quality the user can pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveQuality {
    pub qn: i64,
    pub label: String,
}

/// Fallback names of the live quality ladder.
fn live_qn_label(qn: i64) -> String {
    match qn {
        30000 => "杜比",
        20000 => "4K",
        10000 => "原画",
        400 => "蓝光",
        250 => "超清",
        150 => "高清",
        80 => "流畅",
        _ => return format!("qn {qn}"),
    }
    .to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveStream {
    pub protocol_name: String,
    #[serde(default)]
    pub format: Vec<LiveFormat>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveFormat {
    pub format_name: String,
    #[serde(default)]
    pub codec: Vec<LiveCodec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveCodec {
    pub codec_name: String,
    pub current_qn: i64,
    #[serde(default)]
    pub accept_qn: Vec<i64>,
    pub base_url: String,
    #[serde(default)]
    pub url_info: Vec<LiveUrlInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveUrlInfo {
    pub host: String,
    pub extra: String,
}

impl LivePlayInfoData {
    /// Preserve Bilibili's own stream/format/codec order. This is the safest
    /// default for playback; ranked alternatives are only failover choices.
    pub fn default_stream_urls(&self) -> Vec<String> {
        let Some(info) = &self.playurl_info else {
            return Vec::new();
        };
        info.playurl
            .stream
            .iter()
            .flat_map(|stream| &stream.format)
            .flat_map(|format| &format.codec)
            .flat_map(|codec| {
                codec
                    .url_info
                    .iter()
                    .map(|url| format!("{}{}{}", url.host, codec.base_url, url.extra))
            })
            .collect()
    }

    /// Advertised qualities within `cap`, best first.
    pub fn qualities_within(&self, cap: Option<i64>) -> Vec<LiveQuality> {
        let Some(info) = &self.playurl_info else {
            return Vec::new();
        };
        let mut qns: Vec<i64> = info
            .playurl
            .stream
            .iter()
            .flat_map(|stream| &stream.format)
            .flat_map(|format| &format.codec)
            .flat_map(|codec| codec.accept_qn.iter().copied())
            .filter(|quality| cap.is_none_or(|cap| *quality <= cap))
            .collect();
        qns.sort_unstable_by(|a, b| b.cmp(a));
        qns.dedup();
        qns.into_iter()
            .map(|qn| LiveQuality {
                qn,
                label: info
                    .playurl
                    .g_qn_desc
                    .iter()
                    .find(|desc| desc.qn == qn && !desc.desc.is_empty())
                    .map_or_else(|| live_qn_label(qn), |desc| desc.desc.clone()),
            })
            .collect()
    }

    /// Highest advertised quality that does not exceed `cap`.
    pub fn highest_quality_within(&self, cap: Option<i64>) -> Option<i64> {
        self.playurl_info
            .as_ref()?
            .playurl
            .stream
            .iter()
            .flat_map(|stream| &stream.format)
            .flat_map(|format| &format.codec)
            .flat_map(|codec| codec.accept_qn.iter().copied())
            .filter(|quality| cap.is_none_or(|cap| *quality <= cap))
            .max()
    }

    /// URLs for the best quality within `cap`. Never returns a stream above
    /// the cap; an empty list means nothing within the cap is available.
    pub fn stream_urls_within(&self, cap: Option<i64>) -> Vec<String> {
        let highest = self.highest_quality_within(cap);
        let selected = highest
            .map(|quality| self.stream_urls_for_quality(quality))
            .unwrap_or_default();
        if !selected.is_empty() {
            return selected;
        }

        // Some unauthenticated or geo-restricted responses advertise higher
        // values in `accept_qn` but return URLs only at the currently granted
        // `current_qn`. Keep the best URL that the response actually contains
        // instead of returning an empty list.
        self.current_quality()
            .filter(|quality| cap.is_none_or(|cap| *quality <= cap))
            .map(|quality| self.stream_urls_for_quality(quality))
            .unwrap_or_default()
    }

    fn current_quality(&self) -> Option<i64> {
        self.playurl_info
            .as_ref()?
            .playurl
            .stream
            .iter()
            .flat_map(|stream| &stream.format)
            .flat_map(|format| &format.codec)
            .map(|codec| codec.current_qn)
            .max()
    }

    fn stream_urls_for_quality(&self, quality: i64) -> Vec<String> {
        let Some(info) = &self.playurl_info else {
            return Vec::new();
        };
        let mut choices = info
            .playurl
            .stream
            .iter()
            .flat_map(|stream| {
                stream.format.iter().flat_map(move |format| {
                    format.codec.iter().filter_map(move |codec| {
                        (codec.current_qn == quality).then_some((
                            protocol_rank(&stream.protocol_name, &format.format_name),
                            codec_rank(&codec.codec_name),
                            codec,
                        ))
                    })
                })
            })
            .collect::<Vec<_>>();
        choices.sort_by_key(|(protocol, codec, _)| (*protocol, *codec));
        choices
            .into_iter()
            .flat_map(|(_, _, codec)| {
                codec
                    .url_info
                    .iter()
                    .map(|url| format!("{}{}{}", url.host, codec.base_url, url.extra))
            })
            .collect()
    }
}

fn protocol_rank(protocol: &str, format: &str) -> u8 {
    match (protocol, format) {
        // MPV handles Bilibili's continuous FLV live stream more reliably
        // than its short fMP4 playlists, which may surface as EOF every few
        // seconds and force a visible reload.
        ("http_stream", "flv") => 0,
        ("http_hls", "fmp4") => 1,
        ("http_hls", "ts") => 2,
        _ => 3,
    }
}

fn codec_rank(codec: &str) -> u8 {
    match codec {
        "avc" => 0,
        "hevc" => 1,
        "av1" => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    #[test]
    fn qualities_within_the_cap_are_listed_best_first_with_names() {
        let data: LivePlayInfoData = serde_json::from_value(serde_json::json!({
            "live_status": 1,
            "playurl_info": {"playurl": {
                "g_qn_desc": [{"qn": 10000, "desc": "原画"}, {"qn": 400, "desc": ""}],
                "stream": [{"protocol_name": "http_stream", "format": [{"format_name": "flv", "codec": [
                    {"codec_name": "avc", "current_qn": 10000, "accept_qn": [10000, 400, 250],
                     "base_url": "/a?", "url_info": []},
                    {"codec_name": "hevc", "current_qn": 10000, "accept_qn": [10000, 20000],
                     "base_url": "/b?", "url_info": []}
                ]}]}]
            }}
        }))
        .unwrap();
        let labels: Vec<(i64, String)> = data
            .qualities_within(Some(10000))
            .into_iter()
            .map(|q| (q.qn, q.label))
            .collect();
        assert_eq!(
            labels,
            [
                (10000, "原画".to_string()),
                (400, "蓝光".to_string()),
                (250, "超清".to_string())
            ]
        );
        assert_eq!(data.qualities_within(None)[0].qn, 20000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(roomid: i64) -> LiveRoom {
        LiveRoom {
            roomid,
            uid: roomid,
            title: roomid.to_string(),
            uname: String::new(),
            face: String::new(),
            cover: String::new(),
            keyframe: String::new(),
            online: 0,
            area_v2_name: String::new(),
            area_v2_parent_name: String::new(),
            watched_show: None,
        }
    }

    #[test]
    fn followed_rooms_are_first_and_duplicates_are_removed() {
        let data = LiveHomeData {
            room_list: vec![
                LiveHomeModule {
                    module_info: LiveModuleInfo {
                        title: "推荐直播".into(),
                        module_type: 6,
                    },
                    list: vec![room(2), room(1)],
                },
                LiveHomeModule {
                    module_info: LiveModuleInfo {
                        title: "我的关注".into(),
                        module_type: 8,
                    },
                    list: vec![room(1)],
                },
            ],
        };
        assert_eq!(
            data.followed_then_recommended()
                .into_iter()
                .map(|room| room.roomid)
                .collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn live_play_info_uses_highest_quality_and_prefers_avc() {
        let value = serde_json::json!({
            "live_status": 1,
            "playurl_info": {"playurl": {"stream": [{
                "protocol_name": "http_hls",
                "format": [{"format_name": "fmp4", "codec": [{
                    "codec_name": "avc", "current_qn": 20000,
                    "accept_qn": [10000, 20000], "base_url": "/best?",
                    "url_info": [{"host": "https://cdn.example", "extra": "token=1"}]
                }]}]
            }]}}
        });
        let data: LivePlayInfoData = serde_json::from_value(value).unwrap();
        assert_eq!(data.highest_quality_within(None), Some(20000));
        assert_eq!(
            data.stream_urls_within(None),
            ["https://cdn.example/best?token=1"]
        );
    }

    #[test]
    fn live_play_info_never_exceeds_quality_cap() {
        let value = serde_json::json!({
            "live_status": 1,
            "playurl_info": {"playurl": {"stream": [{
                "protocol_name": "http_stream",
                "format": [{"format_name": "flv", "codec": [
                    {"codec_name": "avc", "current_qn": 10000, "accept_qn": [10000, 400, 250],
                     "base_url": "/source", "url_info": [{"host": "https://cdn", "extra": ""}]},
                    {"codec_name": "avc", "current_qn": 250, "accept_qn": [10000, 400, 250],
                     "base_url": "/720", "url_info": [{"host": "https://cdn", "extra": ""}]}
                ]}]
            }]}}
        });
        let data: LivePlayInfoData = serde_json::from_value(value).unwrap();
        assert_eq!(data.highest_quality_within(Some(400)), Some(400));
        assert_eq!(data.stream_urls_within(Some(250)), ["https://cdn/720"]);
        // 400 is advertised but not returned, and the granted 10000 is above
        // the cap, so nothing may be selected.
        assert!(data.stream_urls_within(Some(400)).is_empty());
        assert!(data.stream_urls_within(Some(150)).is_empty());
    }

    #[test]
    fn live_play_info_prefers_continuous_flv_for_mpv() {
        let value = serde_json::json!({
            "live_status": 1,
            "playurl_info": {"playurl": {"stream": [
                {"protocol_name": "http_hls", "format": [{"format_name": "fmp4", "codec": [{
                    "codec_name": "avc", "current_qn": 10000, "accept_qn": [10000],
                    "base_url": "/hls", "url_info": [{"host": "https://hls", "extra": ""}]
                }]}]},
                {"protocol_name": "http_stream", "format": [{"format_name": "flv", "codec": [{
                    "codec_name": "avc", "current_qn": 10000, "accept_qn": [10000],
                    "base_url": "/live.flv", "url_info": [{"host": "https://flv", "extra": ""}]
                }]}]}
            ]}}
        });
        let data: LivePlayInfoData = serde_json::from_value(value).unwrap();
        assert_eq!(data.stream_urls_within(None)[0], "https://flv/live.flv");
    }

    #[test]
    fn live_play_info_falls_back_to_granted_quality_when_requested_quality_is_unavailable() {
        let value = serde_json::json!({
            "live_status": 1,
            "playurl_info": {"playurl": {"stream": [{
                "protocol_name": "http_hls",
                "format": [{"format_name": "fmp4", "codec": [{
                    "codec_name": "avc", "current_qn": 400,
                    "accept_qn": [10000, 400], "base_url": "/granted?",
                    "url_info": [{"host": "https://cdn.example", "extra": "token=2"}]
                }]}]
            }]}}
        });
        let data: LivePlayInfoData = serde_json::from_value(value).unwrap();
        assert_eq!(data.highest_quality_within(None), Some(10000));
        assert_eq!(
            data.stream_urls_within(None),
            ["https://cdn.example/granted?token=2"]
        );
    }
}

/// Live room detailed info from get_info API
#[derive(Debug, Clone, Deserialize)]
pub struct LiveRoomInfo {
    pub uid: i64,
    pub room_id: i64,
    #[serde(default)]
    pub short_id: i64,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// 0=未开播, 1=直播中, 2=轮播中
    pub live_status: i32,
    #[serde(default)]
    pub area_id: i64,
    #[serde(default)]
    pub area_name: String,
    #[serde(default)]
    pub parent_area_name: String,
    /// 关注数
    #[serde(default)]
    pub attention: i64,
    /// 在线人数
    #[serde(default)]
    pub online: i64,
    #[serde(default)]
    pub user_cover: String,
    #[serde(default)]
    pub keyframe: String,
    #[serde(default)]
    pub live_time: String,
    #[serde(default)]
    pub tags: String,
}

impl LiveRoomInfo {
    /// Get display cover URL
    pub fn cover_url(&self) -> &str {
        if !self.user_cover.is_empty() {
            &self.user_cover
        } else if !self.keyframe.is_empty() {
            &self.keyframe
        } else {
            ""
        }
    }

    /// Get live status text
    pub fn status_text(&self) -> &'static str {
        match self.live_status {
            0 => "未开播",
            1 => "直播中",
            2 => "轮播中",
            _ => "未知",
        }
    }
}
