use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

use crate::core::models::{HttpMethod, KeyValueField, RequestDocument};

use super::{Component, EventResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorTab {
    Url,
    Headers,
    Params,
    Body,
}

impl EditorTab {
    const ALL: [EditorTab; 4] = [
        EditorTab::Url,
        EditorTab::Headers,
        EditorTab::Params,
        EditorTab::Body,
    ];

    fn label(self) -> &'static str {
        match self {
            EditorTab::Url => "URL",
            EditorTab::Headers => "Headers",
            EditorTab::Params => "Params",
            EditorTab::Body => "Body",
        }
    }

    fn index(self) -> usize {
        match self {
            EditorTab::Url => 0,
            EditorTab::Headers => 1,
            EditorTab::Params => 2,
            EditorTab::Body => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorMode {
    Normal,
    Insert,
}

pub struct RequestEditorPane {
    document: Option<RequestDocument>,
    active_tab: EditorTab,
    mode: EditorMode,
    dirty: bool,

    // URL tab
    method_index: usize,
    url_text: String,
    url_cursor: usize,

    // Headers tab
    headers: Vec<(String, String, bool)>,
    header_row: usize,
    header_col: usize, // 0=key, 1=value

    // Params tab
    params: Vec<(String, String, bool)>,
    param_row: usize,
    param_col: usize,

    // Body tab
    body_text: String,
    body_cursor_line: usize,
    body_cursor_col: usize,

    // Grid inline edit buffer
    edit_buffer: String,
    edit_cursor: usize,
}

impl Default for RequestEditorPane {
    fn default() -> Self {
        Self {
            document: None,
            active_tab: EditorTab::Url,
            mode: EditorMode::Normal,
            dirty: false,
            method_index: 0,
            url_text: String::new(),
            url_cursor: 0,
            headers: Vec::new(),
            header_row: 0,
            header_col: 0,
            params: Vec::new(),
            param_row: 0,
            param_col: 0,
            body_text: String::new(),
            body_cursor_line: 0,
            body_cursor_col: 0,
            edit_buffer: String::new(),
            edit_cursor: 0,
        }
    }
}

impl RequestEditorPane {
    /// Load a request document into the editor.
    pub fn load_document(&mut self, doc: RequestDocument) {
        self.method_index = HttpMethod::ALL
            .iter()
            .position(|&m| m == doc.method)
            .unwrap_or(0);

        self.url_text = doc.url.clone();
        self.url_cursor = self.url_text.len();

        self.headers = doc
            .headers
            .iter()
            .map(|h| (h.key.clone(), h.value.clone(), h.enabled))
            .collect();
        self.header_row = 0;
        self.header_col = 0;

        self.params = doc
            .params
            .iter()
            .map(|p| (p.key.clone(), p.value.clone(), p.enabled))
            .collect();
        self.param_row = 0;
        self.param_col = 0;

        self.body_text = doc.body.clone().unwrap_or_default();
        self.body_cursor_line = 0;
        self.body_cursor_col = 0;

        self.document = Some(doc);
        self.dirty = false;
        self.mode = EditorMode::Normal;
        self.active_tab = EditorTab::Url;
    }

    /// Extract the current editor state as a `RequestDocument`.
    pub fn to_document(&self) -> Option<RequestDocument> {
        let doc = self.document.as_ref()?;
        let method = HttpMethod::ALL[self.method_index];

        let mut header_rows = self.headers.clone();
        let mut param_rows = self.params.clone();

        if self.mode == EditorMode::Insert {
            match self.active_tab {
                EditorTab::Headers => {
                    if let Some(item) = header_rows.get_mut(self.header_row) {
                        if self.header_col == 0 {
                            item.0 = self.edit_buffer.clone();
                        } else {
                            item.1 = self.edit_buffer.clone();
                        }
                    }
                }
                EditorTab::Params => {
                    if let Some(item) = param_rows.get_mut(self.param_row) {
                        if self.param_col == 0 {
                            item.0 = self.edit_buffer.clone();
                        } else {
                            item.1 = self.edit_buffer.clone();
                        }
                    }
                }
                EditorTab::Url | EditorTab::Body => {}
            }
        }

        let headers: Vec<KeyValueField> = header_rows
            .iter()
            .map(|(k, v, e)| KeyValueField {
                key: k.clone(),
                value: v.clone(),
                enabled: *e,
            })
            .collect();
        let params: Vec<KeyValueField> = param_rows
            .iter()
            .map(|(k, v, e)| KeyValueField {
                key: k.clone(),
                value: v.clone(),
                enabled: *e,
            })
            .collect();
        let body = if self.body_text.is_empty() {
            None
        } else {
            Some(self.body_text.clone())
        };

        Some(RequestDocument {
            name: doc.name.clone(),
            method,
            url: self.url_text.clone(),
            headers,
            params,
            body,
            assertions: doc.assertions.clone(),
            file_path: doc.file_path.clone(),
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn sync_pending_edit(&mut self) {
        if self.mode == EditorMode::Insert
            && matches!(self.active_tab, EditorTab::Headers | EditorTab::Params)
        {
            self.commit_edit();
        }
    }

    pub fn set_clean(&mut self) {
        self.dirty = false;
    }

    #[allow(dead_code)]
    pub fn has_document(&self) -> bool {
        self.document.is_some()
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> EventResult {
        match key.code {
            // Tab switching
            KeyCode::Char('1') => {
                self.active_tab = EditorTab::Url;
                EventResult::Consumed
            }
            KeyCode::Char('2') => {
                self.active_tab = EditorTab::Headers;
                EventResult::Consumed
            }
            KeyCode::Char('3') => {
                self.active_tab = EditorTab::Params;
                EventResult::Consumed
            }
            KeyCode::Char('4') => {
                self.active_tab = EditorTab::Body;
                EventResult::Consumed
            }
            KeyCode::Char('h') | KeyCode::Left => {
                let idx = self.active_tab.index();
                if idx > 0 {
                    self.active_tab = EditorTab::ALL[idx - 1];
                }
                EventResult::Consumed
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let idx = self.active_tab.index();
                if idx < EditorTab::ALL.len() - 1 {
                    self.active_tab = EditorTab::ALL[idx + 1];
                }
                EventResult::Consumed
            }

            // Enter insert mode
            KeyCode::Char('i') | KeyCode::Enter => {
                if self.document.is_some() {
                    self.enter_insert_mode();
                }
                EventResult::Consumed
            }

            // Method cycling
            KeyCode::Char('<') | KeyCode::Char('[') => {
                if self.active_tab == EditorTab::Url {
                    if self.method_index > 0 {
                        self.method_index -= 1;
                    } else {
                        self.method_index = HttpMethod::ALL.len() - 1;
                    }
                    self.dirty = true;
                }
                EventResult::Consumed
            }
            KeyCode::Char('>') | KeyCode::Char(']') => {
                if self.active_tab == EditorTab::Url {
                    self.method_index = (self.method_index + 1) % HttpMethod::ALL.len();
                    self.dirty = true;
                }
                EventResult::Consumed
            }

            // Grid navigation
            KeyCode::Char('j') | KeyCode::Down => {
                self.grid_move_down();
                EventResult::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.grid_move_up();
                EventResult::Consumed
            }

            // Add/delete rows
            KeyCode::Char('a') => {
                self.grid_add_row();
                EventResult::Consumed
            }
            KeyCode::Char('d') => {
                self.grid_delete_row();
                EventResult::Consumed
            }

            // Toggle enabled
            KeyCode::Char(' ') => {
                self.grid_toggle_enabled();
                EventResult::Consumed
            }

            _ => EventResult::Ignored,
        }
    }

    fn handle_insert_key(&mut self, key: KeyEvent) -> EventResult {
        if key.code == KeyCode::Esc {
            self.commit_edit();
            self.mode = EditorMode::Normal;
            return EventResult::Consumed;
        }

        match self.active_tab {
            EditorTab::Url => {
                self.handle_text_input(&mut InputTarget::Url, key);
                self.dirty = true;
            }
            EditorTab::Body => {
                self.handle_body_input(key);
                self.dirty = true;
            }
            EditorTab::Headers | EditorTab::Params => {
                if key.code == KeyCode::Tab {
                    self.commit_edit();
                    match self.active_tab {
                        EditorTab::Headers => self.header_col = 1 - self.header_col,
                        EditorTab::Params => self.param_col = 1 - self.param_col,
                        _ => {}
                    }
                    self.load_grid_cell_into_edit();
                } else {
                    self.handle_text_input(&mut InputTarget::EditBuffer, key);
                    self.dirty = true;
                }
            }
        }

        EventResult::Consumed
    }

    fn handle_text_input(&mut self, target: &mut InputTarget, key: KeyEvent) {
        let (text, cursor) = match target {
            InputTarget::Url => (&mut self.url_text, &mut self.url_cursor),
            InputTarget::EditBuffer => (&mut self.edit_buffer, &mut self.edit_cursor),
        };

        match key.code {
            KeyCode::Char(c) => {
                text.insert(*cursor, c);
                *cursor += c.len_utf8();
            }
            KeyCode::Backspace => {
                if *cursor > 0 {
                    let prev = text[..*cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    text.drain(prev..*cursor);
                    *cursor = prev;
                }
            }
            KeyCode::Delete => {
                if *cursor < text.len() {
                    let next_char_len = text[*cursor..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    text.drain(*cursor..*cursor + next_char_len);
                }
            }
            KeyCode::Left => {
                if *cursor > 0 {
                    *cursor = text[..*cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            KeyCode::Right => {
                if *cursor < text.len() {
                    *cursor += text[*cursor..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                }
            }
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = text.len(),
            _ => {}
        }
    }

    fn handle_body_input(&mut self, key: KeyEvent) {
        let lines: Vec<&str> = self.body_text.lines().collect();
        let line_count = lines.len().max(1);

        match key.code {
            KeyCode::Char(c) => {
                let pos = self.body_cursor_pos();
                self.body_text.insert(pos, c);
                self.body_cursor_col += c.len_utf8();
            }
            KeyCode::Enter => {
                let pos = self.body_cursor_pos();
                self.body_text.insert(pos, '\n');
                self.body_cursor_line += 1;
                self.body_cursor_col = 0;
            }
            KeyCode::Backspace => {
                let pos = self.body_cursor_pos();
                if pos > 0 {
                    if self.body_cursor_col > 0 {
                        let prev_char_len = self.body_text[..pos]
                            .chars()
                            .next_back()
                            .map(|c| c.len_utf8())
                            .unwrap_or(1);
                        self.body_text.drain(pos - prev_char_len..pos);
                        self.body_cursor_col -= prev_char_len;
                    } else if self.body_cursor_line > 0 {
                        // Join with previous line
                        let prev_line_len = lines
                            .get(self.body_cursor_line - 1)
                            .map(|l| l.len())
                            .unwrap_or(0);
                        self.body_text.remove(pos - 1); // remove newline
                        self.body_cursor_line -= 1;
                        self.body_cursor_col = prev_line_len;
                    }
                }
            }
            KeyCode::Up => {
                if self.body_cursor_line > 0 {
                    self.body_cursor_line -= 1;
                    let new_lines: Vec<&str> = self.body_text.lines().collect();
                    let line_len = new_lines
                        .get(self.body_cursor_line)
                        .map(|l| l.len())
                        .unwrap_or(0);
                    self.body_cursor_col = self.body_cursor_col.min(line_len);
                }
            }
            KeyCode::Down => {
                if self.body_cursor_line < line_count - 1 {
                    self.body_cursor_line += 1;
                    let new_lines: Vec<&str> = self.body_text.lines().collect();
                    let line_len = new_lines
                        .get(self.body_cursor_line)
                        .map(|l| l.len())
                        .unwrap_or(0);
                    self.body_cursor_col = self.body_cursor_col.min(line_len);
                }
            }
            KeyCode::Left => {
                if self.body_cursor_col > 0 {
                    self.body_cursor_col -= 1;
                }
            }
            KeyCode::Right => {
                let current_line_len = lines
                    .get(self.body_cursor_line)
                    .map(|l| l.len())
                    .unwrap_or(0);
                if self.body_cursor_col < current_line_len {
                    self.body_cursor_col += 1;
                }
            }
            _ => {}
        }
    }

    fn body_cursor_pos(&self) -> usize {
        let mut pos = 0;
        for (i, line) in self.body_text.lines().enumerate() {
            if i == self.body_cursor_line {
                return pos + self.body_cursor_col.min(line.len());
            }
            pos += line.len() + 1; // +1 for newline
        }
        // If cursor is past the last line
        self.body_text.len()
    }

    fn enter_insert_mode(&mut self) {
        self.mode = EditorMode::Insert;
        if matches!(self.active_tab, EditorTab::Headers | EditorTab::Params) {
            self.load_grid_cell_into_edit();
        }
    }

    fn load_grid_cell_into_edit(&mut self) {
        let (items, row, col) = match self.active_tab {
            EditorTab::Headers => (&self.headers, self.header_row, self.header_col),
            EditorTab::Params => (&self.params, self.param_row, self.param_col),
            _ => return,
        };

        if let Some(item) = items.get(row) {
            self.edit_buffer = if col == 0 {
                item.0.clone()
            } else {
                item.1.clone()
            };
            self.edit_cursor = self.edit_buffer.len();
        }
    }

    fn commit_edit(&mut self) {
        let (items, row, col) = match self.active_tab {
            EditorTab::Headers => (&mut self.headers, self.header_row, self.header_col),
            EditorTab::Params => (&mut self.params, self.param_row, self.param_col),
            _ => return,
        };

        if let Some(item) = items.get_mut(row) {
            if col == 0 {
                item.0 = self.edit_buffer.clone();
            } else {
                item.1 = self.edit_buffer.clone();
            }
            self.dirty = true;
        }
    }

    fn grid_move_down(&mut self) {
        match self.active_tab {
            EditorTab::Headers if !self.headers.is_empty() => {
                self.header_row = (self.header_row + 1) % self.headers.len();
            }
            EditorTab::Params if !self.params.is_empty() => {
                self.param_row = (self.param_row + 1) % self.params.len();
            }
            _ => {}
        }
    }

    fn grid_move_up(&mut self) {
        match self.active_tab {
            EditorTab::Headers if !self.headers.is_empty() => {
                self.header_row = self
                    .header_row
                    .checked_sub(1)
                    .unwrap_or(self.headers.len() - 1);
            }
            EditorTab::Params if !self.params.is_empty() => {
                self.param_row = self
                    .param_row
                    .checked_sub(1)
                    .unwrap_or(self.params.len() - 1);
            }
            _ => {}
        }
    }

    fn grid_add_row(&mut self) {
        match self.active_tab {
            EditorTab::Headers => {
                self.headers.push((String::new(), String::new(), true));
                self.header_row = self.headers.len() - 1;
                self.dirty = true;
            }
            EditorTab::Params => {
                self.params.push((String::new(), String::new(), true));
                self.param_row = self.params.len() - 1;
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn grid_delete_row(&mut self) {
        match self.active_tab {
            EditorTab::Headers if !self.headers.is_empty() => {
                self.headers.remove(self.header_row);
                if self.header_row >= self.headers.len() && !self.headers.is_empty() {
                    self.header_row = self.headers.len() - 1;
                }
                self.dirty = true;
            }
            EditorTab::Params if !self.params.is_empty() => {
                self.params.remove(self.param_row);
                if self.param_row >= self.params.len() && !self.params.is_empty() {
                    self.param_row = self.params.len() - 1;
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn grid_toggle_enabled(&mut self) {
        match self.active_tab {
            EditorTab::Headers => {
                if let Some(row) = self.headers.get_mut(self.header_row) {
                    row.2 = !row.2;
                    self.dirty = true;
                }
            }
            EditorTab::Params => {
                if let Some(row) = self.params.get_mut(self.param_row) {
                    row.2 = !row.2;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn render_url_tab(&self, frame: &mut Frame, area: Rect) {
        let method = HttpMethod::ALL[self.method_index];
        let method_color = match method {
            HttpMethod::Get => Color::Green,
            HttpMethod::Post => Color::Yellow,
            HttpMethod::Put => Color::Blue,
            HttpMethod::Patch => Color::Magenta,
            HttpMethod::Delete => Color::Red,
            _ => Color::White,
        };

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(10), Constraint::Min(1)])
            .split(area);

        let method_widget = Paragraph::new(format!(" {method} ")).style(
            Style::default()
                .fg(Color::Black)
                .bg(method_color)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(method_widget, chunks[0]);

        let border_color = if self.mode == EditorMode::Insert {
            Color::Yellow
        } else {
            Color::DarkGray
        };

        // Show URL with cursor indicator
        let display_text = if self.mode == EditorMode::Insert {
            // Show cursor position with a block character
            let mut s = self.url_text.clone();
            if self.url_cursor <= s.len() {
                s.insert(self.url_cursor, '│');
            }
            s
        } else {
            self.url_text.clone()
        };

        let url_widget = Paragraph::new(display_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        );
        frame.render_widget(url_widget, chunks[1]);
    }

    fn render_kv_tab(
        &self,
        frame: &mut Frame,
        area: Rect,
        items: &[(String, String, bool)],
        selected_row: usize,
        selected_col: usize,
        is_editing: bool,
    ) {
        if items.is_empty() {
            let hint = Paragraph::new("Press 'a' to add a row")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(hint, area);
            return;
        }

        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .map(|(i, (k, v, enabled))| {
                let is_selected = i == selected_row;
                let enabled_marker = if *enabled { "[x]" } else { "[ ]" };

                let (display_key, display_val) = if is_selected && is_editing {
                    let buf = &self.edit_buffer;
                    let display = format!("{buf}│");
                    if selected_col == 0 {
                        (display, v.clone())
                    } else {
                        (k.clone(), display)
                    }
                } else {
                    (k.clone(), v.clone())
                };

                let key_style = if is_selected && selected_col == 0 {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if !enabled {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };

                let val_style = if is_selected && selected_col == 1 {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if !enabled {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };

                let enabled_style = if is_selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                Row::new(vec![
                    ratatui::text::Text::styled(enabled_marker, enabled_style),
                    ratatui::text::Text::styled(display_key, key_style),
                    ratatui::text::Text::styled(display_val, val_style),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(5),
                Constraint::Percentage(40),
                Constraint::Percentage(55),
            ],
        )
        .header(
            Row::new(vec!["", "Key", "Value"]).style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Cyan),
            ),
        );

        frame.render_widget(table, area);
    }
}

enum InputTarget {
    Url,
    EditBuffer,
}

impl Component for RequestEditorPane {
    fn handle_key(&mut self, key: KeyEvent) -> EventResult {
        if self.document.is_none() {
            return EventResult::Ignored;
        }

        match self.mode {
            EditorMode::Normal => self.handle_normal_key(key),
            EditorMode::Insert => self.handle_insert_key(key),
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border_color = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let title = if self.document.is_some() {
            if self.dirty {
                " Request [modified] "
            } else {
                " Request "
            }
        } else {
            " Request "
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.document.is_none() {
            let msg = Paragraph::new("Select a request to edit")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(msg, inner);
            return;
        }

        // Layout: tab bar + content
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);

        // Tab bar
        let tabs: Vec<Span> = EditorTab::ALL
            .iter()
            .flat_map(|tab| {
                let label = match *tab {
                    EditorTab::Headers => {
                        format!("{}({})", tab.label(), self.headers.len())
                    }
                    EditorTab::Params => {
                        format!("{}({})", tab.label(), self.params.len())
                    }
                    _ => tab.label().to_string(),
                };

                let style = if *tab == self.active_tab {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                vec![Span::styled(format!(" {label} "), style), Span::raw("|")]
            })
            .collect();

        frame.render_widget(Paragraph::new(Line::from(tabs)), chunks[0]);

        // Content
        let is_inserting = self.mode == EditorMode::Insert;
        match self.active_tab {
            EditorTab::Url => self.render_url_tab(frame, chunks[1]),
            EditorTab::Headers => {
                self.render_kv_tab(
                    frame,
                    chunks[1],
                    &self.headers,
                    self.header_row,
                    self.header_col,
                    is_inserting,
                );
            }
            EditorTab::Params => {
                self.render_kv_tab(
                    frame,
                    chunks[1],
                    &self.params,
                    self.param_row,
                    self.param_col,
                    is_inserting,
                );
            }
            EditorTab::Body => {
                let body_display = if is_inserting {
                    // Simple body display with line numbers
                    let lines: Vec<&str> = self.body_text.lines().collect();
                    let mut display_lines = Vec::new();
                    for (i, line) in lines.iter().enumerate() {
                        if i == self.body_cursor_line {
                            let mut l = line.to_string();
                            let col = self.body_cursor_col.min(l.len());
                            l.insert(col, '│');
                            display_lines.push(l);
                        } else {
                            display_lines.push(line.to_string());
                        }
                    }
                    if lines.is_empty() {
                        display_lines.push("│".to_string());
                    }
                    display_lines.join("\n")
                } else if self.body_text.is_empty() {
                    "Press 'i' to edit body".to_string()
                } else {
                    self.body_text.clone()
                };

                let style = if self.body_text.is_empty() && !is_inserting {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };

                let body_widget = Paragraph::new(body_display).style(style);
                frame.render_widget(body_widget, chunks[1]);
            }
        }
    }

    fn is_editing(&self) -> bool {
        self.mode == EditorMode::Insert
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    use crate::core::models::HttpMethod;

    fn sample_doc() -> RequestDocument {
        RequestDocument {
            name: "Test Request".into(),
            method: HttpMethod::Post,
            url: "https://api.example.com/users".into(),
            headers: vec![KeyValueField {
                key: "Content-Type".into(),
                value: "application/json".into(),
                enabled: true,
            }],
            params: vec![KeyValueField {
                key: "page".into(),
                value: "1".into(),
                enabled: true,
            }],
            body: Some("{\"name\": \"test\"}".into()),
            assertions: vec![],
            file_path: Some("/tmp/test.hurl.yml".into()),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn load_document_populates_fields() {
        let mut editor = RequestEditorPane::default();
        editor.load_document(sample_doc());

        assert!(editor.has_document());
        assert!(!editor.is_dirty());
        assert_eq!(editor.method_index, 1); // POST
        assert_eq!(editor.headers.len(), 1);
        assert_eq!(editor.params.len(), 1);
    }

    #[test]
    fn to_document_round_trips() {
        let mut editor = RequestEditorPane::default();
        let original = sample_doc();
        editor.load_document(original.clone());

        let exported = editor.to_document().unwrap();
        assert_eq!(exported.name, original.name);
        assert_eq!(exported.method, original.method);
        assert_eq!(exported.url, original.url);
        assert_eq!(exported.headers, original.headers);
        assert_eq!(exported.params, original.params);
        assert_eq!(exported.file_path, original.file_path);
    }

    #[test]
    fn tab_switching_works() {
        let mut editor = RequestEditorPane::default();
        editor.load_document(sample_doc());

        assert_eq!(editor.active_tab, EditorTab::Url);
        editor.handle_key(key(KeyCode::Char('2')));
        assert_eq!(editor.active_tab, EditorTab::Headers);
        editor.handle_key(key(KeyCode::Char('4')));
        assert_eq!(editor.active_tab, EditorTab::Body);
    }

    #[test]
    fn insert_mode_toggle() {
        let mut editor = RequestEditorPane::default();
        editor.load_document(sample_doc());

        assert!(!editor.is_editing());
        editor.handle_key(key(KeyCode::Char('i')));
        assert!(editor.is_editing());
        editor.handle_key(key(KeyCode::Esc));
        assert!(!editor.is_editing());
    }

    #[test]
    fn dirty_flag_set_on_method_change() {
        let mut editor = RequestEditorPane::default();
        editor.load_document(sample_doc());
        assert!(!editor.is_dirty());

        editor.handle_key(key(KeyCode::Char('>')));
        assert!(editor.is_dirty());
    }

    #[test]
    fn add_and_delete_header_row() {
        let mut editor = RequestEditorPane::default();
        editor.load_document(sample_doc());
        editor.active_tab = EditorTab::Headers;

        let initial_count = editor.headers.len();
        editor.handle_key(key(KeyCode::Char('a')));
        assert_eq!(editor.headers.len(), initial_count + 1);

        editor.handle_key(key(KeyCode::Char('d')));
        assert_eq!(editor.headers.len(), initial_count);
    }

    #[test]
    fn no_document_ignores_keys() {
        let mut editor = RequestEditorPane::default();
        let result = editor.handle_key(key(KeyCode::Char('i')));
        assert!(matches!(result, EventResult::Ignored));
    }

    #[test]
    fn url_editing_in_insert_mode() {
        let mut editor = RequestEditorPane::default();
        editor.load_document(sample_doc());

        let original_url = editor.url_text.clone();
        editor.handle_key(key(KeyCode::Char('i'))); // enter insert
        editor.handle_key(key(KeyCode::End));
        editor.handle_key(key(KeyCode::Char('?')));
        editor.handle_key(key(KeyCode::Char('a')));
        editor.handle_key(key(KeyCode::Esc)); // exit insert

        assert_eq!(editor.url_text, format!("{original_url}?a"));
        assert!(editor.is_dirty());
    }

    #[test]
    fn to_document_includes_pending_header_edits_while_still_in_insert_mode() {
        let mut editor = RequestEditorPane::default();
        editor.load_document(sample_doc());
        editor.active_tab = EditorTab::Headers;
        editor.header_row = 0;
        editor.header_col = 1;

        editor.handle_key(key(KeyCode::Char('i')));
        editor.handle_key(key(KeyCode::End));
        editor.handle_key(key(KeyCode::Char(';')));
        editor.handle_key(key(KeyCode::Char('c')));
        editor.handle_key(key(KeyCode::Char('h')));
        editor.handle_key(key(KeyCode::Char('a')));
        editor.handle_key(key(KeyCode::Char('r')));
        editor.handle_key(key(KeyCode::Char('s')));
        editor.handle_key(key(KeyCode::Char('e')));
        editor.handle_key(key(KeyCode::Char('t')));

        let exported = editor.to_document().unwrap();
        assert_eq!(exported.headers[0].value, "application/json;charset");
    }
}
