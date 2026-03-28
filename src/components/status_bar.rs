use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::action::FocusTarget;

pub struct StatusBarState {
    pub mode: &'static str,
    pub is_dirty: bool,
    pub is_loading: bool,
    pub status_message: Option<String>,
}

pub fn render(frame: &mut Frame, area: Rect, focus: &FocusTarget, state: &StatusBarState) {
    let mode_color = if state.mode == "INSERT" {
        Color::Yellow
    } else {
        Color::Cyan
    };

    let mode = Span::styled(
        format!(" {} ", state.mode),
        Style::default()
            .fg(Color::Black)
            .bg(mode_color)
            .add_modifier(Modifier::BOLD),
    );

    let focus_label = Span::styled(
        format!(" {} ", focus.label()),
        Style::default().fg(Color::Black).bg(Color::DarkGray),
    );

    let mut indicators = Vec::new();
    if state.is_dirty {
        indicators.push(Span::styled(
            " [modified] ",
            Style::default().fg(Color::Yellow),
        ));
    }
    if state.is_loading {
        indicators.push(Span::styled(
            " [sending...] ",
            Style::default().fg(Color::Cyan),
        ));
    }

    let hint = |key: &str, desc: &str| -> Vec<Span> {
        vec![
            Span::styled(
                format!(" {key} "),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {desc} "), Style::default().fg(Color::DarkGray)),
        ]
    };

    let mut hints = Vec::new();

    if state.mode == "INSERT" {
        hints.extend(hint("Esc", "exit edit"));
        if matches!(focus, FocusTarget::RequestEditor) {
            hints.extend(hint("Tab", "next field"));
        }
    } else if state.is_loading {
        hints.extend(hint("Esc", "cancel"));
    } else {
        hints.extend(hint("Tab", "cycle"));
        match focus {
            FocusTarget::Collections => {
                hints.extend(hint("Enter", "select"));
                hints.extend(hint("j/k", "navigate"));
                hints.extend(hint("r", "refresh"));
            }
            FocusTarget::RequestEditor => {
                hints.extend(hint("1-4", "tabs"));
                hints.extend(hint("i", "edit"));
                hints.extend(hint("C-r", "send"));
                hints.extend(hint("C-s", "save"));
            }
            FocusTarget::ResponseViewer => {
                hints.extend(hint("1-3", "tabs"));
                hints.extend(hint("j/k", "scroll"));
            }
        }
        hints.extend(hint("q", "quit"));
    }

    let mut first_line = vec![mode, Span::raw(" "), focus_label];
    first_line.extend(indicators);

    if let Some(msg) = &state.status_message {
        first_line.push(Span::raw("  "));
        first_line.push(Span::styled(
            msg.as_str(),
            Style::default().fg(Color::Green),
        ));
    }

    let lines = vec![Line::from(first_line), Line::from(hints)];

    let bar = Paragraph::new(lines);
    frame.render_widget(bar, area);
}
