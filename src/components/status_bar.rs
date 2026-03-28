use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::action::FocusTarget;

pub fn render(frame: &mut Frame, area: Rect, focus: &FocusTarget) {
    let mode = Span::styled(
        " NORMAL ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let focus_label = Span::styled(
        format!(" {} ", focus.label()),
        Style::default().fg(Color::Black).bg(Color::DarkGray),
    );

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
    hints.extend(hint("Tab", "cycle"));
    hints.extend(hint("j/k", "navigate"));
    hints.extend(hint("q", "quit"));

    let lines = vec![
        Line::from(vec![mode, Span::raw(" "), focus_label]),
        Line::from(hints),
    ];

    let bar = Paragraph::new(lines);
    frame.render_widget(bar, area);
}
