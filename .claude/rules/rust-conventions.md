---
description: Rust coding conventions for claudepot
globs: "**/*.rs"
---

# Rust conventions

## Error handling

- `thiserror` for error types in `claudepot-core`. One enum per module boundary.
- `anyhow` ONLY in `claudepot-cli/main.rs` for the top-level handler.
- Never `unwrap()` or `expect()` in core. Propagate with `?`.
- `unwrap()` is OK only in tests.

## Async

- `tokio` runtime. All I/O operations are async.
- Subprocess calls via `tokio::process::Command`, not `std::process`.
- File I/O can be sync (`std::fs`) for small files (<1 MB).
  Use `tokio::fs` only for large reads (Desktop profile snapshots).

## Security

- NEVER log, print, or include in error messages: access tokens,
  refresh tokens, or any string starting with `sk-ant-`.
  - Sole sanctioned exception: `claudepot cli run <email>
    --print-token` (`cli_ops.rs`, Mode D) prints the access token to
    stdout — explicitly user-requested, pipe-friendly like
    `gh auth token`, preceded by a stderr warning, never logged.
    No other surface may emit a full token; do not extend this
    exception without updating this rule.
- Token values in debug output must be truncated: `sk-ant-oat01-Abc...xyz`.
- Keychain passwords must never appear in source code.
  They come from the environment or user input.
- Before writing any file containing credentials, verify permissions
  will be 0600 (Unix) or user-only ACL (Windows).

## Dependencies

- Use workspace dependencies (defined in root `Cargo.toml`).
- Adding a new dependency requires justification in the commit message.
- Prefer `#[cfg(target_os = "...")]` over runtime OS checks for
  platform-specific code.

## Testing

- Unit tests in the same file (`#[cfg(test)] mod tests`).
- Integration tests that touch the Keychain or filesystem go in
  `tests/` directory and are `#[ignore]` by default (run explicitly
  on test-host test machine).
- Test names: descriptive snake_case behavior sentences, e.g.
  `below_threshold_does_not_fire`, `min_interval_blocks_repeat_fire`.
  The `#[test]` attribute already marks the fn as a test — no
  `test_` prefix required. The older `test_<noun>_<verb>_<scenario>`
  form (e.g. `test_account_add_from_current_success`) remains in
  legacy suites and is acceptable there; don't mass-rename, and
  don't use it for new tests.
