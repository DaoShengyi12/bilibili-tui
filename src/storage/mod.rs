//! Credential storage and persistence

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COOKIE_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn write_private_file(path: &std::path::Path, content: &[u8]) -> Result<()> {
    let sequence = COOKIE_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write_result = (|| -> std::io::Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// User credentials from Bilibili login
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub sessdata: String,
    pub bili_jct: String,
    pub dede_user_id: String,
    pub dede_user_id_ckmd5: Option<String>,
    pub refresh_token: Option<String>,
}

impl Credentials {
    pub fn from_cookies(
        cookies: &[(String, String)],
        refresh_token: Option<String>,
    ) -> Option<Self> {
        // ✅ 使用闭包和迭代器查找 cookie
        let get_cookie = |name: &str| -> Option<String> {
            cookies
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };

        Some(Credentials {
            sessdata: get_cookie("SESSDATA")?,
            bili_jct: get_cookie("bili_jct")?,
            dede_user_id: get_cookie("DedeUserID")?,
            dede_user_id_ckmd5: get_cookie("DedeUserID__ckMd5"),
            refresh_token,
        })
    }
}

/// Keybindings configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keybindings {
    // Global actions
    pub quit: String,
    pub confirm: String,
    pub back: String,
    pub refresh: String,

    // Navigation
    pub nav_up: String,
    pub nav_down: String,
    pub nav_left: String,
    pub nav_right: String,
    pub nav_next_page: String,
    pub nav_prev_page: String,
    /// Move the focused content list by one viewport.
    pub page_down: String,
    pub page_up: String,

    // Section/Tab navigation
    pub section_prev: String,
    pub section_next: String,
    pub tab_1: String,
    pub tab_2: String,
    pub tab_3: String,

    // Actions
    pub next_theme: String,
    pub play: String,
    pub open_settings: String,
    pub search_focus: String,
    /// Stop the running player from any page.
    pub stop_playback: String,

    // Comments
    pub comment: String,
    pub toggle_replies: String,
    /// Switch comments between hot and newest first.
    pub comment_sort: String,
    /// Load the next page where a list does not load it on scroll.
    pub load_more: String,

    // Dynamic page specific
    pub up_prev: String,
    pub up_next: String,

    // Video actions
    pub open_up: String,
    pub video_like: String,
    pub video_coin: String,
    pub video_favorite: String,
    pub video_triple: String,
    pub watch_later: String,
    pub follow: String,
    /// Send a danmaku to the playing video or the live room.
    pub send_danmaku: String,
    /// Pick the live stream quality.
    pub live_quality: String,
    /// Cycle forward/reverse/shuffle for "play all".
    pub play_order: String,
    /// Jump to the first item of the focused list or the top of a text.
    /// Home always works as well.
    pub jump_top: String,
    /// Jump to the last loaded item or the bottom of a text. End always
    /// works as well.
    pub jump_bottom: String,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            // Global
            quit: "q".to_string(),
            confirm: "Enter".to_string(),
            back: "Esc".to_string(),
            refresh: "r".to_string(),

            // Navigation
            nav_up: "k".to_string(),
            nav_down: "j".to_string(),
            nav_left: "h".to_string(),
            nav_right: "l".to_string(),
            nav_next_page: "Tab".to_string(),
            nav_prev_page: "BackTab".to_string(),
            page_down: "PageDown".to_string(),
            page_up: "PageUp".to_string(),

            // Section/Tab
            section_prev: "[".to_string(),
            section_next: "]".to_string(),
            tab_1: "1".to_string(),
            tab_2: "2".to_string(),
            tab_3: "3".to_string(),

            // Actions
            next_theme: "t".to_string(),
            play: "p".to_string(),
            open_settings: "s".to_string(),
            search_focus: "/".to_string(),
            stop_playback: "x".to_string(),

            // Comments
            comment: "c".to_string(),
            toggle_replies: "r".to_string(),
            comment_sort: "o".to_string(),
            load_more: "n".to_string(),

            // Dynamic page
            up_prev: "[".to_string(),
            up_next: "]".to_string(),

            // Video actions
            open_up: "u".to_string(),
            video_like: "a".to_string(),
            video_coin: "b".to_string(),
            video_favorite: "f".to_string(),
            video_triple: "T".to_string(),
            watch_later: "w".to_string(),
            follow: "F".to_string(),
            send_danmaku: "d".to_string(),
            live_quality: "v".to_string(),
            play_order: "s".to_string(),
            jump_top: "g".to_string(),
            jump_bottom: "G".to_string(),
        }
    }
}

use ratatui::crossterm::event::KeyCode;

impl Keybindings {
    /// Parse a string representation into a KeyCode
    pub fn parse_keycode(s: &str) -> Option<KeyCode> {
        let s = s.trim();
        match s.to_lowercase().as_str() {
            "enter" | "return" => Some(KeyCode::Enter),
            "esc" | "escape" => Some(KeyCode::Esc),
            "tab" => Some(KeyCode::Tab),
            "backtab" | "shift+tab" => Some(KeyCode::BackTab),
            "backspace" => Some(KeyCode::Backspace),
            "delete" | "del" => Some(KeyCode::Delete),
            "insert" | "ins" => Some(KeyCode::Insert),
            "home" => Some(KeyCode::Home),
            "end" => Some(KeyCode::End),
            "pageup" | "pgup" => Some(KeyCode::PageUp),
            "pagedown" | "pgdn" => Some(KeyCode::PageDown),
            "up" | "↑" => Some(KeyCode::Up),
            "down" | "↓" => Some(KeyCode::Down),
            "left" | "←" => Some(KeyCode::Left),
            "right" | "→" => Some(KeyCode::Right),
            "space" | " " => Some(KeyCode::Char(' ')),
            "f1" => Some(KeyCode::F(1)),
            "f2" => Some(KeyCode::F(2)),
            "f3" => Some(KeyCode::F(3)),
            "f4" => Some(KeyCode::F(4)),
            "f5" => Some(KeyCode::F(5)),
            "f6" => Some(KeyCode::F(6)),
            "f7" => Some(KeyCode::F(7)),
            "f8" => Some(KeyCode::F(8)),
            "f9" => Some(KeyCode::F(9)),
            "f10" => Some(KeyCode::F(10)),
            "f11" => Some(KeyCode::F(11)),
            "f12" => Some(KeyCode::F(12)),
            _ => {
                // Single character
                let chars: Vec<char> = s.chars().collect();
                if chars.len() == 1 {
                    Some(KeyCode::Char(chars[0]))
                } else {
                    None
                }
            }
        }
    }

    /// Convert a KeyCode to its string representation, or `None` for keys
    /// that `parse_keycode` cannot read back (media keys, modifiers, ...).
    pub fn keycode_to_string(key: KeyCode) -> Option<String> {
        Some(match key {
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => "BackTab".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Delete".to_string(),
            KeyCode::Insert => "Insert".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PageUp".to_string(),
            KeyCode::PageDown => "PageDown".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::F(n) => format!("F{}", n),
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            _ => return None,
        })
    }

    /// Check if a key matches the configured keybinding (including arrow key alternatives)
    pub fn matches(&self, binding: &str, key: KeyCode) -> bool {
        if let Some(configured_key) = Self::parse_keycode(binding)
            && key == configured_key
        {
            return true;
        }
        false
    }

    // Convenience methods for common keybindings
    pub fn matches_quit(&self, key: KeyCode) -> bool {
        self.matches(&self.quit, key)
    }

    pub fn matches_confirm(&self, key: KeyCode) -> bool {
        self.matches(&self.confirm, key)
    }

    pub fn matches_back(&self, key: KeyCode) -> bool {
        self.matches(&self.back, key)
    }

    pub fn matches_refresh(&self, key: KeyCode) -> bool {
        self.matches(&self.refresh, key)
    }

    pub fn matches_up(&self, key: KeyCode) -> bool {
        self.matches(&self.nav_up, key) || key == KeyCode::Up
    }

    pub fn matches_down(&self, key: KeyCode) -> bool {
        self.matches(&self.nav_down, key) || key == KeyCode::Down
    }

    pub fn matches_left(&self, key: KeyCode) -> bool {
        self.matches(&self.nav_left, key) || key == KeyCode::Left
    }

    pub fn matches_right(&self, key: KeyCode) -> bool {
        self.matches(&self.nav_right, key) || key == KeyCode::Right
    }

    pub fn matches_nav_next(&self, key: KeyCode) -> bool {
        self.matches(&self.nav_next_page, key)
    }

    pub fn matches_nav_prev(&self, key: KeyCode) -> bool {
        self.matches(&self.nav_prev_page, key)
    }

    pub fn matches_page_down(&self, key: KeyCode) -> bool {
        self.matches(&self.page_down, key)
    }

    pub fn matches_page_up(&self, key: KeyCode) -> bool {
        self.matches(&self.page_up, key)
    }

    pub fn matches_next_theme(&self, key: KeyCode) -> bool {
        self.matches(&self.next_theme, key)
    }

    pub fn matches_play(&self, key: KeyCode) -> bool {
        self.matches(&self.play, key)
    }

    pub fn matches_open_settings(&self, key: KeyCode) -> bool {
        self.matches(&self.open_settings, key)
    }

    pub fn matches_search_focus(&self, key: KeyCode) -> bool {
        self.matches(&self.search_focus, key) || key == KeyCode::Char('i')
    }

    pub fn matches_stop_playback(&self, key: KeyCode) -> bool {
        self.matches(&self.stop_playback, key)
    }

    pub fn matches_section_prev(&self, key: KeyCode) -> bool {
        self.matches(&self.section_prev, key)
    }

    pub fn matches_section_next(&self, key: KeyCode) -> bool {
        self.matches(&self.section_next, key)
    }

    pub fn matches_comment(&self, key: KeyCode) -> bool {
        self.matches(&self.comment, key)
    }

    pub fn matches_toggle_replies(&self, key: KeyCode) -> bool {
        self.matches(&self.toggle_replies, key)
    }

    pub fn matches_comment_sort(&self, key: KeyCode) -> bool {
        self.matches(&self.comment_sort, key)
    }

    pub fn matches_load_more(&self, key: KeyCode) -> bool {
        self.matches(&self.load_more, key)
    }

    pub fn matches_open_up(&self, key: KeyCode) -> bool {
        self.matches(&self.open_up, key)
    }

    pub fn matches_video_like(&self, key: KeyCode) -> bool {
        self.matches(&self.video_like, key)
    }

    pub fn matches_video_coin(&self, key: KeyCode) -> bool {
        self.matches(&self.video_coin, key)
    }

    pub fn matches_video_favorite(&self, key: KeyCode) -> bool {
        self.matches(&self.video_favorite, key)
    }

    pub fn matches_video_triple(&self, key: KeyCode) -> bool {
        self.matches(&self.video_triple, key)
    }

    pub fn matches_watch_later(&self, key: KeyCode) -> bool {
        self.matches(&self.watch_later, key)
    }

    pub fn matches_follow(&self, key: KeyCode) -> bool {
        self.matches(&self.follow, key)
    }

    pub fn matches_send_danmaku(&self, key: KeyCode) -> bool {
        self.matches(&self.send_danmaku, key)
    }

    pub fn matches_live_quality(&self, key: KeyCode) -> bool {
        self.matches(&self.live_quality, key)
    }

    pub fn matches_play_order(&self, key: KeyCode) -> bool {
        self.matches(&self.play_order, key)
    }

    pub fn matches_jump_top(&self, key: KeyCode) -> bool {
        self.matches(&self.jump_top, key) || key == KeyCode::Home
    }

    pub fn matches_jump_bottom(&self, key: KeyCode) -> bool {
        self.matches(&self.jump_bottom, key) || key == KeyCode::End
    }

    /// Whether `key` is bound to moving through or acting on a list. Page
    /// shortcuts that are not configurable (History's Space/`d`/`u`, UP's
    /// `o`/`s`) yield to these so a user's custom binding always works.
    pub fn matches_list_navigation(&self, key: KeyCode) -> bool {
        self.matches_up(key)
            || self.matches_down(key)
            || self.matches_left(key)
            || self.matches_right(key)
            || self.matches_page_up(key)
            || self.matches_page_down(key)
            || self.matches_confirm(key)
            || self.matches_back(key)
            || self.matches_play(key)
            || self.matches_jump_top(key)
            || self.matches_jump_bottom(key)
    }

    /// Whether `key` switches the theme and nothing else, so the app can
    /// handle it on every page without stealing a page's own shortcut.
    pub fn is_dedicated_theme_key(&self, key: KeyCode) -> bool {
        // Keys some matchers accept besides the configured binding.
        let implicit = matches!(
            key,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Char('i' | '[' | ']')
        );
        self.matches_next_theme(key)
            && !implicit
            && self
                .bindings()
                .iter()
                .filter(|binding| Self::parse_keycode(binding) == Some(key))
                .count()
                == 1
    }

    pub fn matches_tab_1(&self, key: KeyCode) -> bool {
        self.matches(&self.tab_1, key)
    }

    pub fn matches_tab_2(&self, key: KeyCode) -> bool {
        self.matches(&self.tab_2, key)
    }

    pub fn matches_tab_3(&self, key: KeyCode) -> bool {
        self.matches(&self.tab_3, key)
    }

    pub fn matches_up_prev(&self, key: KeyCode) -> bool {
        self.matches(&self.up_prev, key) || key == KeyCode::Char('[')
    }

    pub fn matches_up_next(&self, key: KeyCode) -> bool {
        self.matches(&self.up_next, key) || key == KeyCode::Char(']')
    }

    pub fn get_nav_keys_display(&self) -> String {
        format!(
            "{}{}{}{}",
            self.nav_left, self.nav_up, self.nav_down, self.nav_right
        )
    }

    pub fn get_arrow_keys_display(&self) -> String {
        "←↑↓→".to_string()
    }

    /// Every configurable binding with its settings label, in settings
    /// order. `update_by_index` uses the same order.
    pub fn get_all_labels(&self) -> Vec<(&'static str, &str)> {
        Self::LABELS.into_iter().zip(self.bindings()).collect()
    }

    const LABELS: [&'static str; 40] = [
        "退出",
        "确认",
        "返回",
        "刷新",
        "向上",
        "向下",
        "向左",
        "向右",
        "下一页面",
        "上一页面",
        "内容下翻页",
        "内容上翻页",
        "上一分区",
        "下一分区",
        "标签1",
        "标签2",
        "标签3",
        "切换主题",
        "播放",
        "设置",
        "搜索",
        "评论",
        "展开回复",
        "上一UP",
        "下一UP",
        "停止播放",
        "切换排序",
        "加载更多",
        "打开UP主页",
        "点赞",
        "投币",
        "收藏",
        "一键三连",
        "稍后再看",
        "关注UP",
        "发送弹幕",
        "直播画质",
        "播放顺序",
        "跳到开头",
        "跳到末尾",
    ];

    fn bindings(&self) -> [&str; 40] {
        [
            &self.quit,
            &self.confirm,
            &self.back,
            &self.refresh,
            &self.nav_up,
            &self.nav_down,
            &self.nav_left,
            &self.nav_right,
            &self.nav_next_page,
            &self.nav_prev_page,
            &self.page_down,
            &self.page_up,
            &self.section_prev,
            &self.section_next,
            &self.tab_1,
            &self.tab_2,
            &self.tab_3,
            &self.next_theme,
            &self.play,
            &self.open_settings,
            &self.search_focus,
            &self.comment,
            &self.toggle_replies,
            &self.up_prev,
            &self.up_next,
            &self.stop_playback,
            &self.comment_sort,
            &self.load_more,
            &self.open_up,
            &self.video_like,
            &self.video_coin,
            &self.video_favorite,
            &self.video_triple,
            &self.watch_later,
            &self.follow,
            &self.send_danmaku,
            &self.live_quality,
            &self.play_order,
            &self.jump_top,
            &self.jump_bottom,
        ]
    }

    fn bindings_mut(&mut self) -> [&mut String; 40] {
        [
            &mut self.quit,
            &mut self.confirm,
            &mut self.back,
            &mut self.refresh,
            &mut self.nav_up,
            &mut self.nav_down,
            &mut self.nav_left,
            &mut self.nav_right,
            &mut self.nav_next_page,
            &mut self.nav_prev_page,
            &mut self.page_down,
            &mut self.page_up,
            &mut self.section_prev,
            &mut self.section_next,
            &mut self.tab_1,
            &mut self.tab_2,
            &mut self.tab_3,
            &mut self.next_theme,
            &mut self.play,
            &mut self.open_settings,
            &mut self.search_focus,
            &mut self.comment,
            &mut self.toggle_replies,
            &mut self.up_prev,
            &mut self.up_next,
            &mut self.stop_playback,
            &mut self.comment_sort,
            &mut self.load_more,
            &mut self.open_up,
            &mut self.video_like,
            &mut self.video_coin,
            &mut self.video_favorite,
            &mut self.video_triple,
            &mut self.watch_later,
            &mut self.follow,
            &mut self.send_danmaku,
            &mut self.live_quality,
            &mut self.play_order,
            &mut self.jump_top,
            &mut self.jump_bottom,
        ]
    }

    /// The default key of the binding at `index` in `get_all_labels`.
    pub fn default_binding(index: usize) -> Option<String> {
        Self::default()
            .bindings()
            .get(index)
            .map(|binding| binding.to_string())
    }

    /// Update a keybinding by its index in `get_all_labels`.
    pub fn update_by_index(&mut self, index: usize, new_key: String) {
        if let Some(binding) = self.bindings_mut().into_iter().nth(index) {
            *binding = new_key;
        }
    }
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DanmakuConfig {
    pub enabled: bool,
    pub display_area: f64,
    pub opacity: f64,
    pub font_scale: f64,
    pub duration: f64,
    pub stroke_width: f64,
    pub line_height: f64,
    pub massive_mode: bool,
    pub font_family: String,
}

impl Default for DanmakuConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            display_area: 0.5,
            opacity: 1.0,
            font_scale: 1.0,
            duration: 7.0,
            stroke_width: if cfg!(target_os = "macos") { 2.0 } else { 2.5 },
            line_height: 1.6,
            massive_mode: false,
            font_family: if cfg!(target_os = "macos") {
                "Yuanti SC"
            } else if cfg!(target_os = "windows") {
                "Microsoft YaHei UI"
            } else {
                "Noto Sans CJK SC"
            }
            .to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub theme: String,
    pub keybindings: Keybindings,
    pub danmaku: DanmakuConfig,
    pub auto_play: bool,
    /// Hide recommendation feeds and start with intentional browsing.
    pub minimal_mode: bool,
    /// Unknown values (for example from a newer release) fall back to the
    /// default instead of rejecting the whole config.
    #[serde(deserialize_with = "deserialize_video_quality_lenient")]
    pub video_quality: VideoQuality,
    /// Draw Nerd Font glyphs; off falls back to text labels.
    pub nerd_font_icons: bool,
}

fn deserialize_video_quality_lenient<'de, D>(deserializer: D) -> Result<VideoQuality, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "silkcircuit-neon".to_string(),
            keybindings: Keybindings::default(),
            danmaku: DanmakuConfig::default(),
            auto_play: true,
            minimal_mode: false,
            video_quality: VideoQuality::default(),
            nerd_font_icons: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VideoQuality {
    #[default]
    Best,
    Q4k,
    Q1080pHigh,
    Q1080p,
    Q720p,
    Q480p,
    Q360p,
}

impl VideoQuality {
    pub const ALL: [Self; 7] = [
        Self::Best,
        Self::Q4k,
        Self::Q1080pHigh,
        Self::Q1080p,
        Self::Q720p,
        Self::Q480p,
        Self::Q360p,
    ];

    pub const fn qn(self) -> i64 {
        match self {
            Self::Best => 127,
            Self::Q4k => 120,
            Self::Q1080pHigh => 116,
            Self::Q1080p => 80,
            Self::Q720p => 64,
            Self::Q480p => 32,
            Self::Q360p => 16,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Best => "最佳",
            Self::Q4k => "4K",
            Self::Q1080pHigh => "1080P+/60",
            Self::Q1080p => "1080P",
            Self::Q720p => "720P",
            Self::Q480p => "480P",
            Self::Q360p => "360P",
        }
    }

    pub fn cycle(self, direction: i32) -> Self {
        let index = Self::ALL
            .iter()
            .position(|quality| *quality == self)
            .unwrap_or(0);
        if direction >= 0 {
            Self::ALL[(index + 1) % Self::ALL.len()]
        } else {
            Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
        }
    }

    /// Highest live-room `qn` allowed by this cap. Live rooms use their own
    /// quality ladder (80 流畅, 150 高清, 250 超清, 400 蓝光, 10000 原画,
    /// 20000 4K, 30000 杜比), so the mapping is by nominal resolution.
    pub const fn live_qn_cap(self) -> Option<i64> {
        match self {
            Self::Best => None,
            Self::Q4k => Some(20000),
            Self::Q1080pHigh | Self::Q1080p => Some(10000),
            Self::Q720p => Some(250),
            Self::Q480p => Some(150),
            Self::Q360p => Some(80),
        }
    }

    pub const fn max_height(self) -> Option<u16> {
        match self {
            Self::Best => None,
            Self::Q4k => Some(2160),
            Self::Q1080pHigh | Self::Q1080p => Some(1080),
            Self::Q720p => Some(720),
            Self::Q480p => Some(480),
            Self::Q360p => Some(360),
        }
    }
}

/// Get the config directory path
fn get_config_dir() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("bilibili-tui");

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    Ok(config_dir)
}

/// Get the credentials file path
fn get_credentials_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join("credentials.json"))
}

/// Get the config file path
fn get_config_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join("config.json"))
}

/// Save credentials to disk
pub fn save_credentials(credentials: &Credentials) -> Result<()> {
    let path = get_credentials_path()?;
    let json = serde_json::to_string_pretty(credentials)?;
    write_private_file(&path, json.as_bytes())?;
    Ok(())
}

/// Load credentials from disk
pub fn load_credentials() -> Result<Credentials> {
    let path = get_credentials_path()?;
    let json = fs::read_to_string(path)?;
    let credentials: Credentials = serde_json::from_str(&json)?;
    Ok(credentials)
}

/// Delete credentials (logout)
pub fn delete_credentials() -> Result<()> {
    let path = get_credentials_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Save app config to disk
pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = get_config_path()?;
    let json = serde_json::to_string_pretty(config)?;
    // Write through a temporary file and rename so a crash or full disk
    // cannot leave a truncated config behind.
    write_private_file(&path, json.as_bytes())
}

/// Load app config from disk
pub fn load_config() -> Result<AppConfig> {
    load_config_from(&get_config_path()?)
}

fn load_config_from(path: &std::path::Path) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let json = fs::read_to_string(path)?;
    match parse_config(&json) {
        Ok(config) => Ok(config),
        Err(error) => {
            // Keep the unreadable file so the next save of default settings
            // cannot destroy what the user had.
            let backup = path.with_extension(format!(
                "json.broken-{}",
                chrono::Utc::now().format("%Y%m%d%H%M%S")
            ));
            fs::rename(path, &backup).with_context(|| {
                format!("failed to back up unreadable config {}", path.display())
            })?;
            Err(error.context(format!(
                "config could not be parsed; moved it to {}",
                backup.display()
            )))
        }
    }
}

fn parse_config(json: &str) -> Result<AppConfig> {
    let mut config: AppConfig = serde_json::from_str(json)?;
    // Backward compatibility for dynamic UP navigation defaults:
    // old defaults were h/l, and a previous regression used [ and /.
    if (config.keybindings.up_prev == "h" && config.keybindings.up_next == "l")
        || (config.keybindings.up_prev == "[" && config.keybindings.up_next == "/")
    {
        config.keybindings.up_prev = "[".to_string();
        config.keybindings.up_next = "]".to_string();
    }
    Ok(config)
}

/// Export cookies in Netscape format for yt-dlp
pub fn export_cookies_for_ytdlp(credentials: &Credentials) -> Result<PathBuf> {
    let sequence = COOKIE_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = get_config_dir()?.join(format!("cookies-{}-{sequence}.txt", std::process::id()));

    let content = format!(
        "# Netscape HTTP Cookie File\n\
        .bilibili.com\tTRUE\t/\tTRUE\t0\tSESSDATA\t{}\n\
        .bilibili.com\tTRUE\t/\tFALSE\t0\tbili_jct\t{}\n\
        .bilibili.com\tTRUE\t/\tFALSE\t0\tDedeUserID\t{}\n",
        credentials.sessdata, credentials.bili_jct, credentials.dede_user_id
    );

    write_private_file(&path, content.as_bytes())?;
    Ok(path)
}

/// Remove cookie exports left behind by earlier processes that exited without
/// cleaning up (crash, SIGKILL). Exports owned by live processes are kept.
pub fn remove_stale_cookie_exports() {
    let Ok(dir) = get_config_dir() else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let current_pid = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_stale_cookie_export(name, current_pid, process_is_alive) {
            let _ = remove_cookie_export(&entry.path());
        }
    }
}

fn is_stale_cookie_export(name: &str, current_pid: u32, is_alive: impl Fn(u32) -> bool) -> bool {
    let Some(pid) = name
        .strip_prefix("cookies-")
        .and_then(|rest| rest.strip_suffix(".txt"))
        .and_then(|rest| rest.split_once('-'))
        .filter(|(_, sequence)| sequence.parse::<u64>().is_ok())
        .and_then(|(pid, _)| pid.parse::<u32>().ok())
    else {
        return false;
    };
    pid != current_pid && !is_alive(pid)
}

fn process_is_alive(pid: u32) -> bool {
    let proc_root = std::path::Path::new("/proc");
    if proc_root.is_dir() {
        proc_root.join(pid.to_string()).exists()
    } else {
        // Without procfs the owner cannot be checked; keep the file.
        true
    }
}

/// Remove a temporary cookie export. This is safe to call on error paths and
/// succeeds when the file has already been removed.
pub fn remove_cookie_export(path: &std::path::Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn cookie_exports_of_dead_processes_are_stale() {
        let alive = |pid: u32| pid == 200;
        assert!(is_stale_cookie_export("cookies-100-1.txt", 300, alive));
        assert!(!is_stale_cookie_export("cookies-200-1.txt", 300, alive));
        assert!(!is_stale_cookie_export("cookies-300-4.txt", 300, alive));
        assert!(!is_stale_cookie_export("credentials.json", 300, alive));
        assert!(!is_stale_cookie_export("cookies-100.txt", 300, alive));
        assert!(!is_stale_cookie_export("cookies-x-1.txt", 300, alive));
    }

    #[test]
    fn legacy_config_receives_default_danmaku_settings() {
        let value = serde_json::json!({
            "theme": "silkcircuit-neon",
            "keybindings": Keybindings::default(),
        });
        let config: AppConfig = serde_json::from_value(value).expect("legacy config");
        assert_eq!(config.danmaku, DanmakuConfig::default());
        assert_eq!(config.video_quality, VideoQuality::Best);
        assert!(!config.minimal_mode);
    }

    #[test]
    fn config_without_theme_or_keybindings_keeps_other_settings() {
        let config = parse_config(r#"{"auto_play": false, "minimal_mode": true}"#).unwrap();
        assert_eq!(config.theme, AppConfig::default().theme);
        assert!(!config.auto_play);
        assert!(config.minimal_mode);
    }

    #[test]
    fn unknown_video_quality_falls_back_without_losing_config() {
        let config = parse_config(r#"{"video_quality": "q8k", "auto_play": false}"#).unwrap();
        assert_eq!(config.video_quality, VideoQuality::Best);
        assert!(!config.auto_play);
    }

    #[test]
    fn unreadable_config_is_backed_up_instead_of_overwritten() {
        let dir = std::env::temp_dir().join(format!(
            "bilibili-tui-config-test-{}-{}",
            std::process::id(),
            COOKIE_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, r#"{"theme": "my-theme", "auto_pl"#).unwrap();

        assert!(load_config_from(&path).is_err());
        assert!(!path.exists());
        let backups: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| fs::read_to_string(entry.path()).unwrap())
            .collect();
        assert_eq!(backups, [r#"{"theme": "my-theme", "auto_pl"#]);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn minimal_mode_round_trips() {
        let config = AppConfig {
            minimal_mode: true,
            ..AppConfig::default()
        };
        let restored: AppConfig =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(restored.minimal_mode);
    }

    #[test]
    fn video_quality_has_stable_qn_mapping_and_round_trips() {
        assert_eq!(VideoQuality::Q4k.qn(), 120);
        assert_eq!(VideoQuality::Q1080pHigh.qn(), 116);
        let value = serde_json::to_value(VideoQuality::Q720p).unwrap();
        assert_eq!(value, "q720p");
        assert_eq!(
            serde_json::from_value::<VideoQuality>(value).unwrap(),
            VideoQuality::Q720p
        );
    }

    #[test]
    fn platform_default_danmaku_font_is_explicit() {
        let font = DanmakuConfig::default().font_family;
        if cfg!(target_os = "macos") {
            assert_eq!(font, "Yuanti SC");
        } else if cfg!(target_os = "windows") {
            assert_eq!(font, "Microsoft YaHei UI");
        } else {
            assert_eq!(font, "Noto Sans CJK SC");
        }
    }

    #[test]
    fn stop_playback_key_defaults_for_old_configs_and_is_editable() {
        let keys: Keybindings = serde_json::from_value(serde_json::json!({"quit": "Q"})).unwrap();
        assert_eq!(keys.stop_playback, "x");
        assert!(keys.matches_stop_playback(KeyCode::Char('x')));
        let mut keys = keys;
        keys.update_by_index(25, "X".to_string());
        assert_eq!(keys.stop_playback, "X");
    }

    #[test]
    fn new_bindings_default_for_old_configs_and_follow_the_settings_order() {
        let keys: Keybindings = serde_json::from_value(serde_json::json!({"quit": "Q"})).unwrap();
        assert_eq!(keys.comment_sort, "o");
        assert_eq!(keys.play_order, "s");
        let labels = keys.get_all_labels();
        assert_eq!(labels.len(), Keybindings::LABELS.len());
        assert_eq!(labels[25], ("停止播放", "x"));
        assert_eq!(labels[37], ("播放顺序", "s"));
        let last = labels.len() - 1;
        assert_eq!(labels[last], ("跳到末尾", "G"));
        let mut keys = keys;
        keys.update_by_index(37, "S".into());
        assert_eq!(keys.play_order, "S");
        keys.update_by_index(last, "E".into());
        assert_eq!(keys.jump_bottom, "E");
        keys.update_by_index(last + 1, "Z".into());
        assert_eq!(keys.jump_bottom, "E", "out-of-range index changes nothing");
    }

    #[test]
    fn theme_key_is_global_only_while_nothing_else_uses_it() {
        let mut keys = Keybindings::default();
        assert!(keys.is_dedicated_theme_key(KeyCode::Char('t')));
        assert!(!keys.is_dedicated_theme_key(KeyCode::Char('T')));
        keys.next_theme = "s".into(); // also "设置" and "播放顺序"
        assert!(!keys.is_dedicated_theme_key(KeyCode::Char('s')));
        keys.next_theme = "i".into(); // always opens search
        assert!(!keys.is_dedicated_theme_key(KeyCode::Char('i')));
    }
}
