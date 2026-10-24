use anyhow::Result;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct HeartbeatResponse {
    pub code: i32,
    pub message: String,
}

/// The video a watch report is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchTarget {
    pub aid: i64,
    pub cid: i64,
    /// Empty when unknown; PGC episodes are identified by `pgc`.
    pub bvid: String,
    pub pgc: Option<PgcWatch>,
}

/// Episode identifiers that make a report count as PGC (bangumi) history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PgcWatch {
    pub ep_id: i64,
    pub season_id: i64,
    /// `type` of the season: 1 anime, 2 movie, 3 documentary, ...
    pub season_type: i32,
}

impl WatchTarget {
    fn refer_url(&self) -> String {
        match self.pgc {
            Some(pgc) => format!("https://www.bilibili.com/bangumi/play/ep{}", pgc.ep_id),
            None => format!("https://www.bilibili.com/video/{}", self.bvid),
        }
    }

    /// Fields that identify the video in both report endpoints.
    fn identity_fields(&self) -> Vec<(&'static str, String)> {
        let mut fields = vec![
            ("aid", self.aid.to_string()),
            ("cid", self.cid.to_string()),
            ("bvid", self.bvid.clone()),
            ("mid", "0".to_string()),
        ];
        match self.pgc {
            Some(pgc) => fields.extend([
                ("type", "4".to_string()),
                ("sub_type", pgc.season_type.to_string()),
                ("epid", pgc.ep_id.to_string()),
                ("sid", pgc.season_id.to_string()),
            ]),
            None => fields.push(("type", "3".to_string())),
        }
        fields
    }
}

/// Report video watch start to Bilibili
pub async fn report_watch_start(
    client: &super::client::ApiClient,
    target: &WatchTarget,
) -> Result<HeartbeatResponse> {
    let url = format!(
        "{}/x/click-interface/click/web/h5",
        super::client::BilibiliApiDomain::Main.as_str()
    );

    let mut form_data = target.identity_fields();
    form_data.extend([
        ("dt", "2".to_string()),
        ("auto_continued_play", "0".to_string()),
        ("refer_url", target.refer_url()),
        ("bsource", "".to_string()),
        ("stime", chrono::Utc::now().timestamp().to_string()),
    ]);

    let resp: super::client::ApiResponse<HeartbeatResponse> = client.post(&url, form_data).await?;
    resp.data
        .ok_or_else(|| anyhow::anyhow!("No data in watch start response"))
}

/// Report video playback heartbeat
pub async fn report_heartbeat(
    client: &super::client::ApiClient,
    target: &WatchTarget,
    played_time: i64,
    real_played_time: i64,
    realtime: i64,
    start_ts: i64,
    play_type: i32,
) -> Result<HeartbeatResponse> {
    let url = format!(
        "{}/x/click-interface/web/heartbeat",
        super::client::BilibiliApiDomain::Main.as_str()
    );

    let mut form_data = target.identity_fields();
    form_data.extend([
        ("played_time", played_time.to_string()),
        ("realtime", realtime.to_string()),
        ("real_played_time", real_played_time.to_string()),
        ("start_ts", start_ts.to_string()),
        ("dt", "2".to_string()),
        ("play_type", play_type.to_string()),
        ("auto_continued_play", "0".to_string()),
        ("refer_url", target.refer_url()),
        ("bsource", "".to_string()),
    ]);

    let resp: super::client::ApiResponse<HeartbeatResponse> = client.post(&url, form_data).await?;
    resp.data
        .ok_or_else(|| anyhow::anyhow!("No data in heartbeat response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(fields: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
        fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn pgc_reports_carry_episode_and_season_ids() {
        let target = WatchTarget {
            aid: 1,
            cid: 2,
            bvid: String::new(),
            pgc: Some(PgcWatch {
                ep_id: 3,
                season_id: 4,
                season_type: 1,
            }),
        };
        let fields = target.identity_fields();
        assert_eq!(field(&fields, "type"), Some("4"));
        assert_eq!(field(&fields, "sub_type"), Some("1"));
        assert_eq!(field(&fields, "epid"), Some("3"));
        assert_eq!(field(&fields, "sid"), Some("4"));
        assert_eq!(
            target.refer_url(),
            "https://www.bilibili.com/bangumi/play/ep3"
        );
    }

    #[test]
    fn archive_reports_use_the_archive_type() {
        let target = WatchTarget {
            aid: 1,
            cid: 2,
            bvid: "BV1xx".to_string(),
            pgc: None,
        };
        let fields = target.identity_fields();
        assert_eq!(field(&fields, "type"), Some("3"));
        assert_eq!(field(&fields, "epid"), None);
        assert_eq!(target.refer_url(), "https://www.bilibili.com/video/BV1xx");
    }
}
