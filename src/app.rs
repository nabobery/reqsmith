use color_eyre::eyre::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Constraint, Direction, Layout};
use tokio::time::{MissedTickBehavior, interval};

use crate::action::{Action, FocusTarget};
use crate::components::Component;
use crate::components::collections::CollectionsPane;
use crate::components::request_editor::RequestEditorPane;
use crate::components::response_viewer::ResponseViewerPane;
use crate::components::status_bar;
use crate::config::Config;
use crate::tui::Tui;

pub struct App {
    config: Config,
    focus: FocusTarget,
    should_quit: bool,
    collections: CollectionsPane,
    request_editor: RequestEditorPane,
    response_viewer: ResponseViewerPane,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            focus: FocusTarget::Collections,
            should_quit: false,
            collections: CollectionsPane::default(),
            request_editor: RequestEditorPane,
            response_viewer: ResponseViewerPane,
        }
    }

    pub async fn run(&mut self, tui: &mut Tui) -> Result<()> {
        let mut events = EventStream::new();
        let mut tick = interval(self.config.tick_rate);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut render = interval(self.config.frame_rate);
        render.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            let maybe_action = tokio::select! {
                _ = render.tick() => {
                    Some(Action::Render)
                }
                event = events.next() => {
                    match event {
                        Some(Ok(e)) => self.handle_event(e),
                        Some(Err(e)) => Some(Action::Error(e.to_string())),
                        None => None,
                    }
                }
                _ = tick.tick() => {
                    Some(Action::Tick)
                }
            };

            if let Some(action) = maybe_action {
                let should_render = action == Action::Render;
                self.update(action);

                if should_render {
                    tui.draw(|frame| self.render(frame))?;
                }
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> Option<Action> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
            Event::Resize(w, h) => Some(Action::Resize(w, h)),
            _ => None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Global keys always take priority.
        match key.code {
            KeyCode::Char('q') => return Some(Action::Quit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(Action::Quit);
            }
            KeyCode::Tab => return Some(Action::FocusNext),
            KeyCode::BackTab => return Some(Action::FocusPrev),
            _ => {}
        }

        // Delegate to the focused component.
        let result = self.focused_component_mut().handle_key(key);
        match result {
            crate::components::EventResult::Action(action) => Some(action),
            _ => None,
        }
    }

    fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::FocusNext => {
                self.focused_component_mut().blur();
                self.focus = self.focus.next();
                self.focused_component_mut().focus();
            }
            Action::FocusPrev => {
                self.focused_component_mut().blur();
                self.focus = self.focus.prev();
                self.focused_component_mut().focus();
            }
            Action::Error(msg) => {
                tracing::error!("Application error: {msg}");
            }
            Action::Tick | Action::Render | Action::Resize(_, _) => {}
        }
    }

    fn render(&self, frame: &mut ratatui::Frame) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(2)])
            .split(frame.area());

        let main_area = outer[0];
        let status_area = outer[1];

        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(main_area);

        let left = main[0];
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main[1]);

        self.collections
            .render(frame, left, self.focus == FocusTarget::Collections);
        self.request_editor
            .render(frame, right[0], self.focus == FocusTarget::RequestEditor);
        self.response_viewer
            .render(frame, right[1], self.focus == FocusTarget::ResponseViewer);

        status_bar::render(frame, status_area, &self.focus);
    }

    fn focused_component_mut(&mut self) -> &mut dyn Component {
        match self.focus {
            FocusTarget::Collections => &mut self.collections,
            FocusTarget::RequestEditor => &mut self.request_editor,
            FocusTarget::ResponseViewer => &mut self.response_viewer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn make_app() -> App {
        App::new(Config::default())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn handle_key_q_returns_quit_action() {
        let mut app = make_app();
        let action = app.handle_key(key(KeyCode::Char('q')));
        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn handle_key_ctrl_c_returns_quit_action() {
        let mut app = make_app();
        let action = app.handle_key(key_ctrl(KeyCode::Char('c')));
        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn handle_key_tab_returns_focus_next() {
        let mut app = make_app();
        let action = app.handle_key(key(KeyCode::Tab));
        assert_eq!(action, Some(Action::FocusNext));
    }

    #[test]
    fn handle_key_backtab_returns_focus_prev() {
        let mut app = make_app();
        let action = app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(action, Some(Action::FocusPrev));
    }

    #[test]
    fn handle_event_ignores_release_key_events() {
        let mut app = make_app();
        let event = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ));

        let action = app.handle_event(event);

        assert_eq!(action, None);
    }

    #[test]
    fn handle_event_accepts_repeat_key_events() {
        let mut app = make_app();
        let event = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ));

        let action = app.handle_event(event);

        assert_eq!(action, Some(Action::Quit));
    }

    #[test]
    fn update_quit_sets_should_quit() {
        let mut app = make_app();
        assert!(!app.should_quit);
        app.update(Action::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn update_focus_next_cycles_focus() {
        let mut app = make_app();
        assert_eq!(app.focus, FocusTarget::Collections);

        app.update(Action::FocusNext);
        assert_eq!(app.focus, FocusTarget::RequestEditor);

        app.update(Action::FocusNext);
        assert_eq!(app.focus, FocusTarget::ResponseViewer);

        app.update(Action::FocusNext);
        assert_eq!(app.focus, FocusTarget::Collections);
    }

    #[test]
    fn update_focus_prev_cycles_focus() {
        let mut app = make_app();
        assert_eq!(app.focus, FocusTarget::Collections);

        app.update(Action::FocusPrev);
        assert_eq!(app.focus, FocusTarget::ResponseViewer);

        app.update(Action::FocusPrev);
        assert_eq!(app.focus, FocusTarget::RequestEditor);

        app.update(Action::FocusPrev);
        assert_eq!(app.focus, FocusTarget::Collections);
    }
}
