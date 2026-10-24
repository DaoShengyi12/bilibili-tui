//! Bangumi (番剧/影视) API types

use super::serde_util::null_as_default;
use serde::Deserialize;

// ─── Timeline ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TimelineDay {
    #[serde(default, deserialize_with = "null_as_default")]
    pub date: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub date_ts: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub day_of_week: i32,
    #[serde(default, deserialize_with = "null_as_default")]
    pub episodes: Vec<TimelineEpisode>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub is_today: i32,
}

/// Only the identifiers and title are required; display fields drift often.
#[derive(Debug, Clone, Deserialize)]
pub struct TimelineEpisode {
    #[serde(default, deserialize_with = "null_as_default")]
    pub cover: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub delay: i32,
    #[serde(default, deserialize_with = "null_as_default")]
    pub ep_cover: String,
    pub episode_id: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub follows: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub plays: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub pub_index: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub pub_time: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub pub_ts: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub published: i32,
    pub season_id: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub square_cover: String,
    pub title: String,
}

impl TimelineEpisode {
    pub fn cover_url(&self) -> String {
        let url = if self.ep_cover.is_empty() {
            &self.cover
        } else {
            &self.ep_cover
        };
        if url.starts_with("//") {
            format!("https:{}", url)
        } else {
            url.clone()
        }
    }
}

// ─── Season Rank (replaces index) ───────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SeasonRankItem {
    #[serde(default, deserialize_with = "null_as_default")]
    pub badge: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub cover: String,
    #[serde(default)]
    pub new_ep: Option<NewEp>,
    pub season_id: i64,
    pub title: String,
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub stat: Option<RankStat>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewEp {
    #[serde(default, deserialize_with = "null_as_default")]
    pub index_show: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RankStat {
    pub view: Option<i64>,
}

impl SeasonRankItem {
    pub fn cover_url(&self) -> String {
        if self.cover.starts_with("//") {
            format!("https:{}", self.cover)
        } else {
            self.cover.clone()
        }
    }

    pub fn display_title(&self) -> String {
        self.title.clone()
    }

    pub fn display_subtitle(&self) -> String {
        let mut parts = Vec::new();
        if let Some(ref ep) = self.new_ep
            && !ep.index_show.is_empty()
        {
            parts.push(ep.index_show.clone());
        }
        if let Some(ref stat) = self.stat
            && let Some(view) = stat.view
        {
            parts.push(format_views(view));
        }
        if parts.is_empty() {
            "-".to_string()
        } else {
            parts.join(" · ")
        }
    }

    pub fn badge_text(&self) -> Option<&str> {
        if self.badge.is_empty() {
            None
        } else {
            Some(&self.badge)
        }
    }

    pub fn score_text(&self) -> String {
        self.rating.clone().unwrap_or_default()
    }
}

fn format_views(view: i64) -> String {
    if view >= 100000000 {
        format!("{:.1}亿", view as f64 / 100000000.0)
    } else if view >= 10000 {
        format!("{:.1}万", view as f64 / 10000.0)
    } else {
        view.to_string()
    }
}

// ─── Season Detail ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SeasonResult {
    pub title: String,
    #[serde(rename = "season_id")]
    pub season_id: i64,
    pub cover: String,
    #[serde(rename = "square_cover")]
    pub square_cover: String,
    pub evaluate: Option<String>,
    pub link: Option<String>,
    pub episodes: Option<Vec<BangumiEpisode>>,
    pub section: Option<Vec<EpisodeSection>>,
    pub rating: Option<SeasonRating>,
    pub stat: Option<SeasonStat>,
    pub badge: Option<String>,
    #[serde(rename = "is_finish")]
    pub is_finish: Option<i32>,
    #[serde(rename = "index_show")]
    pub index_show: Option<String>,
    /// 1 anime, 2 movie, 3 documentary, 4 guochuang, 5 TV series, 7 variety.
    #[serde(
        rename = "type",
        default,
        deserialize_with = "super::serde_util::null_as_default"
    )]
    pub season_type: i32,
}

#[derive(Debug, Deserialize)]
pub struct SeasonRating {
    pub score: f32,
    pub count: i64,
}

#[derive(Debug, Deserialize)]
pub struct SeasonStat {
    pub views: Option<i64>,
    pub danmakus: Option<i64>,
    #[serde(rename = "favorite")]
    pub favorites: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EpisodeSection {
    pub episodes: Vec<BangumiEpisode>,
    pub id: i64,
    pub title: String,
    #[serde(rename = "type")]
    pub section_type: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BangumiEpisode {
    pub aid: i64,
    #[serde(default)]
    pub badge: String,
    #[serde(default, deserialize_with = "super::serde_util::null_as_default")]
    pub bvid: String,
    pub cid: i64,
    #[serde(default)]
    pub cover: String,
    #[serde(default)]
    pub from: String,
    pub id: i64,
    #[serde(rename = "is_premiere", default)]
    pub is_premiere: i32,
    #[serde(rename = "long_title", default)]
    pub long_title: String,
    #[serde(rename = "share_url", default)]
    pub share_url: String,
    #[serde(default)]
    pub status: i32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub vid: String,
}

impl BangumiEpisode {
    pub fn cover_url(&self) -> String {
        if self.cover.starts_with("//") {
            format!("https:{}", self.cover)
        } else {
            self.cover.clone()
        }
    }

    pub fn display_title(&self) -> String {
        if self.long_title.is_empty() {
            format!("第{}集", self.title)
        } else {
            format!("{} {}", self.title, self.long_title)
        }
    }

    pub fn badge_text(&self) -> Option<&str> {
        if self.badge.is_empty() {
            None
        } else {
            Some(&self.badge)
        }
    }
}

impl SeasonResult {
    pub fn cover_url(&self) -> String {
        let url = if self.cover.is_empty() {
            self.square_cover.as_str()
        } else {
            &self.cover
        };
        if url.starts_with("//") {
            format!("https:{}", url)
        } else {
            url.to_string()
        }
    }

    pub fn all_sections(&self) -> Vec<EpisodeSection> {
        let mut sections = Vec::new();
        // Modern API puts main episodes directly in `result.episodes`
        if let Some(ref eps) = self.episodes
            && !eps.is_empty()
        {
            sections.push(EpisodeSection {
                episodes: eps.clone(),
                id: 0,
                title: "正片".to_string(),
                section_type: 0,
            });
        }
        // Legacy `main_section` is no longer present in new API
        if let Some(ref secs) = self.section {
            for s in secs {
                if !s.episodes.is_empty() {
                    sections.push(s.clone());
                }
            }
        }
        sections
    }

    pub fn total_episodes(&self) -> usize {
        self.all_sections().iter().map(|s| s.episodes.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::{SeasonRankItem, TimelineDay};
    use crate::api::serde_util::parse_lenient_list;
    use serde_json::json;

    #[test]
    fn rank_keeps_items_with_drifted_display_fields() {
        let list = json!([
            {"season_id": 1, "title": "A", "badge": null, "new_ep": {"index_show": null}},
            {"season_id": "oops", "title": "B"},
            {"season_id": 3, "title": "C", "cover": "//i0.hdslb.com/c.jpg", "stat": {"view": 12}}
        ]);
        let items: Vec<SeasonRankItem> = parse_lenient_list(Some(&list), "rank").unwrap();
        let ids: Vec<i64> = items.iter().map(|item| item.season_id).collect();
        assert_eq!(ids, vec![1, 3]);
        assert_eq!(items[0].badge_text(), None);
        assert_eq!(items[1].cover_url(), "https://i0.hdslb.com/c.jpg");
    }

    #[test]
    fn timeline_day_with_null_episodes_is_empty() {
        let days = json!([
            {"date": "10-24", "episodes": null, "is_today": 1},
            {"date": "10-25", "episodes": [{"episode_id": 9, "season_id": 8, "title": "T"}]}
        ]);
        let days: Vec<TimelineDay> = parse_lenient_list(Some(&days), "timeline").unwrap();
        assert!(days[0].episodes.is_empty());
        assert_eq!(days[1].episodes[0].episode_id, 9);
    }
}
