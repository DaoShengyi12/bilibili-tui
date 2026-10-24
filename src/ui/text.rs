//! Terminal-column aware text helpers. CJK characters and most emoji take two
//! columns, so `chars().count()` under-measures titles and comments.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Apply the readline-style shortcuts of a single-line input: Ctrl+U clears
/// the line, Ctrl+W deletes the previous word. Other Ctrl/Alt chords are
/// swallowed so they are not typed as letters. Returns `false` for keys
/// without such modifiers, which the caller handles as plain input.
pub fn apply_line_edit(buffer: &mut String, key: KeyCode, modifiers: KeyModifiers) -> bool {
    let KeyCode::Char(character) = key else {
        return false;
    };
    if modifiers.contains(KeyModifiers::CONTROL) {
        match character.to_ascii_lowercase() {
            'u' => buffer.clear(),
            'w' => delete_previous_word(buffer),
            _ => {}
        }
        return true;
    }
    modifiers.contains(KeyModifiers::ALT)
}

fn delete_previous_word(buffer: &mut String) {
    let trimmed = buffer.trim_end_matches(char::is_whitespace).len();
    buffer.truncate(trimmed);
    let word_start = buffer
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(index, character)| index + character.len_utf8());
    buffer.truncate(word_start);
}

/// Number of terminal columns `text` occupies.
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Cut `text` to at most `max_width` columns, ending with `ellipsis` when cut.
pub fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }
    let budget = max_width.saturating_sub(display_width(ellipsis));
    let mut width = 0;
    let mut output = String::new();
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if width + character_width > budget {
            break;
        }
        width += character_width;
        output.push(character);
    }
    output.push_str(ellipsis);
    output
}

/// Pad `text` with spaces to `width` columns, like `{:<width}` for ASCII.
pub fn pad_to_width(text: &str, width: usize) -> String {
    let padding = width.saturating_sub(display_width(text));
    format!("{text}{}", " ".repeat(padding))
}

/// Wrap `text` into lines of at most `width` columns. Words are kept whole
/// where they fit; text without spaces (common in Chinese) breaks anywhere.
pub fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut line_width = 0;
        for word in paragraph.split_whitespace() {
            let word_width = display_width(word);
            let separator = usize::from(!line.is_empty());
            if line_width + separator + word_width <= width {
                if separator == 1 {
                    line.push(' ');
                }
                line.push_str(word);
                line_width += separator + word_width;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            for character in word.chars() {
                let character_width = character.width().unwrap_or(0);
                if line_width + character_width > width && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    line_width = 0;
                }
                line.push(character);
                line_width += character_width;
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_edit_shortcuts_are_not_typed_as_letters() {
        let mut buffer = "hello big world  ".to_string();
        assert!(apply_line_edit(
            &mut buffer,
            KeyCode::Char('w'),
            KeyModifiers::CONTROL
        ));
        assert_eq!(buffer, "hello big ");
        assert!(apply_line_edit(
            &mut buffer,
            KeyCode::Char('w'),
            KeyModifiers::CONTROL
        ));
        assert_eq!(buffer, "hello ");
        assert!(apply_line_edit(
            &mut buffer,
            KeyCode::Char('x'),
            KeyModifiers::ALT
        ));
        assert_eq!(buffer, "hello ");
        assert!(apply_line_edit(
            &mut buffer,
            KeyCode::Char('u'),
            KeyModifiers::CONTROL
        ));
        assert!(buffer.is_empty());

        let mut chinese = "中文 评论".to_string();
        apply_line_edit(&mut chinese, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(chinese, "中文 ");

        assert!(!apply_line_edit(
            &mut buffer,
            KeyCode::Char('a'),
            KeyModifiers::NONE
        ));
        assert!(!apply_line_edit(
            &mut buffer,
            KeyCode::Char('A'),
            KeyModifiers::SHIFT
        ));
        assert!(!apply_line_edit(
            &mut buffer,
            KeyCode::Backspace,
            KeyModifiers::CONTROL
        ));
    }

    #[test]
    fn cjk_titles_are_measured_in_columns() {
        assert_eq!(display_width("番剧abc"), 7);
        assert_eq!(truncate_to_width("一二三四五六", 7, "…"), "一二三…");
        assert_eq!(display_width(&truncate_to_width("一二三四五六", 7, "…")), 7);
        assert_eq!(truncate_to_width("short", 10, "…"), "short");
    }

    #[test]
    fn padding_aligns_mixed_width_labels() {
        assert_eq!(display_width(&pad_to_width("播放", 12)), 12);
        assert_eq!(display_width(&pad_to_width("Next tab", 12)), 12);
    }

    #[test]
    fn wrapping_breaks_unspaced_chinese_within_the_width() {
        let lines = wrap_to_width("这是一段没有空格的很长的中文评论内容", 10);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| display_width(line) <= 10));
        assert_eq!(lines.concat(), "这是一段没有空格的很长的中文评论内容");

        let lines = wrap_to_width("hello world again", 11);
        assert_eq!(lines, vec!["hello world", "again"]);
        assert_eq!(wrap_to_width("a\nb", 10), vec!["a", "b"]);
    }
}
