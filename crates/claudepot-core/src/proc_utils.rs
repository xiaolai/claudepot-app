//! Cross-platform subprocess helpers.
//!
//! On Windows, every `Command::new(...)` that runs a console-mode binary
//! will pop up a visible terminal window unless the process is created
//! with `CREATE_NO_WINDOW` (0x08000000). The Tauri shell itself is marked
//! `windows_subsystem = "windows"` (no console) but child processes
//! inherit the *default* console creation behaviour, so they each get
//! their own window.
//!
//! `NoWindowExt` adds a `.no_window()` builder method to both
//! `std::process::Command` and `tokio::process::Command`. Call it between
//! `Command::new(...)` and `.output()` / `.spawn()` / `.status()` on
//! every background subprocess that should be invisible to the user.

/// Applied value of `CREATE_NO_WINDOW` for Windows. On non-Windows
/// platforms `no_window()` is a no-op.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Extension trait: suppress the console popup on Windows.
pub trait NoWindowExt {
    /// Set `CREATE_NO_WINDOW` on Windows; no-op everywhere else.
    fn no_window(&mut self) -> &mut Self;
}

impl NoWindowExt for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

impl NoWindowExt for tokio::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            // tokio::process::Command exposes creation_flags as an
            // inherent method — no trait import needed.
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}
