//! Icons drawn in the UI. Nerd Font glyphs by default; plain-text labels
//! when the terminal font has no Nerd Font (`"icons": "text"` in the config).
//!
//! The codepoints are Material Design Icons (`nf-md-*`), checked against
//! the glyph names of a Nerd Font build.

use std::sync::atomic::{AtomicBool, Ordering};

static NERD_FONT: AtomicBool = AtomicBool::new(true);

/// Choose Nerd Font glyphs (`true`) or text labels for every icon.
pub fn set_nerd_font(enabled: bool) {
    NERD_FONT.store(enabled, Ordering::Relaxed);
}

pub fn nerd_font() -> bool {
    NERD_FONT.load(Ordering::Relaxed)
}

/// A glyph with its text fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon {
    glyph: char,
    text: &'static str,
}

impl Icon {
    fn render(self, nerd_font: bool) -> String {
        if nerd_font {
            self.glyph.to_string()
        } else {
            self.text.to_string()
        }
    }
}

impl std::fmt::Display for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render(nerd_font()))
    }
}

const fn icon(glyph: char, text: &'static str) -> Icon {
    Icon { glyph, text }
}

pub const HOME: Icon = icon('\u{f02dc}', "首"); // md-home
pub const SEARCH: Icon = icon('\u{f0349}', "搜"); // md-magnify
pub const DYNAMIC: Icon = icon('\u{f046b}', "动"); // md-rss
pub const HISTORY: Icon = icon('\u{f02da}', "史"); // md-history
pub const FAVORITE: Icon = icon('\u{f04ce}', "藏"); // md-star
pub const LIVE: Icon = icon('\u{f1720}', "播"); // md-broadcast
pub const BANGUMI: Icon = icon('\u{f0fce}', "番"); // md-movie_open
pub const SETTINGS: Icon = icon('\u{f0493}', "设"); // md-cog
pub const ACCOUNT: Icon = icon('\u{f0004}', "我"); // md-account
pub const PLAY: Icon = icon('\u{f040a}', "播放"); // md-play
pub const DANMAKU: Icon = icon('\u{f0369}', "弹幕"); // md-message_text
pub const COMMENT: Icon = icon('\u{f017a}', "评论"); // md-comment
pub const LIKE: Icon = icon('\u{f0513}', "赞"); // md-thumb_up
pub const COIN: Icon = icon('\u{f0b38}', "币"); // md-circle_multiple
pub const STAR: Icon = icon('\u{f04ce}', "藏"); // md-star
pub const VIEWERS: Icon = icon('\u{f0208}', "人气"); // md-eye
pub const HEART: Icon = icon('\u{f02d1}', "关注"); // md-heart
pub const GIFT: Icon = icon('\u{f0e44}', "礼物"); // md-gift
pub const GUARD: Icon = icon('\u{f0031}', "舰"); // md-anchor
pub const SHARE: Icon = icon('\u{f0497}', "分享"); // md-share_variant
pub const PICTURE: Icon = icon('\u{f02e9}', "图"); // md-image
pub const ARTICLE: Icon = icon('\u{f0395}', "专栏"); // md-newspaper
pub const FORWARD: Icon = icon('\u{f0496}', "转发"); // md-share
pub const TEXT: Icon = icon('\u{f09a8}', "文"); // md-text
pub const CLOCK: Icon = icon('\u{f0150}', "时间"); // md-clock_outline
pub const VIDEO: Icon = icon('\u{f0567}', "视频"); // md-video
pub const REPLY: Icon = icon('\u{f045a}', "回复"); // md-reply

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_fall_back_to_text() {
        assert_eq!(LIKE.render(false), "赞");
        assert_eq!(LIKE.render(true), "\u{f0513}");
    }
}
