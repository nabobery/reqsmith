use std::collections::HashSet;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};

use crate::action::Action;
use crate::core::models::{CollectionNode, CollectionNodeKind};

use super::{Component, EventResult};

/// Flattened tree node for list rendering.
struct FlatNode {
    display_name: String,
    path: PathBuf,
    kind: CollectionNodeKind,
    depth: usize,
    is_expanded: bool,
    #[allow(dead_code)]
    has_children: bool,
}

#[derive(Default)]
pub struct CollectionsPane {
    roots: Vec<CollectionNode>,
    collapsed_paths: HashSet<PathBuf>,
    nodes: Vec<FlatNode>,
    selected: usize,
    focused: bool,
}

impl CollectionsPane {
    /// Replace the tree with newly discovered collection nodes.
    pub fn set_collection(&mut self, roots: Vec<CollectionNode>) {
        self.roots = roots;
        self.collapsed_paths.clear();
        self.rebuild_visible_nodes(None);
    }

    fn flatten_node(&mut self, node: &CollectionNode) {
        let has_children = !node.children.is_empty();
        let is_expanded = has_children && !self.collapsed_paths.contains(&node.path);
        self.nodes.push(FlatNode {
            display_name: node.name.clone(),
            path: node.path.clone(),
            kind: node.kind.clone(),
            depth: node.depth,
            is_expanded,
            has_children,
        });

        if is_expanded {
            for child in &node.children {
                self.flatten_node(child);
            }
        }
    }

    fn rebuild_visible_nodes(&mut self, selected_path: Option<&PathBuf>) {
        self.nodes.clear();

        let roots = self.roots.clone();
        for root in &roots {
            self.flatten_node(root);
        }

        self.selected = selected_path
            .and_then(|path| self.nodes.iter().position(|node| &node.path == path))
            .unwrap_or(0);

        if self.selected >= self.nodes.len() {
            self.selected = 0;
        }
    }

    #[allow(dead_code)]
    /// Returns true if the collection is empty (no files discovered).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn toggle_expand(&mut self) {
        if self.nodes.is_empty() {
            return;
        }
        let node = &self.nodes[self.selected];
        if node.kind != CollectionNodeKind::Directory {
            return;
        }

        let selected_path = node.path.clone();
        if node.is_expanded {
            self.collapsed_paths.insert(selected_path.clone());
        } else {
            self.collapsed_paths.remove(&selected_path);
        }

        self.rebuild_visible_nodes(Some(&selected_path));
    }
}

impl Component for CollectionsPane {
    fn handle_key(&mut self, key: KeyEvent) -> EventResult {
        if self.nodes.is_empty() {
            return match key.code {
                KeyCode::Char('r') => EventResult::Action(Action::RefreshCollections),
                _ => EventResult::Ignored,
            };
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected = (self.selected + 1) % self.nodes.len();
                EventResult::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.checked_sub(1).unwrap_or(self.nodes.len() - 1);
                EventResult::Consumed
            }
            KeyCode::Enter => {
                let node = &self.nodes[self.selected];
                match node.kind {
                    CollectionNodeKind::RequestFile => {
                        EventResult::Action(Action::SelectRequest(node.path.clone()))
                    }
                    CollectionNodeKind::Directory => {
                        self.toggle_expand();
                        EventResult::Consumed
                    }
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if self.nodes[self.selected].kind == CollectionNodeKind::Directory
                    && !self.nodes[self.selected].is_expanded
                {
                    self.toggle_expand();
                }
                EventResult::Consumed
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if self.nodes[self.selected].kind == CollectionNodeKind::Directory
                    && self.nodes[self.selected].is_expanded
                {
                    self.toggle_expand();
                }
                EventResult::Consumed
            }
            KeyCode::Char('r') => EventResult::Action(Action::RefreshCollections),
            _ => EventResult::Ignored,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border_color = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        if self.nodes.is_empty() {
            let paragraph =
                ratatui::widgets::Paragraph::new("No .req.yml files found.\nPress 'r' to refresh.")
                    .block(
                        Block::default()
                            .title(" Collections ")
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(border_color)),
                    )
                    .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(paragraph, area);
            return;
        }

        let items: Vec<ListItem> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let indent = "  ".repeat(node.depth);
                let icon = match &node.kind {
                    CollectionNodeKind::Directory => {
                        if node.is_expanded {
                            "v "
                        } else {
                            "> "
                        }
                    }
                    CollectionNodeKind::RequestFile => "  ",
                };

                let style = if i == self.selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    match &node.kind {
                        CollectionNodeKind::Directory => Style::default().fg(Color::Blue),
                        CollectionNodeKind::RequestFile => Style::default(),
                    }
                };

                ListItem::new(Line::from(vec![
                    Span::raw(indent),
                    Span::styled(icon, style),
                    Span::styled(&node.display_name, style),
                ]))
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

    fn focus(&mut self) {
        self.focused = true;
    }

    fn blur(&mut self) {
        self.focused = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn sample_tree() -> Vec<CollectionNode> {
        vec![
            CollectionNode {
                name: "api".into(),
                path: "/api".into(),
                kind: CollectionNodeKind::Directory,
                depth: 0,
                children: vec![
                    CollectionNode {
                        name: "get_users".into(),
                        path: "/api/get_users.req.yml".into(),
                        kind: CollectionNodeKind::RequestFile,
                        depth: 1,
                        children: vec![],
                    },
                    CollectionNode {
                        name: "create_user".into(),
                        path: "/api/create_user.req.yml".into(),
                        kind: CollectionNodeKind::RequestFile,
                        depth: 1,
                        children: vec![],
                    },
                ],
            },
            CollectionNode {
                name: "health".into(),
                path: "/health.req.yml".into(),
                kind: CollectionNodeKind::RequestFile,
                depth: 0,
                children: vec![],
            },
        ]
    }

    #[test]
    fn set_collection_flattens_tree() {
        let mut pane = CollectionsPane::default();
        pane.set_collection(sample_tree());
        // api dir + 2 children + health file = 4
        assert_eq!(pane.nodes.len(), 4);
    }

    #[test]
    fn enter_on_file_emits_select_request() {
        let mut pane = CollectionsPane::default();
        pane.set_collection(sample_tree());
        // Navigate to first file (index 1 = get_users)
        pane.selected = 1;
        let result = pane.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            result,
            EventResult::Action(Action::SelectRequest(_))
        ));
    }

    #[test]
    fn j_k_navigation_works() {
        let mut pane = CollectionsPane::default();
        pane.set_collection(sample_tree());
        assert_eq!(pane.selected, 0);

        pane.handle_key(key(KeyCode::Char('j')));
        assert_eq!(pane.selected, 1);

        pane.handle_key(key(KeyCode::Char('k')));
        assert_eq!(pane.selected, 0);
    }

    #[test]
    fn j_wraps_to_top() {
        let mut pane = CollectionsPane::default();
        pane.set_collection(sample_tree());
        pane.selected = pane.nodes.len() - 1;

        pane.handle_key(key(KeyCode::Char('j')));
        assert_eq!(pane.selected, 0);
    }

    #[test]
    fn k_wraps_to_bottom() {
        let mut pane = CollectionsPane::default();
        pane.set_collection(sample_tree());
        assert_eq!(pane.selected, 0);

        pane.handle_key(key(KeyCode::Char('k')));
        assert_eq!(pane.selected, pane.nodes.len() - 1);
    }

    #[test]
    fn empty_collection_shows_refresh_hint() {
        let mut pane = CollectionsPane::default();
        assert!(pane.is_empty());

        let result = pane.handle_key(key(KeyCode::Char('r')));
        assert!(matches!(
            result,
            EventResult::Action(Action::RefreshCollections)
        ));
    }

    #[test]
    fn collapsed_directory_can_be_expanded_again() {
        let mut pane = CollectionsPane::default();
        pane.set_collection(sample_tree());
        assert_eq!(pane.nodes.len(), 4);

        pane.selected = 0;
        pane.handle_key(key(KeyCode::Enter));
        assert_eq!(pane.nodes.len(), 2);

        pane.handle_key(key(KeyCode::Enter));
        assert_eq!(pane.nodes.len(), 4);
    }
}
