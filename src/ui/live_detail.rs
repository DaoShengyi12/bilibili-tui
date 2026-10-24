//! Live streaming detail page with room info and real-time messages

use super::{Component, Theme, footer_notice, shortcut_footer};
use crate::api::live::{LiveQuality, LiveRoomInfo};
use crate::api::live_danmaku_hub::LiveDanmakuHub;
use crate::api::live_ws::{HistoryDanmakuData, LiveMessage};
use crate::application::AppAction;
use crate::storage::Keybindings;
use ratatui::crossterm::event::KeyCode;
use ratatui::{prelude::*, widgets::*};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast;

/// Maximum number of messages to keep in buffers
const MAX_MESSAGES: usize = 100;

/// Danmaku item for display
#[derive(Debug, Clone)]
struct DanmakuItem {
    uname: String,
    content: String,
    color: Color,
    #[allow(dead_code)]
    timestamp: Instant,
}

/// Interaction shown in the lower panel: entries, follows, shares,
/// gifts and guard purchases.
#[derive(Debug, Clone)]
struct EntryItem {
    text: String,
    highlight: bool,
}

/// Paid pinned message (醒目留言).
#[derive(Debug, Clone)]
struct SuperChatItem {
    uname: String,
    message: String,
    price: i64,
}

/// Quality list opened with the live quality key.
#[derive(Debug, Default)]
pub struct QualityPicker {
    pub loading: bool,
    pub qualities: Vec<LiveQuality>,
    pub selected: usize,
    pub error: Option<String>,
}

fn guard_name(level: i64) -> &'static str {
    match level {
        1 => "总督",
        2 => "提督",
        _ => "舰长",
    }
}

pub struct LiveDetailPage {
    room_id: i64,
    room_info: Option<LiveRoomInfo>,
    loading: bool,
    error: Option<String>,

    // Shared WebSocket hub and this page's independent subscription.
    live_hub: Option<Arc<LiveDanmakuHub>>,
    live_rx: Option<broadcast::Receiver<LiveMessage>>,
    ws_connected: bool,
    ws_error: Option<String>,

    // Message buffers
    danmakus: VecDeque<DanmakuItem>,
    entries: VecDeque<EntryItem>,
    super_chats: VecDeque<SuperChatItem>,
    popularity: Option<u32>,
    history_loaded: bool,

    /// Typing a danmaku to send.
    pub input_mode: bool,
    pub input_buffer: String,
    pub quality_picker: Option<QualityPicker>,
    /// Transient message shown in the footer until the next key press.
    pub notice: Option<String>,
}

impl LiveDetailPage {
    pub fn new(room_id: i64) -> Self {
        Self {
            room_id,
            room_info: None,
            loading: false,
            error: None,
            live_hub: None,
            live_rx: None,
            ws_connected: false,
            ws_error: None,
            danmakus: VecDeque::with_capacity(MAX_MESSAGES),
            entries: VecDeque::with_capacity(MAX_MESSAGES),
            super_chats: VecDeque::new(),
            popularity: None,
            history_loaded: false,
            input_mode: false,
            input_buffer: String::new(),
            quality_picker: None,
            notice: None,
        }
    }

    /// Whether keys go to the danmaku box or the quality picker.
    pub fn captures_keys(&self) -> bool {
        self.input_mode || self.quality_picker.is_some()
    }

    /// Reopen the danmaku box with a message that could not be sent.
    pub fn restore_danmaku_draft(&mut self, message: String, notice: String) {
        self.input_mode = true;
        self.input_buffer = message;
        self.notice = Some(notice);
    }

    pub fn apply_danmaku_sent(&mut self) {
        self.notice = Some("弹幕已发送".to_string());
    }

    pub fn apply_qualities(&mut self, qualities: Vec<LiveQuality>) {
        let Some(picker) = self.quality_picker.as_mut() else {
            return;
        };
        picker.loading = false;
        picker.error = qualities
            .is_empty()
            .then(|| "没有不超过画质上限的清晰度".to_string());
        picker.qualities = qualities;
        picker.selected = 0;
    }

    pub fn apply_qualities_error(&mut self, error: String) {
        if let Some(picker) = self.quality_picker.as_mut() {
            picker.loading = false;
            picker.error = Some(error);
        }
    }

    fn push_entry(&mut self, text: String, highlight: bool) {
        self.entries.push_back(EntryItem { text, highlight });
        while self.entries.len() > MAX_MESSAGES {
            self.entries.pop_front();
        }
    }

    fn handle_picker_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        let picker = self.quality_picker.as_mut()?;
        if keys.matches_back(key) || keys.matches_quit(key) {
            self.quality_picker = None;
        } else if keys.matches_down(key) {
            if picker.selected + 1 < picker.qualities.len() {
                picker.selected += 1;
            }
        } else if keys.matches_up(key) {
            picker.selected = picker.selected.saturating_sub(1);
        } else if keys.matches_confirm(key)
            && let Some(quality) = picker.qualities.get(picker.selected)
        {
            let qn = quality.qn;
            self.quality_picker = None;
            return Some(AppAction::PlayLive {
                room_id: self.room_id,
                title: self
                    .room_info
                    .as_ref()
                    .map(|info| info.title.clone())
                    .unwrap_or_default(),
                quality: Some(qn),
            });
        }
        Some(AppAction::None)
    }

    fn handle_danmaku_input(&mut self, key: KeyCode, keys: &Keybindings) -> AppAction {
        if keys.matches_back(key) {
            self.input_mode = false;
            self.input_buffer.clear();
        } else if key == KeyCode::Enter {
            let message = self.input_buffer.trim().to_string();
            if !message.is_empty() {
                self.input_mode = false;
                self.input_buffer.clear();
                return AppAction::SendLiveDanmaku {
                    room_id: self.room_id,
                    message,
                };
            }
        } else if key == KeyCode::Backspace {
            self.input_buffer.pop();
        } else if let KeyCode::Char(c) = key {
            self.input_buffer.push(c);
        }
        AppAction::None
    }

    pub fn room_id(&self) -> i64 {
        self.room_id
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn begin_loading(&mut self) {
        self.loading = true;
        self.error = None;
    }

    pub fn apply_room_info(&mut self, info: LiveRoomInfo) {
        self.room_info = Some(info);
        self.loading = false;
    }

    pub fn apply_room_info_error(&mut self, msg: String) {
        self.error = Some(msg);
        self.loading = false;
    }

    /// Shows recent danmaku before the WebSocket delivers new ones.
    pub fn apply_history_danmaku(&mut self, history: HistoryDanmakuData) {
        if self.history_loaded {
            return;
        }
        // Merge admin and room messages, take up to 30 most recent
        for item in history.admin.iter().chain(history.room.iter()).take(30) {
            self.danmakus.push_back(DanmakuItem {
                uname: item.nickname.clone(),
                content: item.text.clone(),
                color: Color::White, // History messages use default color
                timestamp: Instant::now(),
            });
        }
        self.history_loaded = true;
    }

    pub fn attach_danmaku_hub(&mut self, hub: Arc<LiveDanmakuHub>) {
        self.live_rx = Some(hub.subscribe());
        self.live_hub = Some(hub);
        self.ws_connected = false;
        self.ws_error = None;
    }

    pub fn set_ws_error(&mut self, error: impl Into<String>) {
        self.ws_connected = false;
        self.ws_error = Some(error.into());
    }

    /// Poll for new messages from WebSocket
    pub fn poll_messages(&mut self) {
        if let Some(hub) = self.live_hub.as_ref() {
            if let Some(error) = hub.take_error() {
                self.ws_connected = false;
                self.ws_error = Some(format!("WebSocket连接异常: {error}"));
            }
            if hub.is_connected() {
                self.ws_connected = true;
                self.ws_error = None;
            }
        }
        let mut messages = Vec::new();
        if let Some(receiver) = self.live_rx.as_mut() {
            loop {
                match receiver.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                    Err(broadcast::error::TryRecvError::Empty) => break,
                    Err(broadcast::error::TryRecvError::Closed) => {
                        self.ws_connected = false;
                        self.ws_error = Some("直播弹幕通道已关闭".to_string());
                        break;
                    }
                }
            }
        }

        // Process collected messages
        for msg in messages {
            self.handle_message(msg);
        }
    }

    /// Handle a received live message
    fn handle_message(&mut self, msg: LiveMessage) {
        match msg {
            LiveMessage::Danmaku {
                uname,
                content,
                color,
                ..
            } => {
                // Convert color from u32 to ratatui Color
                let r = ((color >> 16) & 0xFF) as u8;
                let g = ((color >> 8) & 0xFF) as u8;
                let b = (color & 0xFF) as u8;
                let color = Color::Rgb(r, g, b);

                self.danmakus.push_back(DanmakuItem {
                    uname,
                    content,
                    color,
                    timestamp: Instant::now(),
                });

                // Keep buffer size limited
                while self.danmakus.len() > MAX_MESSAGES {
                    self.danmakus.pop_front();
                }
            }
            LiveMessage::Enter { uname, .. } => {
                self.push_entry(format!("欢迎 {uname} 进入直播间"), false);
            }
            LiveMessage::Follow { uname, .. } => {
                self.push_entry(format!("❤ {uname} 关注了主播"), true);
            }
            LiveMessage::Share { uname, .. } => {
                self.push_entry(format!("↗ {uname} 分享了直播间"), false);
            }
            LiveMessage::Gift {
                uname,
                gift_name,
                num,
                ..
            } => {
                self.push_entry(format!("🎁 {uname} 送出 {gift_name} ×{num}"), false);
            }
            LiveMessage::Guard {
                uname,
                guard_level,
                num,
                ..
            } => {
                self.push_entry(
                    format!("⚓ {uname} 开通了{} ×{num}", guard_name(guard_level)),
                    true,
                );
            }
            LiveMessage::SuperChat {
                uname,
                message,
                price,
                ..
            } => {
                self.super_chats.push_back(SuperChatItem {
                    uname,
                    message,
                    price,
                });
                while self.super_chats.len() > MAX_MESSAGES {
                    self.super_chats.pop_front();
                }
            }
            LiveMessage::Popularity(count) => {
                self.popularity = Some(count);
            }
            LiveMessage::AuthReply { code } if code != 0 => {
                self.ws_error = Some(format!("WebSocket认证失败: {}", code));
            }
            _ => {}
        }
    }

    fn format_online(online: i64) -> String {
        if online >= 10000 {
            format!("{:.1}万", online as f64 / 10000.0)
        } else {
            format!("{}", online)
        }
    }

    fn format_attention(attention: i64) -> String {
        if attention >= 10000 {
            format!("{:.1}万", attention as f64 / 10000.0)
        } else {
            format!("{}", attention)
        }
    }
}

impl Component for LiveDetailPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, keys: &Keybindings) {
        // Poll for new messages
        self.poll_messages();

        // Main block
        let title = if let Some(ref info) = self.room_info {
            format!(" 📺 {} ", info.title)
        } else {
            " 📺 直播详情 ".to_string()
        };

        let block = Block::default()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(theme.bilibili_pink)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.loading && self.room_info.is_none() {
            let loading = Paragraph::new("加载中...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_muted));
            frame.render_widget(loading, inner);
            return;
        }

        if let Some(ref err) = self.error {
            let error = Paragraph::new(format!("加载失败: {}", err))
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.bilibili_pink));
            frame.render_widget(error, inner);
            return;
        }

        if let Some(ref info) = self.room_info {
            self.render_room_content(frame, inner, info, theme, keys);
        }
    }

    fn handle_input_with_modifiers(
        &mut self,
        key: KeyCode,
        modifiers: crossterm::event::KeyModifiers,
        keys: &Keybindings,
    ) -> Option<AppAction> {
        if self.input_mode && super::text::apply_line_edit(&mut self.input_buffer, key, modifiers) {
            self.notice = None;
            return Some(AppAction::None);
        }
        self.handle_input(key, keys)
    }

    fn handle_input(&mut self, key: KeyCode, keys: &Keybindings) -> Option<AppAction> {
        self.notice = None;
        if self.input_mode {
            return Some(self.handle_danmaku_input(key, keys));
        }
        if self.quality_picker.is_some() {
            return self.handle_picker_input(key, keys);
        }
        if keys.matches_quit(key) || keys.matches_back(key) {
            Some(AppAction::BackToList)
        } else if keys.matches_confirm(key) || keys.matches_play(key) {
            self.room_info.as_ref().map(|info| AppAction::PlayLive {
                room_id: info.room_id,
                title: info.title.clone(),
                quality: None,
            })
        } else if keys.matches_send_danmaku(key) {
            self.input_mode = true;
            self.input_buffer.clear();
            Some(AppAction::None)
        } else if keys.matches_live_quality(key) {
            self.quality_picker = Some(QualityPicker {
                loading: true,
                ..QualityPicker::default()
            });
            Some(AppAction::LoadLiveQualities(self.room_id))
        } else if keys.matches_next_theme(key) {
            Some(AppAction::NextTheme)
        } else if keys.matches_open_settings(key) {
            Some(AppAction::SwitchToSettings)
        } else {
            Some(AppAction::None)
        }
    }
}

impl LiveDetailPage {
    fn render_room_content(
        &self,
        frame: &mut Frame,
        area: Rect,
        info: &LiveRoomInfo,
        theme: &Theme,
        keys: &Keybindings,
    ) {
        // Layout: main content + bottom hints
        let input_height = if self.input_mode { 3 } else { 0 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(10),              // Content
                Constraint::Length(input_height), // Danmaku box
                Constraint::Length(1),            // Hints
            ])
            .split(area);

        // Content layout: left info panel + right messages
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(30), // Room info
                Constraint::Min(20),    // Messages
            ])
            .split(chunks[0]);

        // Right side: messages + entries
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(70), // Danmakus
                Constraint::Percentage(30), // Entries
            ])
            .split(content_chunks[1]);

        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(9), Constraint::Min(3)])
            .split(content_chunks[0]);

        // Room info panel
        self.render_info_panel(frame, left_chunks[0], info, theme);
        self.render_super_chat_panel(frame, left_chunks[1], theme);

        // Danmaku panel
        self.render_danmaku_panel(frame, right_chunks[0], theme);

        // Entry panel
        self.render_entry_panel(frame, right_chunks[1], theme);

        if self.input_mode {
            let input = Paragraph::new(format!("{}_", self.input_buffer))
                .style(Style::default().fg(theme.fg_primary))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.bilibili_pink))
                        .title(" ✏️ 发送弹幕 "),
                );
            frame.render_widget(input, chunks[1]);
        }

        // Bottom hints
        let footer = if let Some(notice) = &self.notice {
            footer_notice(theme, notice)
        } else if self.input_mode {
            shortcut_footer(
                theme,
                [
                    ("Enter".to_string(), "发送".into(), theme.success),
                    (keys.back.clone(), "取消".into(), theme.info),
                ],
            )
        } else {
            shortcut_footer(
                theme,
                [
                    (
                        format!("{}/{}", keys.confirm, keys.play),
                        "播放".into(),
                        theme.success,
                    ),
                    (keys.live_quality.clone(), "选画质".into(), theme.fg_accent),
                    (keys.send_danmaku.clone(), "发弹幕".into(), theme.info),
                    (keys.stop_playback.clone(), "停止播放".into(), theme.warning),
                    (
                        format!("{}/{}", keys.back, keys.quit),
                        "返回".into(),
                        theme.info,
                    ),
                ],
            )
        };
        frame.render_widget(
            Paragraph::new(footer).alignment(Alignment::Center),
            chunks[2],
        );
        self.render_quality_picker(frame, area, theme, keys);
    }

    fn render_quality_picker(
        &self,
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
        keys: &Keybindings,
    ) {
        let Some(picker) = &self.quality_picker else {
            return;
        };
        let height = (picker.qualities.len() as u16 + 4).max(5).min(area.height);
        let width = 30.min(area.width);
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        let mut lines: Vec<Line> = if picker.loading {
            vec![Line::from("加载中...")]
        } else if let Some(error) = &picker.error {
            vec![Line::from(Span::styled(
                error.clone(),
                Style::default().fg(theme.error),
            ))]
        } else {
            picker
                .qualities
                .iter()
                .enumerate()
                .map(|(index, quality)| {
                    let selected = index == picker.selected;
                    Line::from(Span::styled(
                        format!("{} {}", if selected { "▶" } else { " " }, quality.label),
                        if selected {
                            Style::default()
                                .fg(theme.bilibili_pink)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme.fg_primary)
                        },
                    ))
                })
                .collect()
        };
        lines.push(shortcut_footer(
            theme,
            [
                (keys.confirm.clone(), "播放".into(), theme.success),
                (keys.back.clone(), "关闭".into(), theme.info),
            ],
        ));
        let dialog = Paragraph::new(lines)
            .style(Style::default().bg(theme.bg_primary))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_focused))
                    .title(" 直播画质 "),
            );
        frame.render_widget(Clear, popup);
        frame.render_widget(dialog, popup);
    }

    fn render_super_chat_panel(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .title(Span::styled(
                format!(" 醒目留言 ({}) ", self.super_chats.len()),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        // Newest first; each message takes two lines.
        let lines: Vec<Line> = self
            .super_chats
            .iter()
            .rev()
            .take((inner.height as usize).div_ceil(2))
            .flat_map(|sc| {
                [
                    Line::from(Span::styled(
                        format!("¥{} {}", sc.price, sc.uname),
                        Style::default().fg(theme.warning),
                    )),
                    Line::from(Span::styled(
                        sc.message.clone(),
                        Style::default().fg(theme.fg_primary),
                    )),
                ]
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
    }

    fn render_info_panel(&self, frame: &mut Frame, area: Rect, info: &LiveRoomInfo, theme: &Theme) {
        let block = Block::default()
            .title(Span::styled(
                " 房间信息 ",
                Style::default()
                    .fg(theme.fg_secondary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let status_color = match info.live_status {
            1 => theme.success,
            2 => theme.fg_accent,
            _ => theme.fg_muted,
        };

        // Use popularity from WebSocket if available
        let online_str = if let Some(pop) = self.popularity {
            Self::format_online(pop as i64)
        } else {
            Self::format_online(info.online)
        };

        let lines = vec![
            Line::from(vec![
                Span::styled("ID: ", Style::default().fg(theme.fg_muted)),
                Span::styled(
                    format!("{}", info.room_id),
                    Style::default().fg(theme.fg_primary),
                ),
            ]),
            Line::from(vec![
                Span::styled("分区: ", Style::default().fg(theme.fg_muted)),
                Span::styled(&info.area_name, Style::default().fg(theme.fg_accent)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("● ", Style::default().fg(status_color)),
                Span::styled(info.status_text(), Style::default().fg(status_color)),
            ]),
            Line::from(vec![
                Span::styled("👁 ", Style::default().fg(theme.fg_muted)),
                Span::styled(online_str, Style::default().fg(theme.fg_accent)),
            ]),
            Line::from(vec![
                Span::styled("❤ ", Style::default().fg(theme.bilibili_pink)),
                Span::styled(
                    Self::format_attention(info.attention),
                    Style::default().fg(theme.bilibili_pink),
                ),
            ]),
        ];

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
        frame.render_widget(paragraph, inner);
    }

    fn render_danmaku_panel(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .title(Span::styled(
                format!(" 弹幕 ({}) ", self.danmakus.len()),
                Style::default()
                    .fg(theme.fg_secondary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.danmakus.is_empty() {
            let msg = if self.ws_connected {
                "等待弹幕...".to_string()
            } else if let Some(ref err) = self.ws_error {
                // Show the actual error message
                format!("连接失败: {}", err)
            } else {
                "弹幕功能加载中...".to_string()
            };
            let placeholder = Paragraph::new(msg)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.error));
            frame.render_widget(placeholder, inner);
            return;
        }

        // Show recent danmakus (auto-scroll to bottom)
        let visible_lines = inner.height as usize;
        let start = self.danmakus.len().saturating_sub(visible_lines);

        let lines: Vec<Line> = self
            .danmakus
            .iter()
            .skip(start)
            .take(visible_lines)
            .map(|d| {
                Line::from(vec![
                    Span::styled(
                        format!("[{}] ", d.uname),
                        Style::default().fg(theme.fg_muted),
                    ),
                    Span::styled(&d.content, Style::default().fg(d.color)),
                ])
            })
            .collect();

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, inner);
    }

    fn render_entry_panel(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .title(Span::styled(
                " 互动 ",
                Style::default()
                    .fg(theme.fg_secondary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_subtle));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.entries.is_empty() {
            let placeholder = Paragraph::new("等待互动消息...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.fg_muted));
            frame.render_widget(placeholder, inner);
            return;
        }

        // Show recent entries
        let visible_lines = inner.height as usize;
        let start = self.entries.len().saturating_sub(visible_lines);

        let lines: Vec<Line> = self
            .entries
            .iter()
            .skip(start)
            .take(visible_lines)
            .map(|e| {
                Line::from(Span::styled(
                    e.text.clone(),
                    Style::default().fg(if e.highlight {
                        theme.bilibili_pink
                    } else {
                        theme.fg_muted
                    }),
                ))
            })
            .collect();

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, inner);
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;

    fn quality(qn: i64) -> LiveQuality {
        LiveQuality {
            qn,
            label: qn.to_string(),
        }
    }

    #[test]
    fn quality_picker_plays_the_chosen_quality() {
        let keys = Keybindings::default();
        let mut page = LiveDetailPage::new(7);
        assert!(matches!(
            page.handle_input(KeyCode::Char('v'), &keys),
            Some(AppAction::LoadLiveQualities(7))
        ));
        assert!(page.captures_keys());
        page.apply_qualities(vec![quality(10000), quality(400)]);
        page.handle_input(KeyCode::Char('j'), &keys);
        let action = page.handle_input(KeyCode::Enter, &keys);
        assert!(matches!(
            action,
            Some(AppAction::PlayLive {
                room_id: 7,
                quality: Some(400),
                ..
            })
        ));
        assert!(page.quality_picker.is_none());
    }

    #[test]
    fn failed_quality_load_keeps_a_closable_picker() {
        let keys = Keybindings::default();
        let mut page = LiveDetailPage::new(7);
        page.handle_input(KeyCode::Char('v'), &keys);
        page.apply_qualities_error("timeout".into());
        let picker = page.quality_picker.as_ref().unwrap();
        assert!(!picker.loading && picker.error.is_some());
        page.handle_input(KeyCode::Esc, &keys);
        assert!(!page.captures_keys());
    }

    #[test]
    fn danmaku_box_sends_trimmed_text_and_restores_failed_drafts() {
        let keys = Keybindings::default();
        let mut page = LiveDetailPage::new(7);
        page.handle_input(KeyCode::Char('d'), &keys);
        for c in " hi ".chars() {
            page.handle_input(KeyCode::Char(c), &keys);
        }
        let action = page.handle_input(KeyCode::Enter, &keys);
        assert!(matches!(
            action,
            Some(AppAction::SendLiveDanmaku { room_id: 7, ref message }) if message == "hi"
        ));
        assert!(!page.input_mode);
        page.restore_danmaku_draft("hi".into(), "失败".into());
        assert!(page.input_mode && page.input_buffer == "hi");
    }

    #[test]
    fn super_chats_and_guards_are_kept() {
        let mut page = LiveDetailPage::new(7);
        page.handle_message(LiveMessage::SuperChat {
            uid: 1,
            uname: "a".into(),
            message: "m".into(),
            price: 30,
        });
        page.handle_message(LiveMessage::Guard {
            uid: 2,
            uname: "b".into(),
            guard_level: 3,
            num: 1,
        });
        assert_eq!(page.super_chats.len(), 1);
        assert!(page.entries[0].text.contains("舰长") && page.entries[0].highlight);
    }

    #[test]
    fn quality_picker_draws_in_a_tiny_terminal() {
        let mut page = LiveDetailPage::new(7);
        page.quality_picker = Some(QualityPicker::default());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(10, 3)).unwrap();
        terminal
            .draw(|frame| {
                page.render_quality_picker(
                    frame,
                    frame.area(),
                    &Theme::default(),
                    &Keybindings::default(),
                )
            })
            .unwrap();
    }
}
