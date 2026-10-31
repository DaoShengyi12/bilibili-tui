//! Full shortcut list for the page on screen, opened with `?`.

use super::{Page, Theme};
use crate::storage::Keybindings;
use ratatui::{prelude::*, widgets::*};

/// One group of shortcuts: a heading and `(keys, what they do)` rows.
pub type HelpGroup = (&'static str, Vec<(String, &'static str)>);

fn pair(a: &str, b: &str) -> String {
    format!("{a}/{b}")
}

/// Keys that work on every list page.
fn navigation(keys: &Keybindings) -> HelpGroup {
    (
        "通用",
        vec![
            (format!("{} / 方向键", keys.get_nav_keys_display()), "移动"),
            (pair(&keys.page_up, &keys.page_down), "翻页"),
            (
                format!("{}/{} Home/End", keys.jump_top, keys.jump_bottom),
                "跳到开头 / 末尾",
            ),
            ("Ctrl+N/P Ctrl+D/U".to_string(), "移动 / 翻页"),
            (
                pair(&keys.nav_next_page, &keys.nav_prev_page),
                "切换页面或面板",
            ),
            (keys.back.clone(), "返回 / 回到左侧菜单"),
            (keys.stop_playback.clone(), "停止播放"),
            (keys.next_theme.clone(), "切换主题"),
            (format!("{} / F1", super::HELP_KEY), "本帮助"),
            (keys.quit.clone(), "退出（详情页里是返回）"),
            ("Ctrl+C".to_string(), "退出；输入框里是收起"),
        ],
    )
}

/// Keys of the one-line text boxes (search, comments, danmaku).
fn text_input() -> HelpGroup {
    (
        "输入框",
        vec![
            ("←/→ Home/End".to_string(), "移动光标"),
            (
                "Ctrl+A/E/U/K/W".to_string(),
                "行首 / 行尾 / 删到行首 / 删到行尾 / 删词",
            ),
            ("Esc".to_string(), "收起，草稿保留"),
            ("粘贴".to_string(), "整段插入为一行"),
        ],
    )
}

fn comments(keys: &Keybindings) -> HelpGroup {
    (
        "评论区",
        vec![
            (pair(&keys.nav_up, &keys.nav_down), "选择评论"),
            (pair(&keys.page_up, &keys.page_down), "翻页"),
            (
                pair(&keys.jump_bottom, &keys.load_more),
                "跳到最后一条并加载更多",
            ),
            (keys.confirm.clone(), "点赞 / 取消点赞"),
            (pair(&keys.toggle_replies, &keys.nav_right), "查看全部回复"),
            (keys.comment.clone(), "评论；在回复里是回复选中的人"),
            (keys.comment_sort.clone(), "按热度 / 按时间"),
            (keys.back.clone(), "退出回复列表"),
        ],
    )
}

/// The groups for `page`.
pub fn help_for(page: &Page, keys: &Keybindings) -> Vec<HelpGroup> {
    let mut groups = Vec::new();
    match page {
        Page::Home(_) => groups.push((
            "首页",
            vec![
                (keys.confirm.clone(), "打开视频"),
                (keys.open_up.clone(), "打开 UP 主页"),
                (keys.search_focus.clone(), "搜索"),
                (keys.refresh.clone(), "刷新"),
                (keys.load_more.clone(), "跳到末尾并加载更多"),
                (keys.nav_left.clone(), "在第一列时回到菜单"),
                (keys.open_settings.clone(), "设置"),
            ],
        )),
        Page::Search(_) => groups.push((
            "搜索",
            vec![
                (keys.search_focus.clone(), "输入关键词"),
                (
                    pair(&keys.section_prev, &keys.section_next),
                    "切换视频/用户/番剧/直播",
                ),
                (keys.confirm.clone(), "打开"),
                (keys.open_up.clone(), "打开 UP 主页"),
                (keys.load_more.clone(), "跳到末尾并加载更多"),
                ("Esc".to_string(), "收起输入框 / 回到结果"),
            ],
        )),
        Page::VideoDetail(_) => {
            groups.push((
                "视频",
                vec![
                    (keys.play.clone(), "播放"),
                    (
                        pair(&keys.nav_next_page, &keys.nav_prev_page),
                        "评论 / 选集 / 相关推荐",
                    ),
                    (keys.video_like.clone(), "点赞"),
                    (keys.video_coin.clone(), "投币"),
                    (keys.video_favorite.clone(), "收藏"),
                    (keys.video_triple.clone(), "一键三连"),
                    (keys.watch_later.clone(), "稍后再看"),
                    (keys.follow.clone(), "关注 UP 主"),
                    (keys.send_danmaku.clone(), "发弹幕"),
                    (keys.open_up.clone(), "打开 UP 主页"),
                ],
            ));
            groups.push(comments(keys));
            groups.push(text_input());
        }
        Page::DynamicDetail(_) => {
            groups.push((
                "动态",
                vec![
                    (
                        pair(&keys.nav_next_page, &keys.nav_prev_page),
                        "评论 / 正文（正文里上下滚动）",
                    ),
                    (pair(&keys.nav_left, &keys.nav_right), "上一张 / 下一张图"),
                    (keys.play.clone(), "打开视频、专栏、直播或原动态"),
                    (keys.open_up.clone(), "打开 UP 主页"),
                ],
            ));
            groups.push(comments(keys));
            groups.push(text_input());
        }
        Page::ArticleDetail(_) => {
            groups.push((
                "专栏",
                vec![
                    (
                        pair(&keys.nav_next_page, &keys.nav_prev_page),
                        "正文 / 评论",
                    ),
                    (pair(&keys.nav_up, &keys.nav_down), "滚动正文"),
                    (keys.comment.clone(), "写评论"),
                ],
            ));
            groups.push(comments(keys));
            groups.push(text_input());
        }
        Page::Dynamic(_) => groups.push((
            "动态",
            vec![
                (keys.confirm.clone(), "打开"),
                (
                    format!("{}/{}/{}", keys.tab_1, keys.tab_2, keys.tab_3),
                    "全部 / 视频 / 图文",
                ),
                (pair(&keys.up_prev, &keys.up_next), "上一个 / 下一个 UP"),
                (keys.open_up.clone(), "打开 UP 主页"),
                (keys.refresh.clone(), "刷新"),
                (keys.load_more.clone(), "跳到末尾并加载更多"),
                (keys.nav_left.clone(), "在第一列时回到 UP 列表"),
            ],
        )),
        Page::History(_) => groups.push((
            "历史",
            vec![
                (keys.confirm.clone(), "打开"),
                ("Space".to_string(), "选择"),
                ("Ctrl+A / Ctrl+R".to_string(), "全选 / 反选"),
                ("d".to_string(), "删除所选"),
                (keys.open_up.clone(), "打开 UP 主页"),
                (keys.refresh.clone(), "刷新"),
                (keys.load_more.clone(), "跳到末尾并加载更多"),
            ],
        )),
        Page::Favorites(_) => groups.push((
            "收藏夹",
            vec![
                (keys.nav_left.clone(), "在第一列时回到收藏夹列表"),
                (pair(&keys.nav_right, &keys.confirm), "进入收藏夹"),
                (keys.confirm.clone(), "打开视频"),
                (keys.load_more.clone(), "跳到末尾并加载更多"),
            ],
        )),
        Page::Up(_) => groups.push((
            "UP 主页",
            vec![
                (pair(&keys.tab_1, &keys.tab_2), "投稿 / 收藏夹"),
                (keys.confirm.clone(), "打开"),
                (keys.play.clone(), "连播全部"),
                (keys.comment_sort.clone(), "切换排序"),
                (keys.play_order.clone(), "顺序 / 倒序 / 随机"),
                (keys.load_more.clone(), "跳到末尾并加载更多"),
            ],
        )),
        Page::Live(_) => groups.push((
            "直播",
            vec![
                (pair(&keys.confirm, &keys.play), "进入直播间"),
                (keys.refresh.clone(), "刷新"),
                (keys.load_more.clone(), "跳到末尾并加载更多"),
                (keys.open_settings.clone(), "设置"),
            ],
        )),
        Page::LiveDetail(_) => groups.push((
            "直播间",
            vec![
                (pair(&keys.confirm, &keys.play), "播放"),
                (keys.live_quality.clone(), "选择画质"),
                (keys.send_danmaku.clone(), "发弹幕"),
                (keys.open_settings.clone(), "设置"),
            ],
        )),
        Page::Bangumi(_) => groups.push((
            "番剧",
            vec![
                (pair(&keys.confirm, &keys.play), "打开"),
                (keys.refresh.clone(), "刷新"),
            ],
        )),
        Page::BangumiDetail(_) => groups.push((
            "番剧",
            vec![(pair(&keys.confirm, &keys.play), "播放选中集")],
        )),
        Page::Settings(_) => groups.push((
            "设置",
            vec![
                (pair(&keys.section_prev, &keys.section_next), "切换分区"),
                (
                    pair(&keys.nav_left, &keys.nav_right),
                    "调整弹幕 / 播放设置的值",
                ),
                (keys.confirm.clone(), "更改"),
                ("Backspace".to_string(), "改按键时恢复默认"),
            ],
        )),
        Page::Login(_) => groups.push((
            "登录",
            vec![
                (keys.refresh.clone(), "刷新二维码"),
                (keys.back.clone(), "返回之前的页面"),
            ],
        )),
    }
    if matches!(page, Page::Home(_) | Page::Search(_) | Page::LiveDetail(_)) {
        groups.push(text_input());
    }
    groups.push(navigation(keys));
    groups
}

/// Draw the groups as a centered two-column popup scrolled to `scroll`
/// rows, and return that offset clamped to the content.
pub fn render_help(
    frame: &mut Frame,
    area: Rect,
    groups: &[HelpGroup],
    theme: &Theme,
    scroll: u16,
) -> u16 {
    let key_width = groups
        .iter()
        .flat_map(|(_, rows)| rows)
        .map(|(keys, _)| super::text::display_width(keys))
        .max()
        .unwrap_or(4)
        + 2;
    let mut lines = Vec::new();
    for (title, rows) in groups {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            *title,
            Style::default()
                .fg(theme.bilibili_pink)
                .add_modifier(Modifier::BOLD),
        ));
        for (keys, what) in rows {
            lines.push(Line::from(vec![
                Span::styled(
                    super::text::pad_to_width(keys, key_width),
                    Style::default()
                        .fg(theme.fg_accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(*what, Style::default().fg(theme.fg_primary)),
            ]));
        }
    }
    let width = 64.min(area.width);
    let content_rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let height = content_rows.saturating_add(3).min(area.height);
    // Borders take two rows and the bottom title shares the lower border.
    let visible_rows = height.saturating_sub(2);
    let scroll = scroll.min(content_rows.saturating_sub(visible_rows));
    let overflows = content_rows > visible_rows;
    let hint = if overflows {
        format!(
            " {}/{} 滚动 · 其他键关闭 ",
            scroll + visible_rows.min(content_rows),
            content_rows
        )
    } else {
        " 任意键关闭 ".to_string()
    };
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll, 0))
            .style(Style::default().bg(theme.bg_primary))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_focused))
                    .title(" 全部按键 ")
                    .title_bottom(Line::from(hint).centered()),
            ),
        popup,
    );
    scroll
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_lists_its_keys_and_the_common_ones() {
        let keys = Keybindings::default();
        let groups = help_for(
            &Page::VideoDetail(Box::new(super::super::VideoDetailPage::new(
                "BV1".into(),
                1,
            ))),
            &keys,
        );
        let titles: Vec<&str> = groups.iter().map(|(title, _)| *title).collect();
        assert_eq!(titles, ["视频", "评论区", "输入框", "通用"]);
        assert!(groups[0].1.iter().any(|(key, _)| key == &keys.video_triple));
    }

    #[test]
    fn help_draws_in_a_small_terminal() {
        let keys = Keybindings::default();
        let groups = help_for(&Page::Settings(Box::default()), &keys);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 5)).unwrap();
        terminal
            .draw(|frame| {
                render_help(frame, frame.area(), &groups, &Theme::default(), 0);
            })
            .unwrap();
    }

    #[test]
    fn help_scroll_is_clamped_to_the_last_page() {
        let keys = Keybindings::default();
        let groups = help_for(&Page::Settings(Box::default()), &keys);
        let rows: u16 = groups
            .iter()
            .map(|(_, rows)| rows.len() as u16 + 2)
            .sum::<u16>()
            - 1;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(64, 10)).unwrap();
        let mut scroll = 0;
        terminal
            .draw(|frame| {
                scroll = render_help(frame, frame.area(), &groups, &Theme::default(), u16::MAX);
            })
            .unwrap();
        assert_eq!(scroll, rows - 8);
    }
}
