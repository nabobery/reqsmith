use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table, Wrap},
};

use crate::action::Action;
use crate::components::json_tree::JsonTreeState;
use crate::core::diffing;
use crate::core::execution;
use crate::core::models::{AssertionReport, DiffArtifact, DiffLine, HeaderDiff, ResponseArtifact};
use crate::core::redaction;

use super::{Component, EventResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseTab {
    Body,
    Headers,
    Summary,
    Assertions,
    Diff,
}

impl ResponseTab {
    const ALL: [ResponseTab; 5] = [
        ResponseTab::Body,
        ResponseTab::Headers,
        ResponseTab::Summary,
        ResponseTab::Assertions,
        ResponseTab::Diff,
    ];

    fn label(self) -> &'static str {
        match self {
            ResponseTab::Body => "Body",
            ResponseTab::Headers => "Headers",
            ResponseTab::Summary => "Summary",
            ResponseTab::Assertions => "Assertions",
            ResponseTab::Diff => "Diff",
        }
    }
}

#[derive(Debug)]
enum ViewerState {
    Empty,
    Loading,
    Success {
        response: Box<ResponseArtifact>,
        assertions: Box<AssertionReport>,
        diff: Option<Box<DiffArtifact>>,
        /// Headers with sensitive values already redacted, computed once when
        /// the response is set — `render` runs every frame and must not
        /// redact per frame. Living inside `Success` (rather than a
        /// standalone field) means it can never go stale: there is no way to
        /// be in a non-`Success` state while still holding a previous
        /// response's headers.
        display_headers: Vec<(String, String)>,
    },
    Error(String),
}

pub struct ResponseViewerPane {
    state: ViewerState,
    active_tab: ResponseTab,
    scroll_offset: u16,
    json_tree_mode: bool,
    json_tree: Option<JsonTreeState>,
}

impl Default for ResponseViewerPane {
    fn default() -> Self {
        Self {
            state: ViewerState::Empty,
            active_tab: ResponseTab::Body,
            scroll_offset: 0,
            json_tree_mode: false,
            json_tree: None,
        }
    }
}

impl ResponseViewerPane {
    pub fn set_loading(&mut self) {
        self.state = ViewerState::Loading;
        self.scroll_offset = 0;
    }

    /// Try to build a `JsonTreeState` from the response body text.
    fn try_build_json_tree(artifact: &ResponseArtifact) -> Option<JsonTreeState> {
        let body = artifact.body_text.as_deref()?;
        if artifact.is_binary {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(body).ok()?;
        Some(JsonTreeState::from_value(&value))
    }

    #[cfg(test)]
    pub fn set_response(&mut self, artifact: ResponseArtifact, assertions: AssertionReport) {
        self.set_response_with_diff(artifact, assertions, None);
    }

    pub fn set_response_with_diff(
        &mut self,
        artifact: ResponseArtifact,
        assertions: AssertionReport,
        previous: Option<&ResponseArtifact>,
    ) {
        self.json_tree = Self::try_build_json_tree(&artifact);
        self.json_tree_mode = false;
        let display_headers = redaction::redact_headers(&artifact.headers);
        let diff = previous
            .map(|prev| diffing::diff_responses(prev, &artifact, "previous", "current"))
            .map(Box::new);
        self.state = ViewerState::Success {
            response: Box::new(artifact),
            assertions: Box::new(assertions),
            diff,
            display_headers,
        };
        self.scroll_offset = 0;
        self.active_tab = ResponseTab::Body;
    }

    /// Borrow the current response (if any), e.g. to use as the previous
    /// response when diffing the next one.
    pub fn current_response(&self) -> Option<&ResponseArtifact> {
        match &self.state {
            ViewerState::Success { response, .. } => Some(response),
            _ => None,
        }
    }

    pub fn binary_body_bytes(&self) -> Option<&[u8]> {
        match &self.state {
            ViewerState::Success { response, .. } if response.is_binary => {
                response.body_bytes.as_deref()
            }
            _ => None,
        }
    }

    pub fn set_error(&mut self, message: String) {
        self.state = ViewerState::Error(message);
        self.scroll_offset = 0;
    }

    pub fn set_cancelled(&mut self) {
        self.state = ViewerState::Error("Request cancelled".into());
        self.scroll_offset = 0;
    }

    #[cfg(test)]
    pub fn is_loading(&self) -> bool {
        matches!(self.state, ViewerState::Loading)
    }

    fn status_color(code: u16) -> Color {
        match code {
            200..=299 => Color::Green,
            300..=399 => Color::Yellow,
            400..=499 => Color::Red,
            500..=599 => Color::Magenta,
            _ => Color::White,
        }
    }

    fn render_status_line(frame: &mut Frame, area: Rect, response: &ResponseArtifact) {
        let status_color = Self::status_color(response.status_code);
        let mut spans = vec![
            Span::styled(
                format!(" {} ", response.status_code),
                Style::default()
                    .fg(Color::Black)
                    .bg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{}ms", response.duration_ms),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(
                response.content_type.as_deref().unwrap_or("unknown"),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                response
                    .content_length
                    .map(|l| format!("{l}B"))
                    .unwrap_or_default(),
                Style::default().fg(Color::DarkGray),
            ),
        ];
        if response.truncated {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("truncated at {}", execution::max_body_size_label()),
                Style::default().fg(Color::Yellow),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_tab_bar(&self, frame: &mut Frame, area: Rect) {
        let tabs: Vec<Span> = ResponseTab::ALL
            .iter()
            .flat_map(|tab| {
                let style = if *tab == self.active_tab {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                vec![
                    Span::styled(format!(" {} ", tab.label()), style),
                    Span::raw("|"),
                ]
            })
            .collect();
        frame.render_widget(Paragraph::new(Line::from(tabs)), area);
    }

    fn render_body(&self, frame: &mut Frame, area: Rect, response: &ResponseArtifact) {
        if self.json_tree_mode {
            if let Some(ref tree) = self.json_tree {
                let lines =
                    tree.render_lines(area.width as usize, area.height as usize, tree.selected());
                frame.render_widget(Paragraph::new(lines), area);
            }
            return;
        }

        let body = Self::body_display_text(response);
        let paragraph = Paragraph::new(body.as_ref())
            .scroll((self.scroll_offset, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    /// The body text this pane shows. Core leaves a binary body as `None`, so
    /// the placeholder — including the save hint — is written here.
    fn body_display_text(response: &ResponseArtifact) -> Cow<'_, str> {
        if let Some(text) = response.body_text.as_deref() {
            return Cow::Borrowed(text);
        }
        if response.is_binary {
            let len = response.body_bytes.as_ref().map_or(0, Vec::len);
            return Cow::Owned(format!(
                "[Binary response: {len} bytes. Press 'w' to save to disk.]"
            ));
        }
        Cow::Borrowed("<empty body>")
    }

    fn render_headers(&self, frame: &mut Frame, area: Rect, headers: &[(String, String)]) {
        // `headers` was redacted when the response was set.
        let visible_rows = area.height.saturating_sub(1).max(1) as usize;
        let max_offset = headers.len().saturating_sub(visible_rows);
        let start = usize::min(self.scroll_offset as usize, max_offset);
        let end = usize::min(start + visible_rows, headers.len());

        let rows: Vec<Row> = headers
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|(k, v)| Row::new(vec![k.as_str(), v.as_str()]))
            .collect();

        let table = Table::new(
            rows,
            [Constraint::Percentage(35), Constraint::Percentage(65)],
        )
        .header(
            Row::new(vec!["Header", "Value"]).style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Cyan),
            ),
        );
        frame.render_widget(table, area);
    }

    fn render_summary(frame: &mut Frame, area: Rect, response: &ResponseArtifact) {
        let summary = vec![
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{}", response.status_code),
                    Style::default().fg(Self::status_color(response.status_code)),
                ),
            ]),
            Line::from(vec![
                Span::styled("HTTP Version: ", Style::default().fg(Color::Cyan)),
                Span::raw(&response.http_version),
            ]),
            Line::from(vec![
                Span::styled("Duration: ", Style::default().fg(Color::Cyan)),
                Span::raw(format!("{}ms", response.duration_ms)),
            ]),
            Line::from(vec![
                Span::styled("Content-Type: ", Style::default().fg(Color::Cyan)),
                Span::raw(response.content_type.as_deref().unwrap_or("N/A")),
            ]),
            Line::from(vec![
                Span::styled("Content-Length: ", Style::default().fg(Color::Cyan)),
                Span::raw(
                    response
                        .content_length
                        .map(|l| format!("{l} bytes"))
                        .unwrap_or_else(|| "N/A".into()),
                ),
            ]),
            Line::from(vec![
                Span::styled("Headers: ", Style::default().fg(Color::Cyan)),
                Span::raw(format!("{}", response.headers.len())),
            ]),
        ];
        frame.render_widget(Paragraph::new(summary), area);
    }

    fn render_diff(&self, frame: &mut Frame, area: Rect, diff: Option<&DiffArtifact>) {
        let Some(d) = diff else {
            let msg = Paragraph::new("No previous response to diff against")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(msg, area);
            return;
        };

        let mut lines: Vec<Line> = Vec::new();

        if let Some((old, new)) = d.status_diff {
            lines.push(Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Cyan)),
                Span::styled(format!("{old}"), Style::default().fg(Color::Red)),
                Span::raw(" -> "),
                Span::styled(format!("{new}"), Style::default().fg(Color::Green)),
            ]));
            lines.push(Line::raw(""));
        }

        if !d.header_diffs.is_empty() {
            lines.push(Line::styled("Headers:", Style::default().fg(Color::Cyan)));
            for hd in &d.header_diffs {
                let color = match hd {
                    HeaderDiff::Added(..) => Color::Green,
                    HeaderDiff::Removed(..) => Color::Red,
                    HeaderDiff::Changed { .. } => Color::Yellow,
                };
                let (key, value) = hd.display_parts();
                lines.push(Line::styled(
                    format!("  {} {key}: {value}", hd.sigil()),
                    Style::default().fg(color),
                ));
            }
            lines.push(Line::raw(""));
        }

        let has_body_changes = d
            .body_diff
            .iter()
            .any(|l| matches!(l, DiffLine::Insert(_) | DiffLine::Delete(_)));
        if has_body_changes {
            lines.push(Line::styled("Body:", Style::default().fg(Color::Cyan)));
            for dl in &d.body_diff {
                let color = match dl {
                    DiffLine::Equal(_) => Color::DarkGray,
                    DiffLine::Insert(_) => Color::Green,
                    DiffLine::Delete(_) => Color::Red,
                };
                lines.push(Line::styled(
                    format!(" {}{}", dl.sigil(), dl.text()),
                    Style::default().fg(color),
                ));
            }
        }

        if lines.is_empty() {
            lines.push(Line::styled(
                "No differences found",
                Style::default().fg(Color::DarkGray),
            ));
        }

        let paragraph = Paragraph::new(lines)
            .scroll((self.scroll_offset, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_assertions(&self, frame: &mut Frame, area: Rect, assertions: &AssertionReport) {
        if assertions.is_empty() {
            let msg = Paragraph::new("No assertions defined for this request")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(msg, area);
            return;
        }

        let header = Line::from(vec![
            Span::styled(
                format!("  {} passed", assertions.pass_count()),
                Style::default().fg(Color::Green),
            ),
            Span::raw(", "),
            Span::styled(
                format!("{} failed", assertions.fail_count()),
                Style::default().fg(if assertions.fail_count() > 0 {
                    Color::Red
                } else {
                    Color::DarkGray
                }),
            ),
        ]);

        let mut all_lines = vec![header, Line::raw("")];
        all_lines.extend(assertions.results.iter().map(|r| {
            let (icon, color) = if r.passed {
                ("  PASS ", Color::Green)
            } else {
                ("  FAIL ", Color::Red)
            };
            let mut spans = vec![
                Span::styled(
                    icon,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{}", r.assertion)),
            ];
            if let Some(actual) = &r.actual_value {
                spans.push(Span::styled(
                    format!(" (actual: {actual})"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            Line::from(spans)
        }));

        let paragraph = Paragraph::new(all_lines)
            .scroll((self.scroll_offset, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }
}

impl Component for ResponseViewerPane {
    fn handle_key(&mut self, key: KeyEvent) -> EventResult {
        // When in tree mode on the Body tab, route navigation keys to the tree state.
        if self.json_tree_mode
            && self.active_tab == ResponseTab::Body
            && let Some(ref mut tree) = self.json_tree
        {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    tree.move_down();
                    return EventResult::Consumed;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    tree.move_up();
                    return EventResult::Consumed;
                }
                KeyCode::Char('h') | KeyCode::Left => {
                    tree.collapse_selected();
                    return EventResult::Consumed;
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    tree.expand_selected();
                    return EventResult::Consumed;
                }
                KeyCode::Enter => {
                    tree.toggle_selected();
                    return EventResult::Consumed;
                }
                KeyCode::Char('t') => {
                    self.json_tree_mode = false;
                    return EventResult::Consumed;
                }
                _ => {} // Fall through to default handling below
            }
        }

        match key.code {
            KeyCode::Char('t') => {
                // Toggle tree mode when on Body tab and JSON tree is available.
                if self.active_tab == ResponseTab::Body && self.json_tree.is_some() {
                    self.json_tree_mode = !self.json_tree_mode;
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            KeyCode::Char('w') => {
                if self.binary_body_bytes().is_some() {
                    EventResult::Action(Action::SaveResponseBody)
                } else {
                    EventResult::Ignored
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                EventResult::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                EventResult::Consumed
            }
            KeyCode::Char('g') => {
                self.scroll_offset = 0;
                EventResult::Consumed
            }
            KeyCode::Char('G') => {
                self.scroll_offset = u16::MAX; // Will be clamped by content
                EventResult::Consumed
            }
            KeyCode::Char('1') => {
                self.active_tab = ResponseTab::Body;
                self.scroll_offset = 0;
                EventResult::Consumed
            }
            KeyCode::Char('2') => {
                self.active_tab = ResponseTab::Headers;
                self.scroll_offset = 0;
                EventResult::Consumed
            }
            KeyCode::Char('3') => {
                self.active_tab = ResponseTab::Summary;
                self.scroll_offset = 0;
                EventResult::Consumed
            }
            KeyCode::Char('4') => {
                self.active_tab = ResponseTab::Assertions;
                self.scroll_offset = 0;
                EventResult::Consumed
            }
            KeyCode::Char('5') | KeyCode::Char('d') => {
                self.active_tab = ResponseTab::Diff;
                self.scroll_offset = 0;
                EventResult::Consumed
            }
            KeyCode::Char('h') | KeyCode::Left => {
                let idx = ResponseTab::ALL
                    .iter()
                    .position(|&t| t == self.active_tab)
                    .unwrap_or(0);
                if idx > 0 {
                    self.active_tab = ResponseTab::ALL[idx - 1];
                    self.scroll_offset = 0;
                }
                EventResult::Consumed
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let idx = ResponseTab::ALL
                    .iter()
                    .position(|&t| t == self.active_tab)
                    .unwrap_or(0);
                if idx < ResponseTab::ALL.len() - 1 {
                    self.active_tab = ResponseTab::ALL[idx + 1];
                    self.scroll_offset = 0;
                }
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border_color = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let block = Block::default()
            .title(" Response ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let (response, assertions, diff, display_headers) = match &self.state {
            ViewerState::Empty => {
                let msg = Paragraph::new("Send a request to see the response")
                    .style(Style::default().fg(Color::DarkGray));
                frame.render_widget(msg, inner);
                return;
            }
            ViewerState::Loading => {
                let msg =
                    Paragraph::new("Sending request...").style(Style::default().fg(Color::Yellow));
                frame.render_widget(msg, inner);
                return;
            }
            ViewerState::Error(message) => {
                let msg = Paragraph::new(message.as_str())
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: false });
                frame.render_widget(msg, inner);
                return;
            }
            ViewerState::Success {
                response,
                assertions,
                diff,
                display_headers,
            } => (response, assertions, diff, display_headers),
        };

        // Status line + tab bar + content.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        Self::render_status_line(frame, chunks[0], response);
        self.render_tab_bar(frame, chunks[1]);

        let content = chunks[2];
        match self.active_tab {
            ResponseTab::Body => self.render_body(frame, content, response),
            ResponseTab::Headers => self.render_headers(frame, content, display_headers),
            ResponseTab::Summary => Self::render_summary(frame, content, response),
            ResponseTab::Assertions => self.render_assertions(frame, content, assertions),
            ResponseTab::Diff => self.render_diff(frame, content, diff.as_deref()),
        }
    }
}

#[cfg(test)]
#[path = "response_viewer_tests.rs"]
mod tests;
