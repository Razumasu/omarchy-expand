//! A small text editor for form fields: one line for keywords and names, several for snippets.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

#[derive(Debug, Clone)]
pub struct TextInput {
    lines: Vec<String>,
    row: usize,
    /// Counted in characters, not bytes.
    column: usize,
    multiline: bool,
}

impl TextInput {
    pub fn single_line(text: &str) -> Self {
        Self::new(&text.replace('\n', " "), false)
    }

    pub fn multiline(text: &str) -> Self {
        Self::new(text, true)
    }

    fn new(text: &str, multiline: bool) -> Self {
        let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        let row = lines.len() - 1;
        let column = lines[row].chars().count();
        Self {
            lines,
            row,
            column,
            multiline,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Returns whether the key edited or moved within the text.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let shortcut = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Char(character) if !shortcut => self.insert(character),
            KeyCode::Enter if self.multiline => self.newline(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Up if self.multiline => self.vertical(-1),
            KeyCode::Down if self.multiline => self.vertical(1),
            KeyCode::Home => self.column = 0,
            KeyCode::End => self.column = self.line_length(),
            _ => return false,
        }
        true
    }

    pub fn paste(&mut self, text: &str) {
        for character in text.chars() {
            match character {
                '\n' if self.multiline => self.newline(),
                '\n' | '\t' => self.insert(' '),
                '\r' => {}
                character => self.insert(character),
            }
        }
    }

    /// Draws the text scrolled so the cursor stays visible, and places the cursor when focused.
    pub fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let top = self.row.saturating_sub(usize::from(area.height) - 1);
        let before_cursor: String = self.lines[self.row].chars().take(self.column).collect();
        let cursor_x = Span::raw(before_cursor).width();
        let left = cursor_x.saturating_sub(usize::from(area.width) - 1);

        let lines: Vec<Line> = self
            .lines
            .iter()
            .map(|line| Line::raw(line.as_str()))
            .collect();
        frame.render_widget(
            Paragraph::new(lines).scroll((top as u16, left as u16)),
            area,
        );
        if focused {
            frame.set_cursor_position(Position::new(
                area.x + (cursor_x - left) as u16,
                area.y + (self.row - top) as u16,
            ));
        }
    }

    fn byte_index(&self) -> usize {
        let line = &self.lines[self.row];
        line.char_indices()
            .nth(self.column)
            .map_or(line.len(), |(index, _)| index)
    }

    fn line_length(&self) -> usize {
        self.lines[self.row].chars().count()
    }

    fn insert(&mut self, character: char) {
        let index = self.byte_index();
        self.lines[self.row].insert(index, character);
        self.column += 1;
    }

    fn newline(&mut self) {
        let index = self.byte_index();
        let rest = self.lines[self.row].split_off(index);
        self.row += 1;
        self.lines.insert(self.row, rest);
        self.column = 0;
    }

    fn backspace(&mut self) {
        if self.column > 0 {
            self.column -= 1;
            let index = self.byte_index();
            self.lines[self.row].remove(index);
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.column = self.line_length();
            self.lines[self.row].push_str(&line);
        }
    }

    fn delete(&mut self) {
        if self.column < self.line_length() {
            let index = self.byte_index();
            self.lines[self.row].remove(index);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    fn left(&mut self) {
        if self.column > 0 {
            self.column -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.column = self.line_length();
        }
    }

    fn right(&mut self) {
        if self.column < self.line_length() {
            self.column += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = 0;
        }
    }

    fn vertical(&mut self, step: isize) {
        let Some(row) = self
            .row
            .checked_add_signed(step)
            .filter(|row| *row < self.lines.len())
        else {
            return;
        };
        self.row = row;
        self.column = self.column.min(self.line_length());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(input: &mut TextInput, code: KeyCode) {
        input.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn edits_multibyte_text_by_character() {
        let mut input = TextInput::single_line("blåbær");
        press(&mut input, KeyCode::Left);
        press(&mut input, KeyCode::Backspace);
        press(&mut input, KeyCode::Char('e'));
        assert_eq!(input.text(), "blåber");
    }

    #[test]
    fn multiline_joins_and_splits_lines() {
        let mut input = TextInput::multiline("Hej\nverden");
        press(&mut input, KeyCode::Home);
        press(&mut input, KeyCode::Backspace);
        assert_eq!(input.text(), "Hejverden");
        press(&mut input, KeyCode::Enter);
        assert_eq!(input.text(), "Hej\nverden");
    }

    #[test]
    fn single_line_flattens_pasted_newlines() {
        let mut input = TextInput::single_line("");
        input.paste("a\nb\r\n");
        assert_eq!(input.text(), "a b ");
        let mut snippet = TextInput::multiline("");
        snippet.paste("a\r\nb");
        assert_eq!(snippet.text(), "a\nb");
    }

    #[test]
    fn shortcuts_are_not_typed() {
        let mut input = TextInput::single_line("x");
        let handled = input.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!handled);
        assert_eq!(input.text(), "x");
    }
}
