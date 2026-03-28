pub mod collections;
pub mod request_editor;
pub mod response_viewer;
pub mod status_bar;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::Action;

/// Result of a component handling a key event.
#[allow(dead_code)] // Action variant used by Component trait contract.
pub enum EventResult {
    /// The component consumed the event.
    Consumed,
    /// The component did not handle the event.
    Ignored,
    /// The component produced an application-level action.
    Action(Action),
}

/// Trait for pane components that handle input and render themselves.
pub trait Component {
    fn handle_key(&mut self, key: KeyEvent) -> EventResult;
    fn render(&self, frame: &mut Frame, area: Rect, focused: bool);

    /// Called when this component gains focus.
    fn focus(&mut self) {}

    /// Called when this component loses focus.
    fn blur(&mut self) {}
}
