use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table, Wrap},
};

use crate::core::models::ResponseArtifact;

use super::{Component, EventResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseTab {
    Body,
    Headers,
    Summary,
}

impl ResponseTab {
    const ALL: [ResponseTab; 3] = [
        ResponseTab::Body,
        ResponseTab::Headers,
        ResponseTab::Summary,
    ];

    fn label(self) -> &'static str {
        match self {
            ResponseTab::Body => "Body",
            ResponseTab::Headers => "Headers",
            ResponseTab::Summary => "Summary",
        }
    }
}

#[derive(Debug)]
enum ViewerState {
    Empty,
    Loading,
    Success(ResponseArtifact),
    Error(String),
}

pub struct ResponseViewerPane {
    state: ViewerState,
    active_tab: ResponseTab,
    scroll_offset: u16,
}

impl Default for ResponseViewerPane {
    fn default() -> Self {
        Self {
            state: ViewerState::Empty,
            active_tab: ResponseTab::Body,
            scroll_offset: 0,
        }
    }
}

impl ResponseViewerPane {
    pub fn set_loading(&mut self) {
        self.state = ViewerState::Loading;
        self.scroll_offset = 0;
    }

    pub fn set_response(&mut self, artifact: ResponseArtifact) {
        self.state = ViewerState::Success(artifact);
        self.scroll_offset = 0;
        self.active_tab = ResponseTab::Body;
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
        match key.code {
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

            ViewerState::Success(response) => {
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
                        let body = response.body_text.as_deref().unwrap_or("<empty body>");
                        let paragraph = Paragraph::new(body)
                            .scroll((self.scroll_offset, 0))
                            .wrap(Wrap { trim: false });
                        frame.render_widget(paragraph, chunks[2]);
                    }
                    ResponseTab::Headers => {
                        let visible_rows = chunks[2].height.saturating_sub(1).max(1) as usize;
                        let max_offset = response.headers.len().saturating_sub(visible_rows);
                        let start = usize::min(self.scroll_offset as usize, max_offset);
                        let end = usize::min(start + visible_rows, response.headers.len());

                        let rows: Vec<Row> = response
                            .headers
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
        }
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
        viewer.set_response(sample_response());
        assert!(matches!(viewer.state, ViewerState::Success(_)));
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
        viewer.set_response(sample_response());

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
        viewer.set_response(sample_response());

        assert_eq!(viewer.active_tab, ResponseTab::Body);
        viewer.handle_key(key(KeyCode::Char('2')));
        assert_eq!(viewer.active_tab, ResponseTab::Headers);
        viewer.handle_key(key(KeyCode::Char('3')));
        assert_eq!(viewer.active_tab, ResponseTab::Summary);
        viewer.handle_key(key(KeyCode::Char('1')));
        assert_eq!(viewer.active_tab, ResponseTab::Body);
    }

    #[test]
    fn go_to_top_and_bottom() {
        let mut viewer = ResponseViewerPane::default();
        viewer.set_response(sample_response());

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
        viewer.set_response(ResponseArtifact {
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
        });
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
}
