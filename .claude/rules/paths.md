# Path handling — Windows-aware

Every string that might be a filesystem path is also a Windows path until
proven otherwise. Production code that processes paths must handle the
Windows shape explicitly, and must ship with tests that lock that
behavior down.

## The Windows differences that must be handled

1. **Separator:** `\` on Windows, `/` on Unix. Never hardcode `/` when
   building or splitting a path string.
2. **Drive letters:** `C:\Users\joker\project`. Sanitizes to
   `C--Users-joker-project` (leading alpha + `--` is an unambiguous
   Windows signature — no Unix path can produce it).
3. **UNC paths:** `\\server\share\path`. Sanitizes to
   `--server-share-path`. Ambiguous with a Unix path starting with `-`;
   disambiguate by host OS at unsanitize time.
4. **Verbatim / extended-length prefix:** `std::fs::canonicalize` on
   Windows returns `\\?\C:\...` (or `\\?\UNC\server\share\...`). CC
   *never* writes the verbatim form into session `cwd` or project
   slugs, so feeding a verbatim path into `sanitize_path` produces a
   slug that does not match CC's on-disk directory.

## The rule

- **Anything you feed into `sanitize_path` must first go through
  `claudepot_core::path_utils::simplify_windows_path`.** That function
  strips `\\?\` and rewrites `\\?\UNC\server\share\…` → `\\server\share\…`.
  On non-Windows it is a no-op. The two legitimate callers of
  `std::fs::canonicalize` in production (`project_helpers::resolve_path`,
  `project_memory::find_canonical_git_root`) already do this — any new
  caller must do the same.
- **Never call `canonicalize()` directly in new production code.** Go
  through `resolve_path` or add a thin helper that pairs
  `canonicalize` with `simplify_windows_path`. Tests are free to call
  it directly.
- **`unsanitize_path` is lossy and host-biased.** Prefer the
  authoritative `cwd` from `session.jsonl` (see
  `recover_cwd_from_sessions`). Reach for `unsanitize_path` only when
  no session metadata exists.
- **Never hardcode `/` as a separator.** Use `std::path::MAIN_SEPARATOR`,
  `Path::join`, or a platform branch. If you're working on raw strings
  (logs, slugs, JSON `cwd` fields), remember that CC preserves the
  native separator verbatim — don't normalize it away.
- **Never assume a path starts with `/`.** Windows absolute paths start
  with a drive letter or `\\`. Code that validates "is absolute?" must
  use `Path::is_absolute`, not a `str::starts_with("/")` check.

## TDD discipline for path code

Every change that touches path processing lands with tests. No
exceptions.

- **Pure string ops** (sanitize, unsanitize, verbatim stripping, UNC
  detection) are tested on **all host OSs**. Write golden tests with
  literal expected values. These tests catch drift from CC's algorithm
  and catch Windows regressions on macOS/Linux CI.
- **OS-specific behavior** (the output of `canonicalize`, filesystem
  case-insensitivity, `\` as separator) is tested under
  `#[cfg(target_os = "windows")]` or `#[cfg(unix)]`. CI must run on
  `windows-latest` so these gates actually fire. See
  `.github/workflows/ci.yml` — Windows is in the matrix; keep it there.
- **Red → green, never green first.** Before changing path-processing
  behavior:
  1. Write the failing test with the expected Windows form.
  2. Run the suite and confirm it fails for the reason you expect.
  3. Implement the minimum change to make it pass.
  4. Run the full suite (`cargo test --workspace`) — not just the
     touched module.
- **Cover all four shapes** when adding a new path operation:
  - `/Users/…` — Unix absolute
  - `C:\Users\…` — Windows drive letter
  - `\\server\share\…` — Windows UNC
  - `\\?\C:\…` — Windows verbatim (if `canonicalize` is in the path)
  A missing case is a latent bug. The CC-parity golden tests in
  `project::tests` (search for `test_sanitize_cc_parity_*` and
  `test_sanitize_windows_*`) are the template.

## References

- `crates/claudepot-core/src/path_utils.rs` — the one home for path
  string normalization. Add helpers here, not scattered across call
  sites.
- `crates/claudepot-core/src/project_sanitize.rs` — `sanitize_path` /
  `unsanitize_path`. Must stay in byte-for-byte parity with CC's
  `sessionStoragePortable.ts`.
- `crates/claudepot-core/src/project_helpers.rs::resolve_path` — the
  single canonicalization surface for project paths.
