//! Comment API types and functions

use super::serde_util::{lenient_flag, null_as_default};
use serde::Deserialize;

/// Comment order offered by `/x/v2/reply/wbi/main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommentSort {
    #[default]
    Hot,
    Time,
}

impl CommentSort {
    /// The endpoint's `mode`: 3 sorts by popularity, 2 by time.
    pub const fn mode(self) -> i32 {
        match self {
            Self::Hot => 3,
            Self::Time => 2,
        }
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::Hot => Self::Time,
            Self::Time => Self::Hot,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hot => "按热度",
            Self::Time => "按时间",
        }
    }
}

/// `data` of `/x/v2/reply/wbi/main`, which pages by cursor instead of number.
#[derive(Debug, Default, Deserialize)]
pub struct MainCommentData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub cursor: MainCommentCursor,
    #[serde(default, deserialize_with = "null_as_default")]
    pub replies: Vec<CommentItem>,
    /// Pinned comments, only sent with the first page.
    #[serde(default, deserialize_with = "null_as_default")]
    pub top_replies: Vec<CommentItem>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MainCommentCursor {
    #[serde(default, deserialize_with = "lenient_flag")]
    pub is_end: bool,
    #[serde(default, deserialize_with = "null_as_default")]
    pub pagination_reply: PaginationReply,
}

#[derive(Debug, Default, Deserialize)]
pub struct PaginationReply {
    #[serde(default, deserialize_with = "null_as_default")]
    pub next_offset: String,
}

/// One page of comments and the cursor for the next page, if any.
#[derive(Debug, Clone, Default)]
pub struct CommentBatch {
    pub comments: Vec<CommentItem>,
    pub next_offset: Option<String>,
}

impl MainCommentData {
    /// Pinned comments come first; a pinned comment repeated in `replies`
    /// is shown once.
    pub fn into_batch(self) -> CommentBatch {
        let mut comments = self.top_replies;
        for reply in self.replies {
            if !comments.iter().any(|existing| existing.rpid == reply.rpid) {
                comments.push(reply);
            }
        }
        let next_offset = (!self.cursor.is_end)
            .then_some(self.cursor.pagination_reply.next_offset)
            .filter(|offset| !offset.is_empty());
        CommentBatch {
            comments,
            next_offset,
        }
    }
}

/// The `pagination_str` parameter for a page after `offset`, or the first page.
pub fn pagination_str(offset: Option<&str>) -> String {
    serde_json::json!({ "offset": offset.unwrap_or_default() }).to_string()
}

/// Comment list response
#[derive(Debug, Deserialize)]
pub struct CommentData {
    pub page: Option<CommentPage>,
    pub replies: Option<Vec<CommentItem>>,
    pub hots: Option<Vec<CommentItem>>,
}

#[derive(Debug, Deserialize)]
pub struct CommentPage {
    pub num: Option<i32>,
    pub size: Option<i32>,
    pub count: Option<i32>,
    pub acount: Option<i32>,
}

/// Individual comment item
#[derive(Debug, Clone, Deserialize)]
pub struct CommentItem {
    pub rpid: i64,
    pub oid: i64,
    pub mid: i64,
    pub parent: i64,
    pub count: Option<i32>,
    pub rcount: Option<i32>,
    pub floor: Option<i32>,
    pub ctime: Option<i64>,
    pub like: Option<i32>,
    pub member: Option<CommentMember>,
    pub content: Option<CommentContent>,
    pub replies: Option<Vec<CommentItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentMember {
    pub mid: Option<String>,
    pub uname: Option<String>,
    pub avatar: Option<String>,
    pub level_info: Option<LevelInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LevelInfo {
    pub current_level: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentContent {
    pub message: Option<String>,
}

impl CommentItem {
    pub fn author_name(&self) -> &str {
        self.member
            .as_ref()
            .and_then(|m| m.uname.as_deref())
            .unwrap_or("匿名")
    }

    pub fn message(&self) -> &str {
        self.content
            .as_ref()
            .and_then(|c| c.message.as_deref())
            .unwrap_or("")
    }

    pub fn format_like(&self) -> String {
        match self.like {
            Some(n) if n >= 10000 => format!("{:.1}万", n as f64 / 10000.0),
            Some(n) => format!("{}", n),
            None => "-".to_string(),
        }
    }

    pub fn format_time(&self) -> String {
        if let Some(ctime) = self.ctime {
            // Convert timestamp to relative time
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let diff = now - ctime;

            if diff < 60 {
                "刚刚".to_string()
            } else if diff < 3600 {
                format!("{}分钟前", diff / 60)
            } else if diff < 86400 {
                format!("{}小时前", diff / 3600)
            } else if diff < 2592000 {
                format!("{}天前", diff / 86400)
            } else {
                format!("{}月前", diff / 2592000)
            }
        } else {
            "".to_string()
        }
    }

    pub fn reply_count(&self) -> i32 {
        self.rcount.unwrap_or(0)
    }
}

/// Comment type enum for different content types
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum CommentType {
    /// 视频 (Video)
    Video = 1,
    /// 话题 (Topic)
    Topic = 6,
    /// 活动 (Activity)
    Activity = 10,
    /// 相簿/图片动态 (Photo Album)
    Album = 11,
    /// 专栏 (Article)
    Article = 12,
    /// 音频 (Audio)
    Audio = 14,
    /// 动态（纯文字 & 分享）
    Dynamic = 17,
    /// 合辑 (Playlist)
    Playlist = 19,
    /// 课程 (Course)
    Course = 33,
}

#[allow(dead_code)]
impl CommentType {
    pub fn as_i32(&self) -> i32 {
        *self as i32
    }
}

/// Response for adding a comment
#[derive(Debug, Deserialize)]
pub struct AddCommentResponse {
    pub success_action: Option<i32>,
    pub success_toast: Option<String>,
    pub need_captcha: Option<bool>,
    pub rpid: Option<i64>,
    pub rpid_str: Option<String>,
    pub root: Option<i64>,
    pub parent: Option<i64>,
    pub reply: Option<CommentItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(rpid: i64) -> serde_json::Value {
        serde_json::json!({"rpid": rpid, "oid": 1, "mid": 2, "parent": 0})
    }

    #[test]
    fn main_comments_put_pinned_first_without_duplicates() {
        let data: MainCommentData = serde_json::from_value(serde_json::json!({
            "cursor": {"is_end": false, "pagination_reply": {"next_offset": "abc"}, "all_count": "12"},
            "replies": [comment(2), comment(3)],
            "top_replies": [comment(3)],
        }))
        .unwrap();
        let batch = data.into_batch();
        let ids: Vec<i64> = batch.comments.iter().map(|c| c.rpid).collect();
        assert_eq!(ids, [3, 2]);
        assert_eq!(batch.next_offset.as_deref(), Some("abc"));
    }

    #[test]
    fn last_or_empty_main_comment_page_has_no_next_offset() {
        let end: MainCommentData = serde_json::from_value(serde_json::json!({
            "cursor": {"is_end": true, "pagination_reply": {"next_offset": "abc"}},
            "replies": null,
            "top_replies": null,
        }))
        .unwrap();
        let batch = end.into_batch();
        assert!(batch.comments.is_empty() && batch.next_offset.is_none());

        let no_cursor: MainCommentData =
            serde_json::from_value(serde_json::json!({"cursor": null})).unwrap();
        assert!(no_cursor.into_batch().next_offset.is_none());
    }

    #[test]
    fn pagination_str_escapes_the_offset_as_json() {
        assert_eq!(pagination_str(None), r#"{"offset":""}"#);
        assert_eq!(
            pagination_str(Some(r#"{"type":1}"#)),
            r#"{"offset":"{\"type\":1}"}"#
        );
    }

    #[test]
    fn sort_modes_match_the_endpoint() {
        assert_eq!(CommentSort::Hot.mode(), 3);
        assert_eq!(CommentSort::Time.mode(), 2);
        assert_eq!(CommentSort::Hot.toggled(), CommentSort::Time);
    }
}
