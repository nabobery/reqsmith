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
        truncated: false,
    }
}

fn set_sample(viewer: &mut ResponseViewerPane) {
    viewer.set_response(sample_response(), AssertionReport::default());
}

/// The rendered buffer as one string per terminal row (borders included).
fn render_rows(viewer: &ResponseViewerPane, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| viewer.render(frame, frame.area(), true))
        .unwrap();
    let symbols: Vec<String> = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol().to_string())
        .collect();
    symbols
        .chunks(width as usize)
        .map(|row| row.concat())
        .collect()
}

fn rendered_text(viewer: &ResponseViewerPane, width: u16, height: u16) -> String {
    render_rows(viewer, width, height).concat()
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
fn set_error_clears_the_previous_responses_headers() {
    let mut viewer = ResponseViewerPane::default();
    viewer.set_response(
        ResponseArtifact {
            status_code: 200,
            http_version: "HTTP/1.1".into(),
            headers: vec![("x-distinctive-header".into(), "distinctive-value".into())],
            content_type: Some("application/json".into()),
            content_length: None,
            duration_ms: 10,
            body_text: Some("{}".into()),
            body_bytes: None,
            is_binary: false,
            truncated: false,
        },
        AssertionReport::default(),
    );
    viewer.active_tab = ResponseTab::Headers;
    assert!(viewer.current_response().is_some());

    viewer.set_error("Connection refused".into());

    // A previous response's headers must never survive into an Error
    // state's render, even while the tab selection is stale (still
    // Headers).
    let rendered = rendered_text(&viewer, 80, 10);
    assert!(!rendered.contains("x-distinctive-header"));
    assert!(!rendered.contains("distinctive-value"));
    assert!(viewer.current_response().is_none());
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
            truncated: false,
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
    // Longer than the secret below, so if this renders in full the value
    // column is wide enough that a leaked secret could not be hidden by
    // column truncation.
    const LONG_SAFE_VALUE: &str = "application/vnd.example.v3+json;charset=utf8";

    let mut viewer = ResponseViewerPane::default();
    viewer.set_response(
        ResponseArtifact {
            status_code: 200,
            http_version: "HTTP/1.1".into(),
            headers: vec![
                ("authorization".into(), "Bearer super-secret-token".into()),
                ("content-type".into(), LONG_SAFE_VALUE.into()),
            ],
            content_type: Some("application/json".into()),
            content_length: Some(2),
            duration_ms: 10,
            body_text: Some("{}".into()),
            body_bytes: None,
            is_binary: false,
            truncated: false,
        },
        AssertionReport::default(),
    );
    viewer.active_tab = ResponseTab::Headers;

    let rendered = rendered_text(&viewer, 80, 10);

    assert!(!rendered.contains("super-secret-token"));
    assert!(rendered.contains(redaction::REDACTED_PREFIX));
    assert!(rendered.contains("content-type"));
    assert!(
        LONG_SAFE_VALUE.len() > "Bearer super-secret-token".len()
            && rendered.contains(LONG_SAFE_VALUE),
        "column truncation could be masking a leak"
    );
}

#[test]
fn diff_tab_redacts_sensitive_header_values() {
    // Longer than either secret, so its full rendering proves the diff
    // pane is not simply truncating values out of sight.
    const LONG_SAFE_VALUE: &str = "0123456789abcdef0123456789abcdef";

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
        truncated: false,
    };
    let current = ResponseArtifact {
        status_code: 200,
        http_version: "HTTP/1.1".into(),
        headers: vec![
            ("set-cookie".into(), "session=new-secret".into()),
            ("x-trace-id".into(), LONG_SAFE_VALUE.into()),
        ],
        content_type: None,
        content_length: None,
        duration_ms: 10,
        body_text: Some("new".into()),
        body_bytes: None,
        is_binary: false,
        truncated: false,
    };
    viewer.set_response_with_diff(current, AssertionReport::default(), Some(&previous));
    viewer.active_tab = ResponseTab::Diff;

    let rendered = rendered_text(&viewer, 80, 12);

    assert!(!rendered.contains("old-secret"));
    assert!(!rendered.contains("new-secret"));
    assert!(rendered.contains(redaction::REDACTED_PREFIX));
    assert!(
        LONG_SAFE_VALUE.len() > "session=new-secret".len() && rendered.contains(LONG_SAFE_VALUE),
        "truncation could be masking a leak"
    );
}

#[test]
fn diff_body_lines_use_the_shared_sigil_prefix() {
    let mut viewer = ResponseViewerPane::default();
    let mut previous = sample_response();
    previous.body_text = Some("old-line".into());
    let mut current = sample_response();
    current.body_text = Some("new-line".into());
    viewer.set_response_with_diff(current, AssertionReport::default(), Some(&previous));
    viewer.active_tab = ResponseTab::Diff;

    // The prefix the CLI renderer builds from the same accessors.
    let inserted = DiffLine::Insert("new-line".into());
    let expected = format!(" {}{}", inserted.sigil(), inserted.text());
    assert_eq!(expected, " +new-line");

    let rows = render_rows(&viewer, 80, 12);
    assert!(
        rows.iter().any(|row| row
            .chars()
            .skip(1)
            .collect::<String>()
            .starts_with(&expected)),
        "no row rendered with the shared prefix: {rows:#?}"
    );
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
fn binary_placeholder_and_save_hint_come_from_the_viewer() {
    let mut response = sample_response();
    response.body_text = None;
    response.body_bytes = Some(vec![0, 1, 2, 3]);
    response.is_binary = true;

    let text = ResponseViewerPane::body_display_text(&response);
    assert!(text.contains("4 bytes"));
    assert!(text.contains("Press 'w' to save to disk"));
}

#[test]
fn status_line_reports_a_truncated_body() {
    let mut viewer = ResponseViewerPane::default();
    let untruncated = sample_response();
    viewer.set_response(untruncated, AssertionReport::default());
    assert!(!rendered_text(&viewer, 100, 12).contains("truncated at"));

    let mut truncated = sample_response();
    truncated.truncated = true;
    viewer.set_response(truncated, AssertionReport::default());
    assert!(rendered_text(&viewer, 100, 12).contains("truncated at 10 MB"));
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
            body_text: None,
            body_bytes: Some(vec![0, 1, 2, 3]),
            is_binary: true,
            truncated: false,
        },
        AssertionReport::default(),
    );

    let result = viewer.handle_key(key(KeyCode::Char('w')));
    assert!(!matches!(result, EventResult::Ignored));
}
