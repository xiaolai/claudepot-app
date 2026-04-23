//! Editor detection + launcher for the Config section "Open with…" picker.
//!
//! Per `dev-docs/config-section-plan.md` §12.5: probe for installed
//! editors (VS Code, Cursor, Sublime, Zed, Xcode, VMark, JetBrains,
//! TextMate, BBEdit, Nova, Neovim, Vim, Emacs, Notepad++), plus
//! `$EDITOR` and the OS default handler. Results are cached for 5
//! minutes; `/` refresh forces a re-probe.
//!
//! Detection runs OS-dependent probes:
//! - **macOS**: `/Applications/*.app` existence + `which <cli>` +
//!   JetBrains Toolbox scripts.
//! - **Linux**: `which <cli>` first; no `.desktop` scan in P0.
//! - **Windows**: `where <cli>` + well-known install paths.
//!
//! The launcher never waits for the editor process — spawn is
//! fire-and-forget. Spawn errors are returned; callers surface as a toast.

use crate::config_view::model::{
    DetectSource, EditorCandidate, EditorDefaults, Kind, LaunchKind,
};
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ---------- Probe targets --------------------------------------------

/// A single probe entry — what we look for on each OS.
struct ProbeEntry {
    id: &'static str,
    label: &'static str,
    bundle_id: Option<&'static str>,
    macos_app: Option<&'static str>,
    cli_names: &'static [&'static str],
    args_template: &'static [&'static str],
    supports_kinds: Option<&'static [Kind]>,
}

/// `{path}` is substituted with the absolute file path at launch time.
const PROBES: &[ProbeEntry] = &[
    ProbeEntry {
        id: "vscode",
        label: "Visual Studio Code",
        bundle_id: Some("com.microsoft.VSCode"),
        macos_app: Some("Visual Studio Code.app"),
        cli_names: &["code"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "vscode-insiders",
        label: "VS Code Insiders",
        bundle_id: Some("com.microsoft.VSCodeInsiders"),
        macos_app: Some("Visual Studio Code - Insiders.app"),
        cli_names: &["code-insiders"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "cursor",
        label: "Cursor",
        bundle_id: Some("com.todesktop.230313mzl4w4u92"),
        macos_app: Some("Cursor.app"),
        cli_names: &["cursor"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "sublime",
        label: "Sublime Text",
        bundle_id: Some("com.sublimetext.4"),
        macos_app: Some("Sublime Text.app"),
        cli_names: &["subl"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "zed",
        label: "Zed",
        bundle_id: Some("dev.zed.Zed"),
        macos_app: Some("Zed.app"),
        cli_names: &["zed"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "xcode",
        label: "Xcode",
        bundle_id: Some("com.apple.dt.Xcode"),
        macos_app: Some("Xcode.app"),
        cli_names: &[],
        args_template: &[],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "vmark",
        label: "VMark",
        bundle_id: None,
        macos_app: Some("VMark.app"),
        cli_names: &["vmark"],
        args_template: &["{path}"],
        supports_kinds: Some(&[Kind::ClaudeMd, Kind::Memory, Kind::MemoryIndex]),
    },
    ProbeEntry {
        id: "jetbrains:idea",
        label: "IntelliJ IDEA",
        bundle_id: Some("com.jetbrains.intellij"),
        macos_app: Some("IntelliJ IDEA.app"),
        cli_names: &["idea"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "jetbrains:goland",
        label: "GoLand",
        bundle_id: Some("com.jetbrains.goland"),
        macos_app: Some("GoLand.app"),
        cli_names: &["goland"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "jetbrains:pycharm",
        label: "PyCharm",
        bundle_id: Some("com.jetbrains.pycharm"),
        macos_app: Some("PyCharm.app"),
        cli_names: &["pycharm"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "jetbrains:webstorm",
        label: "WebStorm",
        bundle_id: Some("com.jetbrains.WebStorm"),
        macos_app: Some("WebStorm.app"),
        cli_names: &["webstorm"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "jetbrains:rustrover",
        label: "RustRover",
        bundle_id: Some("com.jetbrains.rustrover"),
        macos_app: Some("RustRover.app"),
        cli_names: &["rustrover"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "textmate",
        label: "TextMate",
        bundle_id: Some("com.macromates.TextMate"),
        macos_app: Some("TextMate.app"),
        cli_names: &["mate"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "bbedit",
        label: "BBEdit",
        bundle_id: Some("com.barebones.bbedit"),
        macos_app: Some("BBEdit.app"),
        cli_names: &["bbedit"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "nova",
        label: "Nova",
        bundle_id: Some("com.panic.Nova"),
        macos_app: Some("Nova.app"),
        cli_names: &[],
        args_template: &[],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "nvim",
        label: "Neovim",
        bundle_id: None,
        macos_app: None,
        cli_names: &["nvim"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "vim",
        label: "Vim",
        bundle_id: None,
        macos_app: None,
        cli_names: &["vim"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "emacs",
        label: "Emacs",
        bundle_id: None,
        macos_app: None,
        cli_names: &["emacs"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
    ProbeEntry {
        id: "notepadpp",
        label: "Notepad++",
        bundle_id: None,
        macos_app: None,
        cli_names: &["notepad++"],
        args_template: &["{path}"],
        supports_kinds: None,
    },
];

// ---------- Probe backend trait (swappable for tests) -----------------

pub trait ProbeBackend: Send + Sync {
    /// Return the path to `name` on the $PATH, if found.
    fn which(&self, name: &str) -> Option<PathBuf>;
    /// Return true if `path` exists and is a directory (for `.app` bundles).
    fn dir_exists(&self, path: &Path) -> bool;
    /// `$EDITOR` / `$VISUAL` value if set.
    fn env_editor(&self) -> Option<String>;
}

pub struct RealProbe;

impl ProbeBackend for RealProbe {
    fn which(&self, name: &str) -> Option<PathBuf> {
        use std::process::Command;
        let cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
        let out = Command::new(cmd).arg(name).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout);
        let first = s.lines().next()?.trim();
        if first.is_empty() {
            return None;
        }
        Some(PathBuf::from(first))
    }
    fn dir_exists(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn env_editor(&self) -> Option<String> {
        std::env::var("EDITOR")
            .ok()
            .or_else(|| std::env::var("VISUAL").ok())
            .filter(|s| !s.trim().is_empty())
    }
}

// ---------- Detection -------------------------------------------------

/// Run all probes and produce the full candidate list.
pub fn detect<B: ProbeBackend>(backend: &B) -> Vec<EditorCandidate> {
    let mut out: Vec<EditorCandidate> = Vec::new();

    for p in PROBES {
        if let Some(c) = probe_entry(p, backend) {
            out.push(c);
        }
    }

    if let Some(editor) = backend.env_editor() {
        out.push(EditorCandidate {
            id: "env".to_string(),
            label: format!("$EDITOR ({})", editor),
            binary_path: None,
            bundle_id: None,
            launch: LaunchKind::EnvEditor,
            detected_via: DetectSource::EnvVar { name: "EDITOR".to_string() },
            supports_kinds: None,
        });
    }

    // System default is always available.
    out.push(EditorCandidate {
        id: "system".to_string(),
        label: "System default".to_string(),
        binary_path: None,
        bundle_id: None,
        launch: LaunchKind::SystemHandler,
        detected_via: DetectSource::SystemDefault,
        supports_kinds: None,
    });

    out
}

fn probe_entry<B: ProbeBackend>(p: &ProbeEntry, backend: &B) -> Option<EditorCandidate> {
    // macOS: prefer .app bundle detection (`open -a`); fall back to CLI.
    #[cfg(target_os = "macos")]
    if let Some(app) = p.macos_app {
        let bundle_path = PathBuf::from("/Applications").join(app);
        if backend.dir_exists(&bundle_path) {
            return Some(EditorCandidate {
                id: p.id.to_string(),
                label: p.label.to_string(),
                binary_path: None,
                bundle_id: p.bundle_id.map(String::from),
                launch: LaunchKind::MacosOpenA {
                    app_name: app.trim_end_matches(".app").to_string(),
                },
                detected_via: DetectSource::MacosAppBundle { path: bundle_path },
                supports_kinds: p.supports_kinds.map(|ks| ks.to_vec()),
            });
        }
    }

    // Fall back to CLI: try each candidate name.
    for cli in p.cli_names {
        if let Some(bin) = backend.which(cli) {
            return Some(EditorCandidate {
                id: p.id.to_string(),
                label: p.label.to_string(),
                binary_path: Some(bin.clone()),
                bundle_id: p.bundle_id.map(String::from),
                launch: LaunchKind::Direct {
                    args_template: p.args_template.iter().map(|s| s.to_string()).collect(),
                },
                detected_via: DetectSource::PathBinary { which: bin },
                supports_kinds: p.supports_kinds.map(|ks| ks.to_vec()),
            });
        }
    }

    None
}

// ---------- Cache -----------------------------------------------------

struct CacheEntry {
    at: Instant,
    candidates: Vec<EditorCandidate>,
}

static CACHE: Lazy<Mutex<Option<CacheEntry>>> = Lazy::new(|| Mutex::new(None));

const CACHE_TTL: Duration = Duration::from_secs(300);

/// Detect-with-cache. Re-runs detection after `CACHE_TTL` or when
/// `force` is true.
pub fn detect_cached(force: bool) -> Vec<EditorCandidate> {
    {
        let guard = CACHE.lock().unwrap();
        if !force {
            if let Some(ref e) = *guard {
                if e.at.elapsed() < CACHE_TTL {
                    return e.candidates.clone();
                }
            }
        }
    }
    let fresh = detect(&RealProbe);
    let mut guard = CACHE.lock().unwrap();
    *guard = Some(CacheEntry { at: Instant::now(), candidates: fresh.clone() });
    fresh
}

#[cfg(test)]
pub fn clear_cache_for_test() {
    *CACHE.lock().unwrap() = None;
}

// ---------- Launch ----------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("$EDITOR is not set")]
    NoEnvEditor,
    #[error("editor binary path missing")]
    NoBinary,
    #[error("path is empty")]
    EmptyPath,
    #[error("editor not found by id: {0}")]
    UnknownEditor(String),
}

/// Fire-and-forget launch. Returns immediately on successful spawn.
pub fn invoke(editor: &EditorCandidate, path: &Path) -> Result<(), LaunchError> {
    if path.as_os_str().is_empty() {
        return Err(LaunchError::EmptyPath);
    }
    match &editor.launch {
        LaunchKind::Direct { args_template } => {
            let bin = editor.binary_path.as_ref().ok_or(LaunchError::NoBinary)?;
            let args: Vec<String> = args_template
                .iter()
                .map(|t| t.replace("{path}", &path.display().to_string()))
                .collect();
            std::process::Command::new(bin)
                .args(&args)
                .spawn()
                .map_err(|e| LaunchError::Spawn(e.to_string()))?;
            Ok(())
        }
        LaunchKind::MacosOpenA { app_name } => {
            std::process::Command::new("/usr/bin/open")
                .arg("-a")
                .arg(app_name)
                .arg(path)
                .spawn()
                .map_err(|e| LaunchError::Spawn(e.to_string()))?;
            Ok(())
        }
        LaunchKind::EnvEditor => {
            let editor = std::env::var("EDITOR")
                .ok()
                .or_else(|| std::env::var("VISUAL").ok())
                .filter(|s| !s.trim().is_empty())
                .ok_or(LaunchError::NoEnvEditor)?;
            let mut parts = editor.split_whitespace();
            let bin = parts.next().ok_or(LaunchError::NoEnvEditor)?;
            let mut cmd = std::process::Command::new(bin);
            cmd.args(parts).arg(path);
            cmd.spawn().map_err(|e| LaunchError::Spawn(e.to_string()))?;
            Ok(())
        }
        LaunchKind::SystemHandler => {
            #[cfg(target_os = "macos")]
            {
                std::process::Command::new("/usr/bin/open")
                    .arg(path)
                    .spawn()
                    .map_err(|e| LaunchError::Spawn(e.to_string()))?;
            }
            #[cfg(target_os = "linux")]
            {
                std::process::Command::new("xdg-open")
                    .arg(path)
                    .spawn()
                    .map_err(|e| LaunchError::Spawn(e.to_string()))?;
            }
            #[cfg(target_os = "windows")]
            {
                let p = path.to_str().ok_or_else(|| {
                    LaunchError::Spawn("path is not valid UTF-8".to_string())
                })?;
                std::process::Command::new("cmd")
                    .args(["/C", "start", "", p])
                    .spawn()
                    .map_err(|e| LaunchError::Spawn(e.to_string()))?;
            }
            Ok(())
        }
    }
}

// ---------- Per-kind default resolution -------------------------------

/// Resolve which editor should launch for `kind` given `defaults`
/// and the detected `candidates`. Precedence:
///
/// 1. `defaults.by_kind[kind]` if still present in candidates.
/// 2. Any detected editor with `supports_kinds` containing `kind`,
///    preferring the `defaults.fallback` id.
/// 3. `defaults.fallback` if present.
/// 4. `EditorCandidate` with `id == "system"` (always present).
pub fn resolve_editor_for<'a>(
    kind: &Kind,
    defaults: &EditorDefaults,
    candidates: &'a [EditorCandidate],
) -> Option<&'a EditorCandidate> {
    if let Some(id) = defaults.by_kind.get(kind) {
        if let Some(c) = candidates.iter().find(|c| &c.id == id) {
            return Some(c);
        }
    }
    let matches_kind: Vec<&EditorCandidate> = candidates
        .iter()
        .filter(|c| c.supports_kinds.as_ref().is_some_and(|ks| ks.contains(kind)))
        .collect();
    if !matches_kind.is_empty() {
        if let Some(c) = matches_kind.iter().find(|c| c.id == defaults.fallback) {
            return Some(*c);
        }
        return matches_kind.into_iter().next();
    }
    if let Some(c) = candidates.iter().find(|c| c.id == defaults.fallback) {
        return Some(c);
    }
    candidates.iter().find(|c| c.id == "system")
}

// ---------- Tests -----------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Stub backend: deterministic answers for tests.
    struct StubBackend {
        apps: Vec<&'static str>,
        bins: HashMap<&'static str, PathBuf>,
        env_editor: Option<String>,
    }

    impl ProbeBackend for StubBackend {
        fn which(&self, name: &str) -> Option<PathBuf> {
            self.bins.get(name).cloned()
        }
        fn dir_exists(&self, path: &Path) -> bool {
            self.apps.iter().any(|app| {
                path == Path::new("/Applications").join(app)
            })
        }
        fn env_editor(&self) -> Option<String> {
            self.env_editor.clone()
        }
    }

    #[test]
    fn detect_always_includes_system_default() {
        let backend = StubBackend {
            apps: vec![],
            bins: HashMap::new(),
            env_editor: None,
        };
        let cands = detect(&backend);
        assert!(
            cands.iter().any(|c| c.id == "system"),
            "system default must always be present"
        );
        // Only "system" when nothing else is detected.
        assert_eq!(cands.len(), 1);
    }

    #[test]
    fn detect_env_editor_when_set() {
        let backend = StubBackend {
            apps: vec![],
            bins: HashMap::new(),
            env_editor: Some("vim".to_string()),
        };
        let cands = detect(&backend);
        assert!(cands.iter().any(|c| c.id == "env"));
    }

    #[test]
    fn detect_cli_only_on_linux_or_missing_app() {
        let mut bins = HashMap::new();
        bins.insert("code", PathBuf::from("/usr/local/bin/code"));
        let backend = StubBackend { apps: vec![], bins, env_editor: None };
        let cands = detect(&backend);
        // On macOS, VS Code app bundle is absent in the stub → falls back
        // to CLI. On Linux/Windows, the CLI is the only path.
        assert!(cands.iter().any(|c| c.id == "vscode"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn detect_prefers_app_bundle_on_macos() {
        let mut bins = HashMap::new();
        bins.insert("code", PathBuf::from("/usr/local/bin/code"));
        let backend = StubBackend {
            apps: vec!["Visual Studio Code.app"],
            bins,
            env_editor: None,
        };
        let cands = detect(&backend);
        let vscode = cands.iter().find(|c| c.id == "vscode").unwrap();
        match &vscode.launch {
            LaunchKind::MacosOpenA { app_name } => {
                assert_eq!(app_name, "Visual Studio Code");
            }
            other => panic!("expected MacosOpenA, got {:?}", other),
        }
    }

    #[test]
    fn resolve_editor_falls_back_to_system() {
        let defaults = EditorDefaults::new();
        let cands = vec![EditorCandidate {
            id: "system".to_string(),
            label: "System default".to_string(),
            binary_path: None,
            bundle_id: None,
            launch: LaunchKind::SystemHandler,
            detected_via: DetectSource::SystemDefault,
            supports_kinds: None,
        }];
        let r = resolve_editor_for(&Kind::Settings, &defaults, &cands).unwrap();
        assert_eq!(r.id, "system");
    }

    #[test]
    fn resolve_editor_honors_per_kind_default() {
        let mut defaults = EditorDefaults::new();
        defaults.by_kind.insert(Kind::ClaudeMd, "vmark".to_string());
        let cands = vec![
            EditorCandidate {
                id: "vmark".to_string(),
                label: "VMark".to_string(),
                binary_path: None,
                bundle_id: None,
                launch: LaunchKind::MacosOpenA { app_name: "VMark".to_string() },
                detected_via: DetectSource::SystemDefault,
                supports_kinds: Some(vec![Kind::ClaudeMd]),
            },
            EditorCandidate {
                id: "system".to_string(),
                label: "System default".to_string(),
                binary_path: None,
                bundle_id: None,
                launch: LaunchKind::SystemHandler,
                detected_via: DetectSource::SystemDefault,
                supports_kinds: None,
            },
        ];
        let r = resolve_editor_for(&Kind::ClaudeMd, &defaults, &cands).unwrap();
        assert_eq!(r.id, "vmark");
    }

    #[test]
    fn resolve_editor_skips_missing_by_kind_default() {
        let mut defaults = EditorDefaults::new();
        // User's saved default no longer present on this machine.
        defaults.by_kind.insert(Kind::Settings, "cursor".to_string());
        defaults.fallback = "system".to_string();
        let cands = vec![EditorCandidate {
            id: "system".to_string(),
            label: "System default".to_string(),
            binary_path: None,
            bundle_id: None,
            launch: LaunchKind::SystemHandler,
            detected_via: DetectSource::SystemDefault,
            supports_kinds: None,
        }];
        let r = resolve_editor_for(&Kind::Settings, &defaults, &cands).unwrap();
        assert_eq!(r.id, "system");
    }

    #[test]
    fn resolve_editor_prefers_kind_capable_when_no_saved_default() {
        let defaults = EditorDefaults::new();
        let cands = vec![
            EditorCandidate {
                id: "vmark".to_string(),
                label: "VMark".to_string(),
                binary_path: None,
                bundle_id: None,
                launch: LaunchKind::MacosOpenA { app_name: "VMark".to_string() },
                detected_via: DetectSource::SystemDefault,
                supports_kinds: Some(vec![Kind::ClaudeMd]),
            },
            EditorCandidate {
                id: "vscode".to_string(),
                label: "Visual Studio Code".to_string(),
                binary_path: None,
                bundle_id: None,
                launch: LaunchKind::MacosOpenA { app_name: "Visual Studio Code".to_string() },
                detected_via: DetectSource::SystemDefault,
                supports_kinds: None,
            },
            EditorCandidate {
                id: "system".to_string(),
                label: "System default".to_string(),
                binary_path: None,
                bundle_id: None,
                launch: LaunchKind::SystemHandler,
                detected_via: DetectSource::SystemDefault,
                supports_kinds: None,
            },
        ];
        let r = resolve_editor_for(&Kind::ClaudeMd, &defaults, &cands).unwrap();
        assert_eq!(r.id, "vmark");
    }

    #[test]
    fn invoke_empty_path_errors() {
        let cand = EditorCandidate {
            id: "system".to_string(),
            label: "System default".to_string(),
            binary_path: None,
            bundle_id: None,
            launch: LaunchKind::SystemHandler,
            detected_via: DetectSource::SystemDefault,
            supports_kinds: None,
        };
        let err = invoke(&cand, Path::new("")).unwrap_err();
        assert!(matches!(err, LaunchError::EmptyPath));
    }
}
