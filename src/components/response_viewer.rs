use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use super::{Component, EventResult};

pub struct ResponseViewerPane;

impl Default for ResponseViewerPane {
    fn default() -> Self {
        Self
    }
}

impl Component for ResponseViewerPane {
    fn handle_key(&mut self, _key: KeyEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border_color = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let paragraph = Paragraph::new("Send a request to see the response").block(
            Block::default()
                .title(" Response ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        );

        frame.render_widget(paragraph, area);
    }
}
