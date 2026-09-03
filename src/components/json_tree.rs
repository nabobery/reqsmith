use std::collections::HashSet;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonNodeType {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

#[derive(Debug, Clone)]
pub struct JsonTreeNode {
    pub key: Option<String>,
    pub value_preview: String,
    pub depth: usize,
    pub is_expandable: bool,
    pub is_expanded: bool,
    pub node_type: JsonNodeType,
    /// Dot-notation path for tracking collapse state.
    pub path: String,
}

pub struct JsonTreeState {
    /// The original JSON value used to rebuild visible nodes.
    root: JsonValue,
    /// Current visible (flattened) nodes.
    nodes: Vec<JsonTreeNode>,
    /// Set of paths that are collapsed.
    collapsed: HashSet<String>,
    /// Index of the selected node within `nodes`.
    selected: usize,
    /// Vertical scroll offset for rendering.
    scroll_offset: usize,
}

impl JsonTreeState {
    /// Build initial state from a JSON value. All nodes start expanded.
    pub fn from_value(value: &JsonValue) -> Self {
        let mut state = Self {
            root: value.clone(),
            nodes: Vec::new(),
            collapsed: HashSet::new(),
            selected: 0,
            scroll_offset: 0,
        };
        state.rebuild_visible();
        state
    }

    /// Toggle expand/collapse on the selected node.
    pub fn toggle_selected(&mut self) {
        if let Some(node) = self.nodes.get(self.selected)
            && node.is_expandable
        {
            let path = node.path.clone();
            if self.collapsed.contains(&path) {
                self.collapsed.remove(&path);
            } else {
                self.collapsed.insert(path);
            }
            self.rebuild_visible();
            // Clamp selection after rebuild.
            if self.selected >= self.nodes.len() && !self.nodes.is_empty() {
                self.selected = self.nodes.len() - 1;
            }
        }
    }

    /// Collapse the selected node if it is expandable and currently expanded.
    pub fn collapse_selected(&mut self) {
        if let Some(node) = self.nodes.get(self.selected)
            && node.is_expandable
            && node.is_expanded
        {
            let path = node.path.clone();
            self.collapsed.insert(path);
            self.rebuild_visible();
            if self.selected >= self.nodes.len() && !self.nodes.is_empty() {
                self.selected = self.nodes.len() - 1;
            }
        }
    }

    /// Expand the selected node if it is expandable and currently collapsed.
    pub fn expand_selected(&mut self) {
        if let Some(node) = self.nodes.get(self.selected)
            && node.is_expandable
            && !node.is_expanded
        {
            let path = node.path.clone();
            self.collapsed.remove(&path);
            self.rebuild_visible();
        }
    }

    /// Move selection up by one, clamping at 0.
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Move selection down by one, clamping at the last node.
    pub fn move_down(&mut self) {
        if !self.nodes.is_empty() && self.selected < self.nodes.len() - 1 {
            self.selected += 1;
        }
    }

    /// Return the currently visible nodes.
    #[cfg(test)]
    pub fn visible_nodes(&self) -> &[JsonTreeNode] {
        &self.nodes
    }

    /// Return the selected index.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Render visible nodes to ratatui `Line`s suitable for display.
    ///
    /// `width` is the available character width (unused for wrapping but kept for API symmetry).
    /// `height` is the number of visible rows.
    /// `selected` is the index of the highlighted row within `self.nodes`.
    pub fn render_lines(
        &self,
        _width: usize,
        height: usize,
        selected: usize,
    ) -> Vec<Line<'static>> {
        // Determine the visible window with scroll adjustment.
        let total = self.nodes.len();
        if total == 0 {
            return vec![Line::from(Span::styled(
                "<empty>",
                Style::default().fg(Color::DarkGray),
            ))];
        }

        // Compute scroll offset so `selected` is always visible.
        let scroll = if selected < self.scroll_offset {
            selected
        } else if selected >= self.scroll_offset + height {
            selected.saturating_sub(height - 1)
        } else {
            self.scroll_offset
        };

        let end = total.min(scroll + height);

        (scroll..end)
            .map(|i| {
                let node = &self.nodes[i];
                let is_selected = i == selected;
                self.render_node_line(node, is_selected)
            })
            .collect()
    }

    // ---- private helpers ----

    /// Rebuild the flat visible node list from the root value and collapse state.
    fn rebuild_visible(&mut self) {
        self.nodes.clear();
        Self::flatten_value(
            &mut self.nodes,
            &self.collapsed,
            &self.root,
            None,
            0,
            "root",
        );
    }

    /// Recursively flatten a JSON value into the node list.
    fn flatten_value(
        nodes: &mut Vec<JsonTreeNode>,
        collapsed: &HashSet<String>,
        value: &JsonValue,
        key: Option<String>,
        depth: usize,
        path: &str,
    ) {
        match value {
            JsonValue::Object(map) => {
                let is_collapsed = collapsed.contains(path);
                let preview = format!("{{ {} keys }}", map.len());
                nodes.push(JsonTreeNode {
                    key,
                    value_preview: preview,
                    depth,
                    is_expandable: true,
                    is_expanded: !is_collapsed,
                    node_type: JsonNodeType::Object,
                    path: path.to_string(),
                });
                if !is_collapsed {
                    let mut keys: Vec<&String> = map.keys().collect();
                    keys.sort();
                    for k in keys {
                        let child_path = format!("{}.{}", path, k);
                        Self::flatten_value(
                            nodes,
                            collapsed,
                            &map[k],
                            Some(k.clone()),
                            depth + 1,
                            &child_path,
                        );
                    }
                }
            }
            JsonValue::Array(arr) => {
                let is_collapsed = collapsed.contains(path);
                let preview = format!("[ {} items ]", arr.len());
                nodes.push(JsonTreeNode {
                    key,
                    value_preview: preview,
                    depth,
                    is_expandable: true,
                    is_expanded: !is_collapsed,
                    node_type: JsonNodeType::Array,
                    path: path.to_string(),
                });
                if !is_collapsed {
                    for (i, v) in arr.iter().enumerate() {
                        let child_path = format!("{}[{}]", path, i);
                        Self::flatten_value(
                            nodes,
                            collapsed,
                            v,
                            Some(i.to_string()),
                            depth + 1,
                            &child_path,
                        );
                    }
                }
            }
            JsonValue::String(s) => {
                let preview = format!("\"{}\"", s);
                nodes.push(JsonTreeNode {
                    key,
                    value_preview: preview,
                    depth,
                    is_expandable: false,
                    is_expanded: false,
                    node_type: JsonNodeType::String,
                    path: path.to_string(),
                });
            }
            JsonValue::Number(n) => {
                nodes.push(JsonTreeNode {
                    key,
                    value_preview: n.to_string(),
                    depth,
                    is_expandable: false,
                    is_expanded: false,
                    node_type: JsonNodeType::Number,
                    path: path.to_string(),
                });
            }
            JsonValue::Bool(b) => {
                nodes.push(JsonTreeNode {
                    key,
                    value_preview: b.to_string(),
                    depth,
                    is_expandable: false,
                    is_expanded: false,
                    node_type: JsonNodeType::Bool,
                    path: path.to_string(),
                });
            }
            JsonValue::Null => {
                nodes.push(JsonTreeNode {
                    key,
                    value_preview: "null".to_string(),
                    depth,
                    is_expandable: false,
                    is_expanded: false,
                    node_type: JsonNodeType::Null,
                    path: path.to_string(),
                });
            }
        }
    }

    /// Render a single node to a styled `Line`.
    fn render_node_line(&self, node: &JsonTreeNode, is_selected: bool) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();

        // Indent.
        let indent = "  ".repeat(node.depth);
        spans.push(Span::raw(indent));

        // Arrow / expandable indicator.
        let arrow = if node.is_expandable {
            if node.is_expanded { "▼ " } else { "▶ " }
        } else {
            "  "
        };
        spans.push(Span::raw(arrow.to_string()));

        // Key.
        if let Some(ref k) = node.key {
            let key_style = if node.depth > 0
                && matches!(node.node_type, JsonNodeType::Object | JsonNodeType::Array)
            {
                // For expandable children keyed in an array, use dim style.
                // For object keys, use cyan.
                if k.parse::<usize>().is_ok() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::Cyan)
                }
            } else if k.parse::<usize>().is_ok() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Cyan)
            };
            spans.push(Span::styled(k.clone(), key_style));
            spans.push(Span::raw(": "));
        }

        // Value preview with type-based coloring.
        let value_style = match node.node_type {
            JsonNodeType::String => Style::default().fg(Color::Green),
            JsonNodeType::Number => Style::default().fg(Color::Yellow),
            JsonNodeType::Bool => Style::default().fg(Color::Magenta),
            JsonNodeType::Null => Style::default().fg(Color::DarkGray),
            JsonNodeType::Object | JsonNodeType::Array => Style::default().fg(Color::DarkGray),
        };
        spans.push(Span::styled(node.value_preview.clone(), value_style));

        // Apply highlight for selected line.
        if is_selected {
            let bold_mod = Modifier::BOLD;
            spans = spans
                .into_iter()
                .map(|span| {
                    Span::styled(
                        span.content.into_owned(),
                        span.style.add_modifier(bold_mod).bg(Color::DarkGray),
                    )
                })
                .collect();
        }

        Line::from(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_value_simple_object() {
        let val = json!({"alpha": 1, "beta": "hello"});
        let state = JsonTreeState::from_value(&val);
        let nodes = state.visible_nodes();

        // Root object + 2 children = 3 nodes.
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].node_type, JsonNodeType::Object);
        assert!(nodes[0].is_expandable);
        assert!(nodes[0].is_expanded);
        // Children are sorted: alpha, beta.
        assert_eq!(nodes[1].key.as_deref(), Some("alpha"));
        assert_eq!(nodes[1].node_type, JsonNodeType::Number);
        assert_eq!(nodes[2].key.as_deref(), Some("beta"));
        assert_eq!(nodes[2].node_type, JsonNodeType::String);
    }

    #[test]
    fn from_value_nested() {
        let val = json!({"outer": {"inner": 42}});
        let state = JsonTreeState::from_value(&val);
        let nodes = state.visible_nodes();

        // root(0) -> outer(1, depth=1) -> inner(2, depth=2)
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].depth, 0);
        assert_eq!(nodes[1].depth, 1);
        assert_eq!(nodes[1].key.as_deref(), Some("outer"));
        assert_eq!(nodes[1].node_type, JsonNodeType::Object);
        assert_eq!(nodes[2].depth, 2);
        assert_eq!(nodes[2].key.as_deref(), Some("inner"));
    }

    #[test]
    fn collapse_hides_children() {
        let val = json!({"a": 1, "b": 2});
        let mut state = JsonTreeState::from_value(&val);

        // Initially 3 visible nodes (root + 2 children).
        assert_eq!(state.visible_nodes().len(), 3);

        // Collapse root (selected = 0).
        state.toggle_selected();
        assert_eq!(state.visible_nodes().len(), 1);
        assert!(!state.visible_nodes()[0].is_expanded);
    }

    #[test]
    fn expand_shows_children() {
        let val = json!({"a": 1, "b": 2});
        let mut state = JsonTreeState::from_value(&val);

        // Collapse then expand.
        state.toggle_selected();
        assert_eq!(state.visible_nodes().len(), 1);

        state.toggle_selected();
        assert_eq!(state.visible_nodes().len(), 3);
        assert!(state.visible_nodes()[0].is_expanded);
    }

    #[test]
    fn move_up_down() {
        let val = json!({"a": 1, "b": 2, "c": 3});
        let mut state = JsonTreeState::from_value(&val);

        assert_eq!(state.selected(), 0);

        state.move_down();
        assert_eq!(state.selected(), 1);

        state.move_down();
        assert_eq!(state.selected(), 2);

        state.move_down();
        assert_eq!(state.selected(), 3);

        // Clamp at last node.
        state.move_down();
        assert_eq!(state.selected(), 3);

        state.move_up();
        assert_eq!(state.selected(), 2);

        // Clamp at 0.
        state.move_up();
        state.move_up();
        state.move_up();
        assert_eq!(state.selected(), 0);

        state.move_up();
        assert_eq!(state.selected(), 0);
    }
}
