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
use crate::core::models::{AssertionReport, DiffArtifact, DiffLine, ResponseArtifact};
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

    #[allow(dead_code)]
    pub fn set_response(&mut self, artifact: ResponseArtifact, assertions: AssertionReport) {
        self.json_tree = Self::try_build_json_tree(&artifact);
        self.json_tree_mode = false;
        self.state = ViewerState::Success {
            response: Box::new(artifact),
            assertions: Box::new(assertions),
            diff: None,
        };
        self.scroll_offset = 0;
        self.active_tab = ResponseTab::Body;
    }

    pub fn set_response_with_diff(
        &mut self,
        artifact: ResponseArtifact,
        assertions: AssertionReport,
        previous: Option<&ResponseArtifact>,
    ) {
        self.json_tree = Self::try_build_json_tree(&artifact);
        self.json_tree_mode = false;
        let diff = previous
            .map(|prev| diffing::diff_responses(prev, &artifact, "previous", "current"))
            .map(Box::new);
        self.state = ViewerState::Success {
            response: Box::new(artifact),
            assertions: Box::new(assertions),
            diff,
        };
        self.scroll_offset = 0;
        self.active_tab = ResponseTab::Body;
    }

    /// Extract the current response (if any) for use as previous response in diffing.
    pub fn take_response(&self) -> Option<ResponseArtifact> {
        match &self.state {
            ViewerState::Success { response, .. } => Some(response.as_ref().clone()),
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

    #[allow(dead_code)]
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

        match &self.state {
            ViewerState::Empty => {
                let msg = Paragraph::new("Send a request to see the response")
                    .style(Style::default().fg(Color::DarkGray));
                frame.render_widget(msg, inner);
            }

            ViewerState::Loading => {
                let msg =
                    Paragraph::new("Sending request...").style(Style::default().fg(Color::Yellow));
                frame.render_widget(msg, inner);
            }

            ViewerState::Error(message) => {
                let msg = Paragraph::new(message.as_str())
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: false });
                frame.render_widget(msg, inner);
            }

            ViewerState::Success {
                response,
                assertions,
                diff,
            } => {
                // Layout: status line + tab bar + content
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1), // Status line
                        Constraint::Length(1), // Tab bar
                        Constraint::Min(1),    // Content
                    ])
                    .split(inner);

                // Status line
                let status_color = Self::status_color(response.status_code);
                let status_line = Line::from(vec![
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
                ]);
                frame.render_widget(Paragraph::new(status_line), chunks[0]);

                // Tab bar
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
                frame.render_widget(Paragraph::new(Line::from(tabs)), chunks[1]);

                // Content
                match self.active_tab {
                    ResponseTab::Body => {
                        if self.json_tree_mode {
                            if let Some(ref tree) = self.json_tree {
                                let width = chunks[2].width as usize;
                                let height = chunks[2].height as usize;
                                let lines = tree.render_lines(width, height, tree.selected());
                                let paragraph = Paragraph::new(lines);
                                frame.render_widget(paragraph, chunks[2]);
                            }
                        } else {
                            let body = response.body_text.as_deref().unwrap_or("<empty body>");
                            let paragraph = Paragraph::new(body)
                                .scroll((self.scroll_offset, 0))
                                .wrap(Wrap { trim: false });
                            frame.render_widget(paragraph, chunks[2]);
                        }
                    }
                    ResponseTab::Headers => {
                        // Redact sensitive values (e.g. Set-Cookie) before display.
                        let headers = redaction::redact_headers(&response.headers);
                        let visible_rows = chunks[2].height.saturating_sub(1).max(1) as usize;
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
                        frame.render_widget(table, chunks[2]);
                    }
                    ResponseTab::Summary => {
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
                        frame.render_widget(Paragraph::new(summary), chunks[2]);
                    }
                    ResponseTab::Diff => match diff {
                        None => {
                            let msg = Paragraph::new("No previous response to diff against")
                                .style(Style::default().fg(Color::DarkGray));
                            frame.render_widget(msg, chunks[2]);
                        }
                        Some(d) => {
                            let mut lines: Vec<Line> = Vec::new();

                            if let Some((old, new)) = d.status_diff {
                                lines.push(Line::from(vec![
                                    Span::styled("Status: ", Style::default().fg(Color::Cyan)),
                                    Span::styled(format!("{old}"), Style::default().fg(Color::Red)),
                                    Span::raw(" -> "),
                                    Span::styled(
                                        format!("{new}"),
                                        Style::default().fg(Color::Green),
                                    ),
                                ]));
                                lines.push(Line::raw(""));
                            }

                            if !d.header_diffs.is_empty() {
                                lines.push(Line::styled(
                                    "Headers:",
                                    Style::default().fg(Color::Cyan),
                                ));
                                for hd in &d.header_diffs {
                                    let line = match hd {
                                        crate::core::models::HeaderDiff::Added(k, v) => {
                                            Line::styled(
                                                format!("  + {k}: {v}"),
                                                Style::default().fg(Color::Green),
                                            )
                                        }
                                        crate::core::models::HeaderDiff::Removed(k, v) => {
                                            Line::styled(
                                                format!("  - {k}: {v}"),
                                                Style::default().fg(Color::Red),
                                            )
                                        }
                                        crate::core::models::HeaderDiff::Changed {
                                            key,
                                            old,
                                            new,
                                        } => Line::styled(
                                            format!("  ~ {key}: {old} -> {new}"),
                                            Style::default().fg(Color::Yellow),
                                        ),
                                    };
                                    lines.push(line);
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
                                    let line = match dl {
                                        DiffLine::Equal(s) => Line::styled(
                                            format!("  {s}"),
                                            Style::default().fg(Color::DarkGray),
                                        ),
                                        DiffLine::Insert(s) => Line::styled(
                                            format!(" +{s}"),
                                            Style::default().fg(Color::Green),
                                        ),
                                        DiffLine::Delete(s) => Line::styled(
                                            format!(" -{s}"),
                                            Style::default().fg(Color::Red),
                                        ),
                                    };
                                    lines.push(line);
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
                            frame.render_widget(paragraph, chunks[2]);
                        }
                    },
                    ResponseTab::Assertions => {
                        if assertions.is_empty() {
                            let msg = Paragraph::new("No assertions defined for this request")
                                .style(Style::default().fg(Color::DarkGray));
                            frame.render_widget(msg, chunks[2]);
                        } else {
                            let lines: Vec<Line> = assertions
                                .results
                                .iter()
                                .map(|r| {
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
                                })
                                .collect();

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
                            all_lines.extend(lines);

                            let paragraph = Paragraph::new(all_lines)
                                .scroll((self.scroll_offset, 0))
                                .wrap(Wrap { trim: false });
                            frame.render_widget(paragraph, chunks[2]);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn sample_response() -> ResponseArtifact {
        ResponseArtifact {
            status_code: 200,
            http_version: "HTTP/1.1".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            content_type: Some("application/json".into()),
            content_length: Some(42),
            duration_ms: 150,
            body_text: Some("{\"ok\": true}".into()),
            body_bytes: None,
            is_binary: false,
        }
    }

    fn set_sample(viewer: &mut ResponseViewerPane) {
        viewer.set_response(sample_response(), AssertionReport::default());
    }

    #[test]
    fn starts_in_empty_state() {
        let viewer = ResponseViewerPane::default();
        assert!(matches!(viewer.state, ViewerState::Empty));
    }

    #[test]
    fn set_loading_changes_state() {
        let mut viewer = ResponseViewerPane::default();
        viewer.set_loading();
        assert!(viewer.is_loading());
    }

    #[test]
    fn set_response_changes_state() {
        let mut viewer = ResponseViewerPane::default();
        set_sample(&mut viewer);
        assert!(matches!(viewer.state, ViewerState::Success { .. }));
    }

    #[test]
    fn set_error_changes_state() {
        let mut viewer = ResponseViewerPane::default();
        viewer.set_error("Connection refused".into());
        assert!(matches!(viewer.state, ViewerState::Error(_)));
    }

    #[test]
    fn scroll_increments_and_decrements() {
        let mut viewer = ResponseViewerPane::default();
        set_sample(&mut viewer);

        viewer.handle_key(key(KeyCode::Char('j')));
        assert_eq!(viewer.scroll_offset, 1);

        viewer.handle_key(key(KeyCode::Char('k')));
        assert_eq!(viewer.scroll_offset, 0);

        // Should not go below 0
        viewer.handle_key(key(KeyCode::Char('k')));
        assert_eq!(viewer.scroll_offset, 0);
    }

    #[test]
    fn tab_switching_with_number_keys() {
        let mut viewer = ResponseViewerPane::default();
        set_sample(&mut viewer);

        assert_eq!(viewer.active_tab, ResponseTab::Body);
        viewer.handle_key(key(KeyCode::Char('2')));
        assert_eq!(viewer.active_tab, ResponseTab::Headers);
        viewer.handle_key(key(KeyCode::Char('3')));
        assert_eq!(viewer.active_tab, ResponseTab::Summary);
        viewer.handle_key(key(KeyCode::Char('4')));
        assert_eq!(viewer.active_tab, ResponseTab::Assertions);
        viewer.handle_key(key(KeyCode::Char('1')));
        assert_eq!(viewer.active_tab, ResponseTab::Body);
    }

    #[test]
    fn go_to_top_and_bottom() {
        let mut viewer = ResponseViewerPane::default();
        set_sample(&mut viewer);

        viewer.handle_key(key(KeyCode::Char('G')));
        assert_eq!(viewer.scroll_offset, u16::MAX);

        viewer.handle_key(key(KeyCode::Char('g')));
        assert_eq!(viewer.scroll_offset, 0);
    }

    #[test]
    fn status_color_mapping() {
        assert_eq!(ResponseViewerPane::status_color(200), Color::Green);
        assert_eq!(ResponseViewerPane::status_color(301), Color::Yellow);
        assert_eq!(ResponseViewerPane::status_color(404), Color::Red);
        assert_eq!(ResponseViewerPane::status_color(500), Color::Magenta);
    }

    #[test]
    fn headers_tab_scrolls_the_visible_rows() {
        let mut viewer = ResponseViewerPane::default();
        viewer.set_response(
            ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![
                    ("first-header".into(), "one".into()),
                    ("second-header".into(), "two".into()),
                    ("third-header".into(), "three".into()),
                ],
                content_type: Some("text/plain".into()),
                content_length: Some(3),
                duration_ms: 10,
                body_text: Some("ok".into()),
                body_bytes: None,
                is_binary: false,
            },
            AssertionReport::default(),
        );
        viewer.active_tab = ResponseTab::Headers;
        viewer.scroll_offset = 1;

        let backend = TestBackend::new(60, 7);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| viewer.render(frame, frame.area(), true))
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(!rendered.contains("first-header"));
        assert!(rendered.contains("second-header"));
    }

    #[test]
    fn headers_tab_redacts_sensitive_values() {
        let mut viewer = ResponseViewerPane::default();
        viewer.set_response(
            ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![
                    ("authorization".into(), "Bearer super-secret-token".into()),
                    ("content-type".into(), "application/json".into()),
                ],
                content_type: Some("application/json".into()),
                content_length: Some(2),
                duration_ms: 10,
                body_text: Some("{}".into()),
                body_bytes: None,
                is_binary: false,
            },
            AssertionReport::default(),
        );
        viewer.active_tab = ResponseTab::Headers;

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| viewer.render(frame, frame.area(), true))
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(!rendered.contains("super-secret-token"));
        assert!(rendered.contains("redacted"));
        assert!(rendered.contains("content-type"));
    }

    #[test]
    fn diff_tab_redacts_sensitive_header_values() {
        let mut viewer = ResponseViewerPane::default();
        let previous = ResponseArtifact {
            status_code: 200,
            http_version: "HTTP/1.1".into(),
            headers: vec![("set-cookie".into(), "session=old-secret".into())],
            content_type: None,
            content_length: None,
            duration_ms: 10,
            body_text: Some("old".into()),
            body_bytes: None,
            is_binary: false,
        };
        let current = ResponseArtifact {
            status_code: 200,
            http_version: "HTTP/1.1".into(),
            headers: vec![("set-cookie".into(), "session=new-secret".into())],
            content_type: None,
            content_length: None,
            duration_ms: 10,
            body_text: Some("new".into()),
            body_bytes: None,
            is_binary: false,
        };
        viewer.set_response_with_diff(current, AssertionReport::default(), Some(&previous));
        viewer.active_tab = ResponseTab::Diff;

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| viewer.render(frame, frame.area(), true))
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(!rendered.contains("old-secret"));
        assert!(!rendered.contains("new-secret"));
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn assertions_tab_renders() {
        use crate::core::models::{Assertion, AssertionResult};

        let mut viewer = ResponseViewerPane::default();
        let report = AssertionReport {
            results: vec![
                AssertionResult {
                    assertion: Assertion::ExpectStatus(200),
                    passed: true,
                    message: "Status 200 matches".into(),
                    actual_value: Some("200".into()),
                },
                AssertionResult {
                    assertion: Assertion::ExpectTimeUnder(100),
                    passed: false,
                    message: "Expected under 100ms, took 150ms".into(),
                    actual_value: Some("150ms".into()),
                },
            ],
        };
        viewer.set_response(sample_response(), report);
        viewer.active_tab = ResponseTab::Assertions;

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| viewer.render(frame, frame.area(), true))
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("PASS"));
        assert!(rendered.contains("FAIL"));
    }

    #[test]
    fn binary_body_w_key_does_not_get_ignored() {
        let mut viewer = ResponseViewerPane::default();
        viewer.set_response(
            ResponseArtifact {
                status_code: 200,
                http_version: "HTTP/1.1".into(),
                headers: vec![("content-type".into(), "application/octet-stream".into())],
                content_type: Some("application/octet-stream".into()),
                content_length: Some(4),
                duration_ms: 10,
                body_text: Some("[Binary response: 4 bytes. Press 'w' to save to disk.]".into()),
                body_bytes: Some(vec![0, 1, 2, 3]),
                is_binary: true,
            },
            AssertionReport::default(),
        );

        let result = viewer.handle_key(key(KeyCode::Char('w')));
        assert!(!matches!(result, EventResult::Ignored));
    }
}
