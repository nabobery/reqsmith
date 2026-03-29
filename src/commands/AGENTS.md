# hurl/commands

**CLI Subcommand Implementations** — `run`, `fmt`, `validate`, `list`.

## COMMAND MAP

| Command | File | Sync/Async | Description |
|---------|------|------------|-------------|
| `run` | `run.rs` | async | Execute single `.hurl.yml` with env, vars, output mode |
| `fmt` | `fmt.rs` | sync | Format or check formatting of request files |
| `validate` | `validate.rs` | sync | Validate files with optional environment |
| `list` | `list.rs` | sync | Discover and list `.hurl.yml` files |

## SHARED PATTERNS

- **Path Resolution**: All commands use `resolve_paths()` to expand dirs → `.hurl.yml` files
- **Discovery**: Reuses `repository::discover_requests()` for file tree walking
- **Exit Codes**: Use `crate::core::models::ExitCode` enum values (0=Success, 1=Validation, 2=Internal)
- **Output**: Delegates to `crate::output` module for JSON/YAML/text formatting

## CONVENTIONS

- **Error Handling**: `eprintln!` for user errors, `ExitCode::from(1)` for failures
- **File Collection**: Recursive via `collect_file_paths()` helper in each command
- **Environment**: Commands that need env use `environment::resolve_environment()`

## QUICK START

- **Add a command**: Create `commands/new_cmd.rs`, add `pub mod new_cmd;` to `mod.rs`
- **Wire to CLI**: Add variant to `cli::Command` enum, match in `main.rs::run_command()`
