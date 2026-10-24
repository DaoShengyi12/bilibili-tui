//! Account actions on videos: like, coin, favourite, triple, watch later,
//! follow, danmaku, plus the player info used for subtitles and resume.

use super::{ApiClient, ApiResponse, BilibiliApiDomain};
use crate::api::serde_util::{lenient_flag, lenient_i64, null_as_default};
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

/// The logged-in user's relation to a video.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct ArchiveRelation {
    /// Following the uploader.
    #[serde(default, deserialize_with = "lenient_flag")]
    pub attention: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub favorite: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub like: bool,
    /// Coins given, 0 to 2.
    #[serde(default, deserialize_with = "lenient_i64")]
    pub coin: i64,
}

/// What a one-click triple actually did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct TripleResult {
    #[serde(default, deserialize_with = "lenient_flag")]
    pub like: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub coin: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub fav: bool,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub multiply: i64,
}

/// A CC or AI subtitle track of a video part.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubtitleTrack {
    #[serde(default)]
    pub lan: String,
    #[serde(default)]
    pub lan_doc: String,
    #[serde(default)]
    pub subtitle_url: String,
}

impl SubtitleTrack {
    /// AI-generated tracks are loaded but not selected by default.
    pub fn is_ai(&self) -> bool {
        self.lan.starts_with("ai-")
    }
}

/// The parts of `/x/player/wbi/v2` the player uses.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlayerInfo {
    /// Milliseconds into `last_play_cid`; 0 or negative when there is
    /// nothing to resume.
    #[serde(default, deserialize_with = "lenient_i64")]
    pub last_play_time: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub last_play_cid: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub subtitle: PlayerSubtitles,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlayerSubtitles {
    #[serde(default, deserialize_with = "null_as_default")]
    pub subtitles: Vec<SubtitleTrack>,
}

impl PlayerInfo {
    /// Seconds to resume `cid` from, skipping the first few seconds.
    pub fn resume_position(&self, cid: i64) -> Option<f64> {
        (self.last_play_cid == cid && self.last_play_time > 5_000)
            .then(|| self.last_play_time as f64 / 1000.0)
    }
}

#[derive(Deserialize)]
struct SubtitleBody {
    #[serde(default, deserialize_with = "null_as_default")]
    body: Vec<SubtitleLine>,
}

#[derive(Deserialize)]
struct SubtitleLine {
    #[serde(default)]
    from: f64,
    #[serde(default)]
    to: f64,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct FolderList {
    #[serde(default, deserialize_with = "null_as_default")]
    list: Vec<FolderState>,
}

#[derive(Deserialize)]
struct FolderState {
    id: i64,
    #[serde(default, deserialize_with = "lenient_flag")]
    fav_state: bool,
}

fn srt_time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1000 % 60,
        millis % 1000
    )
}

/// Convert Bilibili's JSON subtitle body to SRT.
pub fn subtitle_json_to_srt(json: &[u8]) -> Result<String> {
    let body: SubtitleBody = serde_json::from_slice(json).context("invalid subtitle JSON")?;
    let mut srt = String::new();
    for (index, line) in body.body.iter().enumerate() {
        srt.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            srt_time(line.from),
            srt_time(line.to),
            line.content
        ));
    }
    Ok(srt)
}

impl ApiClient {
    fn require_login(&self) -> Result<()> {
        self.csrf_token()
            .map(|_| ())
            .ok_or_else(|| anyhow!("该操作需要登录"))
    }

    pub async fn get_archive_relation(&self, aid: i64) -> Result<ArchiveRelation> {
        let url = format!(
            "{}/x/web-interface/archive/relation?aid={aid}",
            BilibiliApiDomain::Main.as_str()
        );
        let resp: ApiResponse<ArchiveRelation> = self.get(&url).await?;
        Ok(resp.data.unwrap_or_default())
    }

    pub async fn like_video(&self, aid: i64, like: bool) -> Result<()> {
        self.require_login()?;
        let url = self.build_url(BilibiliApiDomain::Main, "/x/web-interface/archive/like");
        let _: ApiResponse<serde_json::Value> = self
            .post(
                &url,
                vec![
                    ("aid", aid.to_string()),
                    ("like", if like { "1" } else { "2" }.to_string()),
                ],
            )
            .await?;
        Ok(())
    }

    /// Give one coin without also liking.
    pub async fn coin_video(&self, aid: i64) -> Result<()> {
        self.require_login()?;
        let url = self.build_url(BilibiliApiDomain::Main, "/x/web-interface/coin/add");
        let _: ApiResponse<serde_json::Value> = self
            .post(
                &url,
                vec![
                    ("aid", aid.to_string()),
                    ("multiply", "1".to_string()),
                    ("select_like", "0".to_string()),
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn triple_video(&self, aid: i64) -> Result<TripleResult> {
        self.require_login()?;
        let url = self.build_url(
            BilibiliApiDomain::Main,
            "/x/web-interface/archive/like/triple",
        );
        let resp: ApiResponse<TripleResult> =
            self.post(&url, vec![("aid", aid.to_string())]).await?;
        Ok(resp.data.unwrap_or_default())
    }

    /// Add the video to the user's default folder, or remove it from every
    /// folder that holds it. Returns whether it is favourited afterwards.
    pub async fn set_video_favorite(
        &self,
        aid: i64,
        user_mid: i64,
        favorite: bool,
    ) -> Result<bool> {
        self.require_login()?;
        let url = format!(
            "{}/x/v3/fav/folder/created/list-all?type=2&rid={aid}&up_mid={user_mid}",
            BilibiliApiDomain::Main.as_str()
        );
        let resp: ApiResponse<FolderList> = self.get(&url).await?;
        let folders = resp.data.map(|data| data.list).unwrap_or_default();
        let (add, del): (Vec<i64>, Vec<i64>) = if favorite {
            let default = folders.first().ok_or_else(|| anyhow!("没有可用的收藏夹"))?;
            (vec![default.id], Vec::new())
        } else {
            (
                Vec::new(),
                folders
                    .iter()
                    .filter(|folder| folder.fav_state)
                    .map(|folder| folder.id)
                    .collect(),
            )
        };
        if add.is_empty() && del.is_empty() {
            return Ok(favorite);
        }
        let join = |ids: &[i64]| ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let url = self.build_url(BilibiliApiDomain::Main, "/x/v3/fav/resource/deal");
        let _: ApiResponse<serde_json::Value> = self
            .post(
                &url,
                vec![
                    ("rid", aid.to_string()),
                    ("type", "2".to_string()),
                    ("add_media_ids", join(&add)),
                    ("del_media_ids", join(&del)),
                ],
            )
            .await?;
        Ok(favorite)
    }

    pub async fn add_to_watch_later(&self, aid: i64) -> Result<()> {
        self.require_login()?;
        let url = self.build_url(BilibiliApiDomain::Main, "/x/v2/history/toview/add");
        let _: ApiResponse<serde_json::Value> =
            self.post(&url, vec![("aid", aid.to_string())]).await?;
        Ok(())
    }

    pub async fn set_following(&self, mid: i64, follow: bool) -> Result<()> {
        self.require_login()?;
        let url = self.build_url(BilibiliApiDomain::Main, "/x/relation/modify");
        let _: ApiResponse<serde_json::Value> = self
            .post(
                &url,
                vec![
                    ("fid", mid.to_string()),
                    ("act", if follow { "1" } else { "2" }.to_string()),
                    ("re_src", "11".to_string()),
                ],
            )
            .await?;
        Ok(())
    }

    /// Send a scrolling danmaku `progress_ms` into part `cid`.
    pub async fn send_video_danmaku(
        &self,
        aid: i64,
        cid: i64,
        progress_ms: i64,
        message: &str,
    ) -> Result<()> {
        self.require_login()?;
        let url = self.build_url(BilibiliApiDomain::Main, "/x/v2/dm/post");
        let _: ApiResponse<serde_json::Value> = self
            .post(
                &url,
                vec![
                    ("type", "1".to_string()),
                    ("oid", cid.to_string()),
                    ("aid", aid.to_string()),
                    ("msg", message.to_string()),
                    ("progress", progress_ms.max(0).to_string()),
                    ("color", "16777215".to_string()),
                    ("fontsize", "25".to_string()),
                    ("pool", "0".to_string()),
                    ("mode", "1".to_string()),
                    ("plat", "1".to_string()),
                    ("rnd", chrono::Utc::now().timestamp_micros().to_string()),
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn get_player_info(&self, aid: i64, cid: i64) -> Result<PlayerInfo> {
        let url = self.build_url(BilibiliApiDomain::Main, "/x/player/wbi/v2");
        let resp: ApiResponse<PlayerInfo> = self
            .get_with_wbi(
                &url,
                vec![("aid", aid.to_string()), ("cid", cid.to_string())],
            )
            .await?;
        Ok(resp.data.unwrap_or_default())
    }

    /// Download a subtitle track as SRT.
    pub async fn get_subtitle_srt(&self, track: &SubtitleTrack) -> Result<String> {
        let url = if track.subtitle_url.starts_with("//") {
            format!("https:{}", track.subtitle_url)
        } else {
            track.subtitle_url.clone()
        };
        let body = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("subtitle request failed")?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?;
        subtitle_json_to_srt(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_reads_numbers_and_bools() {
        let relation: ArchiveRelation = serde_json::from_value(serde_json::json!({
            "attention": true, "favorite": 0, "like": 1, "coin": "2", "dislike": false
        }))
        .unwrap();
        assert_eq!(
            relation,
            ArchiveRelation {
                attention: true,
                favorite: false,
                like: true,
                coin: 2
            }
        );
    }

    #[test]
    fn subtitles_convert_to_srt() {
        let srt = subtitle_json_to_srt(
            br#"{"body":[{"from":1.5,"to":3725.25,"content":"hello"},{"from":4,"to":5,"content":"x"}]}"#,
        )
        .unwrap();
        assert!(srt.starts_with("1\n00:00:01,500 --> 01:02:05,250\nhello\n\n2\n"));
        assert!(
            subtitle_json_to_srt(br#"{"body":null}"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn resume_needs_the_same_part_and_some_progress() {
        let info: PlayerInfo = serde_json::from_value(serde_json::json!({
            "last_play_time": 125_000, "last_play_cid": 7, "subtitle": null
        }))
        .unwrap();
        assert_eq!(info.resume_position(7), Some(125.0));
        assert_eq!(info.resume_position(8), None);
        let start: PlayerInfo =
            serde_json::from_value(serde_json::json!({"last_play_time": 3000, "last_play_cid": 7}))
                .unwrap();
        assert_eq!(start.resume_position(7), None);
    }

    #[test]
    fn ai_subtitles_are_recognized() {
        let track = SubtitleTrack {
            lan: "ai-zh".into(),
            lan_doc: "中文（自动生成）".into(),
            subtitle_url: String::new(),
        };
        assert!(track.is_ai());
    }

    /// Read-only: relation, player info and a subtitle track if any.
    #[tokio::test]
    #[ignore = "contacts the live Bilibili API with saved credentials"]
    async fn video_relation_and_player_info_contract() {
        let credentials = crate::storage::load_credentials().expect("saved credentials");
        let client = ApiClient::with_cookies(&credentials);
        let video = client.get_video_info("BV1GJ411x7h7").await.unwrap();
        client.get_archive_relation(video.aid).await.unwrap();
        let info = client.get_player_info(video.aid, video.cid).await.unwrap();
        eprintln!("subtitle tracks: {}", info.subtitle.subtitles.len());
        if let Some(track) = info.subtitle.subtitles.first() {
            let srt = client.get_subtitle_srt(track).await.unwrap();
            assert!(srt.contains(" --> "));
        }
    }
}
