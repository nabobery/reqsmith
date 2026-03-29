# hurl/components

**UI Component Layer** — implementation of the `Component` trait for TUI panes.

## COMPONENT TRAIT

Defined in `mod.rs`. All panes must implement:

- `handle_key(KeyEvent) -> EventResult`: Process input. Return `Consumed` to stop propagation, `Ignored` to bubble up, or `Action(Action)` to trigger state changes.
- `render(Frame, Rect, focused: bool)`: Draw to terminal. Use `focused` for border styling (Cyan vs DarkGray).
- `focus()` / `blur()`: Lifecycle hooks for state transitions.
- `is_editing() -> bool`: Returns true when in insert mode (suppresses global keys).

## EVENT ROUTING

1. `App::handle_key` receives raw `KeyEvent`.
2. Delegated to `focused_component.handle_key()`.
3. If `Ignored`, `App` checks global shortcuts (Tab for focus cycling, q for quit).
4. If `Action(Action)`, `App` processes the state transition.

## PANES

| File | Component | Responsibility |
|------|-----------|----------------|
| `collections.rs` | `CollectionsPane` | Sidebar navigation. Vim-style `j`/`k` with wrapping. |
| `request_editor.rs` | `RequestEditorPane` | Request editor (headers, body, params). |
| `response_viewer.rs` | `ResponseViewerPane` | Displays response with JSON tree + headers. |
| `json_tree.rs` | `JsonTreeState` | Interactive JSON tree: collapse/expand, scroll, selection. |

## JSON TREE VIEWER

`JsonTreeState` manages a collapsible tree view for JSON responses:

- **State**: `root: JsonValue`, `collapsed: HashSet<String>`, `selected: usize`, `scroll_offset`
- **Keys**: `j`/`k` for navigation, `Enter`/`Space` to toggle, `h` to collapse parent
- **Rendering**: `render_lines(width, height, selected) -> Vec<Line>` with type-based coloring
- **Sorting**: Object keys sorted alphabetically for deterministic display
- **Paths**: Dot-notation paths (`$.user.id`) for tracking collapse state

## UTILITIES

- `status_bar.rs`: Pure function `render_status_bar`. Not a `Component`. Renders global key hints.

## CONVENTIONS

- **Border Color**: Cyan when `focused == true`, DarkGray otherwise.
- **Vim Keys**: Prefer `j`/`k` for vertical movement.
- **Bubbling**: Return `Ignored` for any key not explicitly handled to allow global shortcuts to work.
- **Selection Highlight**: Bold + DarkGray background for selected rows.
