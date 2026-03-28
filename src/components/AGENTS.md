# hurl/components

**UI Component Layer** — implementation of the `Component` trait for TUI panes.

## COMPONENT TRAIT

Defined in `mod.rs`. All panes (Collections, Request, Response) must implement:

- `handle_key(KeyEvent) -> EventResult`: Process input. Return `Consumed` to stop propagation, `Ignored` to bubble up, or `Action(Action)` to trigger state changes.
- `render(Frame, Rect, focused: bool)`: Draw to terminal. Use `focused` for border styling (Cyan vs DarkGray).
- `focus()` / `blur()`: Lifecycle hooks for state transitions.

## EVENT ROUTING

1. `App::handle_key` receives raw `KeyEvent`.
2. Delegated to `focused_component.handle_key()`.
3. If `Ignored`, `App` checks global shortcuts (Tab for focus cycling, q for quit).
4. If `Action(Action)`, `App` processes the state transition.

## PANES

| File | Component | Responsibility |
|------|-----------|----------------|
| `collections.rs` | `CollectionsPane` | Sidebar navigation. Vim-style `j`/`k` with wrapping. |
| `request_editor.rs` | `RequestEditorPane` | Placeholder. Currently returns `Ignored`. |
| `response_viewer.rs` | `ResponseViewerPane` | Placeholder. Currently returns `Ignored`. |

## UTILITIES

- `status_bar.rs`: Pure function `render_status_bar`. Not a `Component`. Renders global key hints at screen bottom.

## CONVENTIONS

- **Border Color**: Cyan when `focused == true`, DarkGray otherwise.
- **Vim Keys**: Prefer `j`/`k` for vertical movement.
- **Bubbling**: Return `Ignored` for any key not explicitly handled to allow global shortcuts to work.
