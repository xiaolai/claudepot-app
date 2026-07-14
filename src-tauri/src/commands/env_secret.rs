//! Tauri commands for the `.env` secret-movement surface.
//!
//! Two groups:
//!
//! - **Vault** (`env_vault_*`) — the local named-secret store at
//!   `~/.claudepot/env-vault.db` (0600).
//! - **Per-project `.env`** (`env_file_*`) — list / set / comment /
//!   uncomment / delete / copy-out / inject keys in a project's
//!   `.env*` files.
//!
//! Secret discipline mirrors `commands/keys.rs`: inbound secret args
//! are zeroized on every exit path; outbound secret values cross the
//! bridge *only* by being written to the OS clipboard Rust-side, with
//! the renderer receiving a `KeyCopyReceiptDto` — never the value.
//! Every handler is `async fn` and runs its I/O via `spawn_blocking`.
//!
//! Scrub boundary: every buffer *this module owns* that holds `.env`
//! plaintext — whole-file read buffers (`Zeroizing<String>`), parsed
//! [`EnvLine`] vectors ([`zeroize_lines`]), and single moved values —
//! is scrubbed before drop. Transient allocations inside
//! `claudepot_core::env_vault::env_file`'s line editor (split/join
//! intermediates) are accepted residue; scrubbing them would mean
//! rebuilding the parser around owned-secret types.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use claudepot_core::env_vault::env_file::{self, EnvEditError};
use claudepot_core::env_vault::store::{secret_preview, VaultStore};
use claudepot_core::env_vault::vault_db_path;
use claudepot_core::fs_utils::atomic_write;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;
use zeroize::{Zeroize, Zeroizing};

use super::validate_project_path;
use crate::commands::keys::{
    now_unix_ms, schedule_self_clear, KeyCopyReceiptDto, CLIPBOARD_CLEAR_MS,
};
use crate::dto_env::{entries_from_lines, EnvFileDto, ProjectEnvDto, VaultSecretDto};

const MAX_ENV_FILE_BYTES: u64 = 4 * 1024 * 1024;

fn open_vault() -> Result<VaultStore, String> {
    VaultStore::open(&vault_db_path()).map_err(|e| format!("env vault open failed: {e}"))
}

/// Scrub the owned key/value `String`s inside parsed `.env` lines.
/// A parse result holds *every* secret in the file, not just the one
/// being moved — dropping it unscrubbed would leave more plaintext
/// heap residue than the single value the callers zeroize.
fn zeroize_lines(lines: &mut [env_file::EnvLine]) {
    for line in lines {
        match line {
            env_file::EnvLine::Active { key, value }
            | env_file::EnvLine::Commented { key, value } => {
                key.zeroize();
                value.zeroize();
            }
            env_file::EnvLine::Other(raw) => raw.zeroize(),
        }
    }
}

fn edit_err(e: EnvEditError) -> String {
    e.to_string()
}

fn read_env_content(path: &Path) -> Result<Zeroizing<String>, String> {
    let file = File::open(path).map_err(|e| format!("read {} failed: {e}", path.display()))?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_ENV_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read {} failed: {e}", path.display()))?;
    if bytes.len() as u64 > MAX_ENV_FILE_BYTES {
        return Err(format!(
            "{} is too large for the environment-file editor (maximum {MAX_ENV_FILE_BYTES} bytes)",
            path.display()
        ));
    }
    let content = String::from_utf8(bytes.to_vec())
        .map_err(|_| format!("{} is not valid UTF-8", path.display()))?;
    Ok(Zeroizing::new(content))
}

/// Serialize read-modify-write operations for one dotenv file across both
/// Tauri commands and cooperating Claudepot processes. The sidecar is not a
/// security boundary; it prevents our own concurrent editors from silently
/// losing one another's changes.
fn lock_env_file(path: &Path) -> Result<File, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid dotenv path: {}", path.display()))?;
    let lock_path = path.with_file_name(format!(".{name}.claudepot.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("open dotenv lock {} failed: {e}", lock_path.display()))?;
    file.lock()
        .map_err(|e| format!("lock dotenv file {} failed: {e}", path.display()))?;
    Ok(file)
}

// ─────────────────────────── path safety ───────────────────────────

/// Is `name` a dotenv filename — exactly `.env` or a `.env.<suffix>`
/// variant (`.env.local`, `.env.production`, …)?
///
/// Deliberately rejects other `.env`-prefixed names. `.envrc` in
/// particular is NOT a dotenv file — it is executable direnv shell
/// code, and routing the vault's set/inject writes into it would put
/// secret-bearing lines (or attacker-shaped keys) into a file the
/// shell *runs*. Same for `.environment` and friends: this surface
/// is a dotenv movement layer, nothing else.
fn is_dotenv_file_name(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.")
}

/// Validate a `.env*` file name received from the renderer. The
/// renderer must only ever pass a *bare filename* that one of the
/// `env_file_list` results carried; this rejects anything that could
/// escape the project root (separators, `..`, NUL) or that isn't a
/// dotenv file at all (see [`is_dotenv_file_name`]).
fn safe_env_file_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty .env file name".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') || name.contains("..") {
        return Err(format!("unsafe .env file name: {name:?}"));
    }
    if !is_dotenv_file_name(name) {
        return Err(format!("not a .env file: {name:?}"));
    }
    Ok(())
}

/// `<project_root>/<file_name>`, after validating `file_name` is a
/// safe bare dotenv filename.
fn env_file_path(project_path: &str, file_name: &str) -> Result<PathBuf, String> {
    validate_project_path(project_path)?;
    safe_env_file_name(file_name)?;
    Ok(Path::new(project_path).join(file_name))
}

/// Scan a project root for `.env*` files and parse each into a DTO.
/// Best-effort over directory entries, but a file that exists and
/// can't be read is a hard error (fail loud, don't silently hide a
/// permission problem).
fn scan_env_files(project_path: &str) -> Result<Vec<EnvFileDto>, String> {
    validate_project_path(project_path)?;
    let root = Path::new(project_path);
    let read_dir = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        // Orphan project / unreachable source → no files, not an error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("read project dir failed: {e}")),
    };
    let mut files = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|e| format!("read dir entry failed: {e}"))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("stat dir entry failed: {e}"))?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // Same predicate as the write-path validator — listing a file
        // the mutation commands would reject (e.g. `.envrc`) would
        // offer the user actions that can only fail.
        if !is_dotenv_file_name(&name) {
            continue;
        }
        let path = entry.path();
        // The read buffer and the parsed lines hold every secret in
        // the file; only previews leave this scope. `Zeroizing`
        // scrubs the buffer on drop, `zeroize_lines` the parse.
        let content = read_env_content(&path)?;
        let mut lines = env_file::parse(&content);
        let entries = entries_from_lines(&lines);
        zeroize_lines(&mut lines);
        files.push(EnvFileDto {
            file_name: name,
            path: path.to_string_lossy().into_owned(),
            entries,
        });
    }
    // Stable order so the UI doesn't reshuffle between refreshes.
    files.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(files)
}

fn project_env_dto(project_path: String) -> Result<ProjectEnvDto, String> {
    let files = scan_env_files(&project_path)?;
    Ok(ProjectEnvDto {
        project_path,
        files,
    })
}

/// Read a `.env*` file, apply a line-level edit, write it back at
/// 0600, and return the project's refreshed env view.
fn mutate_env_file(
    project_path: String,
    file_name: String,
    edit: impl FnOnce(&str) -> Result<String, String>,
) -> Result<ProjectEnvDto, String> {
    let path = env_file_path(&project_path, &file_name)?;
    let _lock = lock_env_file(&path)?;
    // The pre-edit buffer holds every secret already in the file —
    // scrub it on drop, same as the freshly-written copy below.
    let content = match read_env_content(&path) {
        Ok(c) => c,
        Err(_e) if !path.exists() => Zeroizing::new(String::new()),
        Err(e) => return Err(e),
    };
    let mut new_content = edit(&content)?;
    let write_result = atomic_write(&path, new_content.as_bytes())
        .map_err(|e| format!("write {} failed: {e}", path.display()));
    // The freshly-written content has the secret value newly placed —
    // scrub our heap copy once it's on disk.
    new_content.zeroize();
    write_result?;
    project_env_dto(project_path)
}

// ─────────────────────────────── vault ──────────────────────────────

/// Every named secret in the local vault. No plaintext crosses.
#[tauri::command]
pub async fn env_vault_list() -> Result<Vec<VaultSecretDto>, String> {
    tokio::task::spawn_blocking(|| {
        let vault = open_vault()?;
        let rows = vault.list().map_err(|e| format!("vault list: {e}"))?;
        Ok::<_, String>(rows.iter().map(VaultSecretDto::from).collect())
    })
    .await
    .map_err(|e| format!("env_vault_list join: {e}"))?
}

/// Add a new named secret. The `secret` arrives over the IPC bridge
/// and is zeroized on every exit path.
#[tauri::command]
pub async fn env_vault_add(name: String, mut secret: String) -> Result<VaultSecretDto, String> {
    let result = {
        let name = name.clone();
        let secret_copy = secret.clone();
        tokio::task::spawn_blocking(move || {
            let vault = open_vault()?;
            let mut buf = secret_copy;
            let outcome = vault
                .insert(&name, &buf)
                .map(|r| VaultSecretDto::from(&r))
                .map_err(|e| format!("vault add: {e}"));
            buf.zeroize();
            outcome
        })
        .await
        .map_err(|e| format!("env_vault_add join: {e}"))?
    };
    secret.zeroize();
    result
}

/// Replace the value of an existing named secret. `secret` zeroized
/// on every exit path.
#[tauri::command]
pub async fn env_vault_update(name: String, mut secret: String) -> Result<VaultSecretDto, String> {
    let result = {
        let name = name.clone();
        let secret_copy = secret.clone();
        tokio::task::spawn_blocking(move || {
            let vault = open_vault()?;
            let mut buf = secret_copy;
            let outcome = vault
                .update(&name, &buf)
                .map(|r| VaultSecretDto::from(&r))
                .map_err(|e| format!("vault update: {e}"));
            buf.zeroize();
            outcome
        })
        .await
        .map_err(|e| format!("env_vault_update join: {e}"))?
    };
    secret.zeroize();
    result
}

/// Delete a named secret from the vault.
#[tauri::command]
pub async fn env_vault_delete(name: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let vault = open_vault()?;
        vault
            .delete(&name)
            .map_err(|e| format!("vault delete: {e}"))
    })
    .await
    .map_err(|e| format!("env_vault_delete join: {e}"))?
}

/// Copy a vault secret to the OS clipboard. The value is fetched and
/// written to the clipboard entirely Rust-side; the renderer receives
/// only a `KeyCopyReceiptDto`. Self-clears after 30s if the clipboard
/// still holds our payload.
#[tauri::command]
pub async fn env_vault_copy(name: String, app: AppHandle) -> Result<KeyCopyReceiptDto, String> {
    let (mut secret, preview) = {
        let name = name.clone();
        tokio::task::spawn_blocking(move || -> Result<(String, String), String> {
            let vault = open_vault()?;
            let record = vault.get(&name).map_err(|e| format!("vault get: {e}"))?;
            let secret = vault
                .reveal(&name)
                .map_err(|e| format!("vault reveal: {e}"))?;
            Ok((secret, record.secret_preview))
        })
        .await
        .map_err(|e| format!("env_vault_copy join: {e}"))??
    };

    if let Err(e) = app.clipboard().write_text(secret.clone()) {
        secret.zeroize();
        return Err(format!("clipboard: {e}"));
    }
    let clears_at = now_unix_ms() + CLIPBOARD_CLEAR_MS;
    schedule_self_clear(app.clone(), secret.clone());
    secret.zeroize();

    Ok(KeyCopyReceiptDto {
        label: name,
        preview,
        clipboard_clears_at_unix_ms: clears_at,
    })
}

// ──────────────────────── per-project .env ──────────────────────────

/// Every `.env*` file in a project root, each parsed into its
/// active / commented-out key entries.
#[tauri::command]
pub async fn env_file_list(project_path: String) -> Result<ProjectEnvDto, String> {
    tokio::task::spawn_blocking(move || project_env_dto(project_path))
        .await
        .map_err(|e| format!("env_file_list join: {e}"))?
}

/// Upsert `key=value` in a project `.env*` file (creates the file if
/// absent). `value` arrives over the IPC bridge and is zeroized on
/// every exit path.
#[tauri::command]
pub async fn env_file_set(
    project_path: String,
    file_name: String,
    key: String,
    mut value: String,
) -> Result<ProjectEnvDto, String> {
    let result = {
        let value_copy = value.clone();
        tokio::task::spawn_blocking(move || {
            let mut buf = value_copy;
            let out = mutate_env_file(project_path, file_name, |content| {
                env_file::set(content, &key, &buf).map_err(edit_err)
            });
            buf.zeroize();
            out
        })
        .await
        .map_err(|e| format!("env_file_set join: {e}"))?
    };
    value.zeroize();
    result
}

/// Comment out the active line for `key` (the value stays on disk,
/// inactive).
#[tauri::command]
pub async fn env_file_comment(
    project_path: String,
    file_name: String,
    key: String,
) -> Result<ProjectEnvDto, String> {
    tokio::task::spawn_blocking(move || {
        mutate_env_file(project_path, file_name, |content| {
            env_file::comment(content, &key).map_err(edit_err)
        })
    })
    .await
    .map_err(|e| format!("env_file_comment join: {e}"))?
}

/// Uncomment the commented-out line for `key`, making it active again.
#[tauri::command]
pub async fn env_file_uncomment(
    project_path: String,
    file_name: String,
    key: String,
) -> Result<ProjectEnvDto, String> {
    tokio::task::spawn_blocking(move || {
        mutate_env_file(project_path, file_name, |content| {
            env_file::uncomment(content, &key).map_err(edit_err)
        })
    })
    .await
    .map_err(|e| format!("env_file_uncomment join: {e}"))?
}

/// Delete `key`'s line — active or commented — from a project
/// `.env*` file.
#[tauri::command]
pub async fn env_file_delete(
    project_path: String,
    file_name: String,
    key: String,
) -> Result<ProjectEnvDto, String> {
    tokio::task::spawn_blocking(move || {
        mutate_env_file(project_path, file_name, |content| {
            env_file::delete(content, &key).map_err(edit_err)
        })
    })
    .await
    .map_err(|e| format!("env_file_delete join: {e}"))?
}

/// Copy a `.env*` entry's value to the OS clipboard. Reads the value
/// Rust-side, writes it to the clipboard, returns a
/// `KeyCopyReceiptDto` — the value never reaches the renderer.
/// Prefers the active line; falls back to a commented-out one.
#[tauri::command]
pub async fn env_file_copy_value(
    project_path: String,
    file_name: String,
    key: String,
    app: AppHandle,
) -> Result<KeyCopyReceiptDto, String> {
    let (mut value, preview) = {
        let key = key.clone();
        tokio::task::spawn_blocking(move || -> Result<(String, String), String> {
            let path = env_file_path(&project_path, &file_name)?;
            // The read buffer and the parse hold every secret in the
            // file, not just the one being copied — scrub both before
            // this closure returns (`Zeroizing` on drop for the
            // buffer, `zeroize_lines` for the parse).
            let content = Zeroizing::new(
                std::fs::read_to_string(&path)
                    .map_err(|e| format!("read {} failed: {e}", path.display()))?,
            );
            let mut lines = env_file::parse(&content);
            // Active first, then commented — copy-out works for either.
            let value = lines
                .iter()
                .find_map(|l| match l {
                    env_file::EnvLine::Active { key: k, value } if *k == key => Some(value.clone()),
                    _ => None,
                })
                .or_else(|| {
                    lines.iter().find_map(|l| match l {
                        env_file::EnvLine::Commented { key: k, value } if *k == key => {
                            Some(value.clone())
                        }
                        _ => None,
                    })
                });
            zeroize_lines(&mut lines);
            let value = value.ok_or_else(|| format!("no `{key}` in {file_name}"))?;
            let preview = secret_preview(&value);
            Ok((value, preview))
        })
        .await
        .map_err(|e| format!("env_file_copy_value join: {e}"))??
    };

    if let Err(e) = app.clipboard().write_text(value.clone()) {
        value.zeroize();
        return Err(format!("clipboard: {e}"));
    }
    let clears_at = now_unix_ms() + CLIPBOARD_CLEAR_MS;
    schedule_self_clear(app.clone(), value.clone());
    value.zeroize();

    Ok(KeyCopyReceiptDto {
        label: key,
        preview,
        clipboard_clears_at_unix_ms: clears_at,
    })
}

/// Inject a vault secret into a project `.env*` file: the secret is
/// revealed Rust-side and written as `vault_name=<secret>` via the
/// line-level `set` (creating the file if absent). The plaintext
/// never reaches the renderer.
#[tauri::command]
pub async fn env_file_inject(
    project_path: String,
    file_name: String,
    vault_name: String,
) -> Result<ProjectEnvDto, String> {
    tokio::task::spawn_blocking(move || {
        let vault = open_vault()?;
        let mut secret = vault
            .reveal(&vault_name)
            .map_err(|e| format!("vault reveal: {e}"))?;
        let out = mutate_env_file(project_path, file_name, |content| {
            env_file::set(content, &vault_name, &secret).map_err(edit_err)
        });
        secret.zeroize();
        out
    })
    .await
    .map_err(|e| format!("env_file_inject join: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── safe_env_file_name — renderer trust-boundary validator ─────
    // Table tests in the style of core's `validate_slug` traversal
    // tests: every rejection class and every accepted dotenv shape.

    #[test]
    fn test_safe_env_file_name_accepts_dotenv_shapes() {
        for ok in [
            ".env",
            ".env.local",
            ".env.production",
            ".env.test",
            ".env.development.local",
            // Unicode in the suffix is harmless — no separators, so
            // `Path::join` cannot escape the project root.
            ".env.ünïcode",
        ] {
            assert!(safe_env_file_name(ok).is_ok(), "should accept {ok:?}");
        }
    }

    #[test]
    fn test_safe_env_file_name_rejects_empty() {
        assert!(safe_env_file_name("").is_err());
    }

    #[test]
    fn test_safe_env_file_name_rejects_separators_and_traversal() {
        for bad in [
            "a/b",
            "a\\b",
            "..",
            "../.env",
            "..\\.env",
            "sub/.env",
            "sub\\.env",
            ".env/..",
            ".env/../../x",
            // `..` embedded in an otherwise dotenv-shaped name —
            // the traversal check fires before the shape check.
            ".env..",
            // Unicode fraction-slash is NOT a path separator, but the
            // embedded `..` still rejects it.
            ".env\u{2044}..\u{2044}x",
        ] {
            assert!(safe_env_file_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn test_safe_env_file_name_rejects_nul() {
        assert!(safe_env_file_name(".env\0").is_err());
        assert!(safe_env_file_name(".env.\0local").is_err());
    }

    #[test]
    fn test_safe_env_file_name_rejects_non_dotenv_shapes() {
        for bad in ["env", "env.local", "x.env", ".environ", ".env2"] {
            assert!(safe_env_file_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    /// `.envrc` is direnv shell code that gets *executed* — admitting
    /// it would let the vault inject secret-bearing lines into a file
    /// the shell runs. Explicit decision test, not just a shape case.
    #[test]
    fn test_safe_env_file_name_rejects_envrc() {
        assert!(safe_env_file_name(".envrc").is_err());
    }

    #[test]
    fn test_is_dotenv_file_name_matches_validator_shape() {
        assert!(is_dotenv_file_name(".env"));
        assert!(is_dotenv_file_name(".env.local"));
        assert!(!is_dotenv_file_name(".envrc"));
        assert!(!is_dotenv_file_name(".environment"));
        assert!(!is_dotenv_file_name("env"));
    }

    // ── scan filter parity ──────────────────────────────────────────
    // The directory listing and the write-path validator must agree:
    // a file the list shows must be mutable, and `.envrc` must never
    // surface in the UI at all.
    #[test]
    fn test_scan_env_files_skips_envrc_and_lists_dotenv() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "A=1\n").unwrap();
        std::fs::write(tmp.path().join(".env.local"), "B=2\n").unwrap();
        std::fs::write(tmp.path().join(".envrc"), "export C=3\n").unwrap();

        let files = scan_env_files(tmp.path().to_str().unwrap()).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.file_name.as_str()).collect();
        assert_eq!(names, vec![".env", ".env.local"]);
    }

    #[test]
    fn test_read_env_content_rejects_oversized_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".env");
        std::fs::write(&path, vec![b'x'; (MAX_ENV_FILE_BYTES + 1) as usize]).unwrap();

        let error = read_env_content(&path).unwrap_err();
        assert!(error.contains("too large"));
    }
}
