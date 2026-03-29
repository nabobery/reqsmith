use color_eyre::eyre::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Constraint, Direction, Layout};
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;

use crate::action::{Action, FocusTarget};
use crate::components::Component;
use crate::components::collections::CollectionsPane;
use crate::components::request_editor::RequestEditorPane;
use crate::components::response_viewer::ResponseViewerPane;
use crate::components::status_bar::{self, StatusBarState};
use crate::config::Config;
use crate::core::models::ResponseArtifact;
use crate::core::repository;
use crate::core::runner::{self, RunOptions};
use crate::core::storage;
use crate::infra::http_client;
use crate::tui::Tui;

pub struct App {
    config: Config,
    focus: FocusTarget,
    should_quit: bool,
    collections: CollectionsPane,
    request_editor: RequestEditorPane,
    response_viewer: ResponseViewerPane,

    // Async communication
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: mpsc::UnboundedReceiver<Action>,

    // Execution state
    http_client: reqwest::Client,
    cancel_token: Option<CancellationToken>,
    cwd: std::path::PathBuf,
    request_in_flight: bool,

    // Response history for diffing
    previous_response: Option<ResponseArtifact>,

    // Status
    status_message: Option<String>,
    status_message_ticks: u32,

    // Plugin registry
    #[cfg(feature = "plugins")]
    plugin_registry: Option<std::sync::Arc<crate::plugins::registry::PluginRegistry>>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();

        let cwd = std::env::current_dir().unwrap_or_default();
        let http_client = http_client::build_client();

        #[cfg(feature = "plugins")]
        let plugin_registry =
            match crate::plugins::registry::PluginRegistry::discover_and_load(&cwd) {
                Ok(r) if !r.is_empty() => {
                    tracing::info!("Loaded {} plugin(s)", r.len());
                    Some(std::sync::Arc::new(r))
                }
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!("Plugin loading failed: {e}");
                    None
                }
            };

        Self {
            config,
            focus: FocusTarget::Collections,
            should_quit: false,
            collections: CollectionsPane::default(),
            request_editor: RequestEditorPane::default(),
            response_viewer: ResponseViewerPane::default(),
            action_tx,
            action_rx,
            http_client,
            cancel_token: None,
            cwd,
            request_in_flight: false,
            previous_response: None,
            status_message: None,
            status_message_ticks: 0,
            #[cfg(feature = "plugins")]
            plugin_registry,
        }
    }

    pub async fn run(&mut self, tui: &mut Tui) -> Result<()> {
        // Spawn initial collection discovery
        self.spawn_collection_discovery();

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
                Some(action) = self.action_rx.recv() => {
                    Some(action)
                }
            };

            if let Some(action) = maybe_action {
                let should_render = matches!(action, Action::Render);
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
        let editing = self.request_editor.is_editing();

        if editing {
            // In insert mode: only Esc and Ctrl-shortcuts are global
            match key.code {
                KeyCode::Esc => {
                    if self.request_in_flight {
                        return Some(Action::CancelRequest);
                    }
                    // Let editor handle Esc to exit insert mode
                    let result = self.request_editor.handle_key(key);
                    return match result {
                        crate::components::EventResult::Action(action) => Some(action),
                        _ => None,
                    };
                }
                _ if key.modifiers.contains(KeyModifiers::CONTROL) => match key.code {
                    KeyCode::Char('c') => return Some(Action::Quit),
                    KeyCode::Char('r') => {
                        self.request_editor.sync_pending_edit();
                        return Some(Action::SendRequest);
                    }
                    KeyCode::Char('s') => {
                        self.request_editor.sync_pending_edit();
                        return Some(Action::SaveRequest);
                    }
                    _ => {}
                },
                _ => {
                    // All other keys go to the focused component in insert mode
                    let result = self.focused_component_mut().handle_key(key);
                    return match result {
                        crate::components::EventResult::Action(action) => Some(action),
                        _ => None,
                    };
                }
            }
            return None;
        }

        // Normal mode: global keys first
        match key.code {
            KeyCode::Char('q') => return Some(Action::Quit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(Action::Quit);
            }
            KeyCode::Tab => return Some(Action::FocusNext),
            KeyCode::BackTab => return Some(Action::FocusPrev),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(Action::SendRequest);
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(Action::SaveRequest);
            }
            KeyCode::Esc if self.request_in_flight => {
                return Some(Action::CancelRequest);
            }
            _ => {}
        }

        // Delegate to focused component
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
                self.set_status_message(format!("Error: {msg}"));
            }

            // Collection actions
            Action::CollectionsDiscovered(nodes) => {
                self.collections.set_collection(nodes);
            }
            Action::RefreshCollections => {
                self.spawn_collection_discovery();
            }
            Action::SelectRequest(path) => {
                let tx = self.action_tx.clone();
                tokio::task::spawn_blocking(move || match repository::load_request(&path) {
                    Ok(doc) => {
                        let _ = tx.send(Action::RequestLoaded(Box::new(doc)));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::Error(e.to_string()));
                    }
                });
            }
            Action::RequestLoaded(doc) => {
                self.request_editor.load_document(*doc);
                self.focus = FocusTarget::RequestEditor;
            }

            // Save
            Action::SaveRequest => {
                self.request_editor.sync_pending_edit();
                if let Some(doc) = self.request_editor.to_document() {
                    if let Some(path) = doc.file_path.clone() {
                        let tx = self.action_tx.clone();
                        tokio::task::spawn_blocking(move || {
                            match repository::save_request(&doc, &path) {
                                Ok(()) => {
                                    let _ = tx.send(Action::RequestSaved(path));
                                }
                                Err(e) => {
                                    let _ = tx.send(Action::Error(e.to_string()));
                                }
                            }
                        });
                    } else {
                        self.set_status_message("Cannot save: no file path".into());
                    }
                }
            }
            Action::RequestSaved(path) => {
                self.request_editor.set_clean();
                self.set_status_message(format!(
                    "Saved to {}",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("file")
                ));
            }

            // Execution
            Action::SendRequest => {
                if self.request_in_flight {
                    return;
                }
                self.request_editor.sync_pending_edit();
                if let Some(doc) = self.request_editor.to_document() {
                    let client = self.http_client.clone();
                    let tx = self.action_tx.clone();
                    let token = CancellationToken::new();
                    self.cancel_token = Some(token.clone());
                    self.request_in_flight = true;
                    self.response_viewer.set_loading();

                    let options = RunOptions {
                        env_name: None,
                        cli_vars: vec![],
                        validate_before_run: false,
                        cwd: self.cwd.clone(),
                        #[cfg(feature = "plugins")]
                        plugin_registry: self.plugin_registry.clone(),
                    };

                    tokio::spawn(async move {
                        let result = runner::run_request(&client, &doc, &options, token).await;
                        if let Some(artifact) = result.response {
                            let _ = tx.send(Action::RequestCompleted(
                                Box::new(artifact),
                                Box::new(result.assertions),
                            ));
                        } else if result.cancelled {
                            let _ = tx.send(Action::RequestCancelled);
                        } else if let Some(error) = result.error {
                            let _ = tx.send(Action::RequestFailed(error));
                        }
                    });
                }
            }
            Action::CancelRequest => {
                if let Some(token) = self.cancel_token.take() {
                    token.cancel();
                }
            }
            Action::RequestCompleted(artifact, assertions) => {
                self.request_in_flight = false;
                self.cancel_token = None;
                let duration = artifact.duration_ms;
                // Save current response as previous for diffing
                let prev = self.response_viewer.take_response();
                self.previous_response = prev;
                self.response_viewer.set_response_with_diff(
                    *artifact,
                    *assertions,
                    self.previous_response.as_ref(),
                );
                self.set_status_message(format!("Response received in {duration}ms"));
            }
            Action::RequestFailed(msg) => {
                self.request_in_flight = false;
                self.cancel_token = None;
                self.response_viewer.set_error(msg);
            }
            Action::RequestCancelled => {
                self.request_in_flight = false;
                self.cancel_token = None;
                self.response_viewer.set_cancelled();
                self.set_status_message("Request cancelled".into());
            }
            Action::SaveResponseBody => {
                let Some(bytes) = self.response_viewer.binary_body_bytes().map(|b| b.to_vec())
                else {
                    self.set_status_message("No binary response body available to save".into());
                    return;
                };

                let cwd = self.cwd.clone();
                let tx = self.action_tx.clone();
                tokio::task::spawn_blocking(move || {
                    match storage::save_response_body(&bytes, &cwd) {
                        Ok(path) => {
                            let _ = tx.send(Action::StatusMessage(format!(
                                "Saved response body to {}",
                                path.display()
                            )));
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Action::Error(format!("Failed to save response body: {e}")));
                        }
                    }
                });
            }

            Action::StatusMessage(msg) => {
                self.set_status_message(msg);
            }

            Action::Tick => {
                // Clear status message after a few ticks
                if self.status_message.is_some() {
                    self.status_message_ticks += 1;
                    if self.status_message_ticks > 20 {
                        // ~5 seconds at 250ms tick
                        self.status_message = None;
                        self.status_message_ticks = 0;
                    }
                }
            }

            Action::Render | Action::Resize(_, _) => {}
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

        let mode = if self.request_editor.is_editing() {
            "INSERT"
        } else {
            "NORMAL"
        };

        let plugin_count = {
            #[cfg(feature = "plugins")]
            {
                self.plugin_registry.as_ref().map_or(0, |r| r.len())
            }
            #[cfg(not(feature = "plugins"))]
            {
                0
            }
        };
        let bar_state = StatusBarState {
            mode,
            is_dirty: self.request_editor.is_dirty(),
            is_loading: self.request_in_flight,
            status_message: self.status_message.clone(),
            plugin_count,
        };

        status_bar::render(frame, status_area, &self.focus, &bar_state);
    }

    fn set_status_message(&mut self, msg: String) {
        self.status_message = Some(msg);
        self.status_message_ticks = 0;
    }

    fn spawn_collection_discovery(&self) {
        let tx = self.action_tx.clone();
        let cwd = std::env::current_dir().unwrap_or_default();
        tokio::task::spawn_blocking(move || match repository::discover_requests(&cwd) {
            Ok(nodes) => {
                let _ = tx.send(Action::CollectionsDiscovered(nodes));
            }
            Err(e) => {
                let _ = tx.send(Action::Error(format!("Discovery failed: {e}")));
            }
        });
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
        assert!(matches!(action, Some(Action::Quit)));
    }

    #[test]
    fn handle_key_ctrl_c_returns_quit_action() {
        let mut app = make_app();
        let action = app.handle_key(key_ctrl(KeyCode::Char('c')));
        assert!(matches!(action, Some(Action::Quit)));
    }

    #[test]
    fn handle_key_tab_returns_focus_next() {
        let mut app = make_app();
        let action = app.handle_key(key(KeyCode::Tab));
        assert!(matches!(action, Some(Action::FocusNext)));
    }

    #[test]
    fn handle_key_backtab_returns_focus_prev() {
        let mut app = make_app();
        let action = app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert!(matches!(action, Some(Action::FocusPrev)));
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
        assert!(action.is_none());
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
        assert!(matches!(action, Some(Action::Quit)));
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

    #[test]
    fn send_request_when_no_document_does_nothing() {
        let mut app = make_app();
        app.update(Action::SendRequest);
        assert!(!app.request_in_flight);
    }

    #[test]
    fn cancel_request_when_not_in_flight_is_noop() {
        let mut app = make_app();
        app.update(Action::CancelRequest);
        assert!(!app.request_in_flight);
    }

    #[test]
    fn request_completed_clears_in_flight() {
        let mut app = make_app();
        app.request_in_flight = true;

        let artifact = crate::core::models::ResponseArtifact {
            status_code: 200,
            http_version: "HTTP/1.1".into(),
            headers: vec![],
            content_type: None,
            content_length: None,
            duration_ms: 100,
            body_text: Some("ok".into()),
            body_bytes: None,
            is_binary: false,
        };

        app.update(Action::RequestCompleted(
            Box::new(artifact),
            Box::<crate::core::models::AssertionReport>::default(),
        ));
        assert!(!app.request_in_flight);
    }

    #[test]
    fn request_failed_clears_in_flight() {
        let mut app = make_app();
        app.request_in_flight = true;
        app.update(Action::RequestFailed("timeout".into()));
        assert!(!app.request_in_flight);
    }

    #[test]
    fn ctrl_r_sends_request_in_normal_mode() {
        let mut app = make_app();
        let action = app.handle_key(key_ctrl(KeyCode::Char('r')));
        assert!(matches!(action, Some(Action::SendRequest)));
    }

    #[test]
    fn ctrl_s_triggers_save_in_normal_mode() {
        let mut app = make_app();
        let action = app.handle_key(key_ctrl(KeyCode::Char('s')));
        assert!(matches!(action, Some(Action::SaveRequest)));
    }

    #[test]
    fn status_message_clears_after_ticks() {
        let mut app = make_app();
        app.set_status_message("test".into());
        assert!(app.status_message.is_some());

        for _ in 0..21 {
            app.update(Action::Tick);
        }
        assert!(app.status_message.is_none());
    }
}
