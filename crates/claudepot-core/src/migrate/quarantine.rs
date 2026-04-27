//! macOS `com.apple.quarantine` xattr stripping (§7.6).
//!
//! Stub on non-mac targets. The macOS implementation would call
//! `removexattr(2)` directly via libc; deferred to v0.1 because
//! bundles produced by claudepot do not currently carry the
//! quarantine xattr (we write files via `tempfile` + rename, neither
//! of which sets quarantine).

use std::path::Path;

/// Strip `com.apple.quarantine` from a single file. No-op on non-mac.
pub fn strip(_path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // Deferred: shell out to `/usr/bin/xattr -d com.apple.quarantine`.
        // For v0 the bundle pipeline does not produce files that
        // carry the xattr, so the no-op is correct in practice. When
        // `--include-global` ships with statusline-script support
        // and the user drops in a downloaded `.sh`, this is the hook
        // point.
        let _ = _path;
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    Ok(())
}
