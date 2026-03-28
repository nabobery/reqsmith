use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem},
};

use super::{Component, EventResult};

pub struct CollectionsPane {
    items: Vec<String>,
    selected: usize,
}

impl Default for CollectionsPane {
    fn default() -> Self {
        Self {
            items: vec![
                "GET /users".into(),
                "POST /users".into(),
                "GET /users/:id".into(),
            ],
            selected: 0,
        }
    }
}

impl Component for CollectionsPane {
    fn handle_key(&mut self, key: KeyEvent) -> EventResult {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.items.is_empty() {
                    self.selected = (self.selected + 1) % self.items.len();
                }
                EventResult::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !self.items.is_empty() {
                    self.selected = self.selected.checked_sub(1).unwrap_or(self.items.len() - 1);
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

        let items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let style = if i == self.selected {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                ListItem::new(item.as_str()).style(style)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .title(" Collections ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        );

        frame.render_widget(list, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn collections_pane_j_navigates_down() {
        let mut pane = CollectionsPane::default();
        assert_eq!(pane.selected, 0);

        let result = pane.handle_key(key(KeyCode::Char('j')));
        assert!(matches!(result, EventResult::Consumed));
        assert_eq!(pane.selected, 1);
    }

    #[test]
    fn collections_pane_k_navigates_up() {
        let mut pane = CollectionsPane {
            selected: 1,
            ..CollectionsPane::default()
        };

        let result = pane.handle_key(key(KeyCode::Char('k')));
        assert!(matches!(result, EventResult::Consumed));
        assert_eq!(pane.selected, 0);
    }

    #[test]
    fn collections_pane_k_wraps_to_bottom() {
        let mut pane = CollectionsPane::default();
        assert_eq!(pane.selected, 0);

        let result = pane.handle_key(key(KeyCode::Char('k')));
        assert!(matches!(result, EventResult::Consumed));
        assert_eq!(pane.selected, 2); // wraps to last item
    }

    #[test]
    fn collections_pane_j_wraps_to_top() {
        let mut pane = CollectionsPane {
            selected: 2,
            ..CollectionsPane::default()
        };

        let result = pane.handle_key(key(KeyCode::Char('j')));
        assert!(matches!(result, EventResult::Consumed));
        assert_eq!(pane.selected, 0);
    }

    #[test]
    fn collections_pane_ignores_unknown_keys() {
        let mut pane = CollectionsPane::default();
        let result = pane.handle_key(key(KeyCode::Char('x')));
        assert!(matches!(result, EventResult::Ignored));
    }
}
