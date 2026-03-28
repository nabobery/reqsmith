use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use super::{Component, EventResult};

pub struct RequestEditorPane;

impl Default for RequestEditorPane {
    fn default() -> Self {
        Self
    }
}

impl Component for RequestEditorPane {
    fn handle_key(&mut self, _key: KeyEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border_color = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let paragraph = Paragraph::new("Select a request to edit").block(
            Block::default()
                .title(" Request ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        );

        frame.render_widget(paragraph, area);
    }
}
