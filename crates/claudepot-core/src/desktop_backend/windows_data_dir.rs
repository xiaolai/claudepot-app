//! Where Claude Desktop keeps its user data on Windows (github#108).
//!
//! Desktop used to keep it in the MSIX package's virtualised AppData,
//! `%LOCALAPPDATA%\Packages\<family>\LocalCache\Roaming\Claude`. It now
//! moves it out to `%LOCALAPPDATA%\Claude-Data\<family, lowercased>` and
//! leaves a `.user-data-left.<family, lowercased>` marker behind in the
//! old folder. Verified in Desktop 4.4212.0's `app.asar` (`BWe`, `hWe`,
//! `aUe`, `KUe`, `HUe`, `sWe`): a packaged install uses the moved folder
//! when it exists, no `back` marker is present, and either the old
//! folder holds the `left` marker or it holds nothing but markers (or
//! is gone). It never moves when `%LOCALAPPDATA%` starts with `\\`.
//!
//! Not modelled, each a case where Claudepot names the package copy and
//! Desktop may use another folder — the error then says "keeps its data
//! elsewhere", which stays true:
//!
//! - `%LOCALAPPDATA%\Claude-Data` itself (`dataDir`), used only when
//!   `%APPDATA%` starts with `\\`;
//! - `%APPDATA%\Claude` (`squirrel1p`), Desktop's last fallback;
//! - the birthtime and whole-folder branches of `hWe`, and a moved
//!   folder that is a link onto a share.
//!
//! Compiled on every host under `test` so the selection logic runs in
//! macOS and Linux CI too; only the Windows backend calls it.

use std::path::{Path, PathBuf};

/// Desktop's `iUe`: the moved folder is `<LOCALAPPDATA>\Claude-Data`.
const MOVED_ROOT: &str = "Claude-Data";

/// Desktop's `KUe`: the family, lowercased, if it has the shape
/// `<name>_<publisher id>` (`[a-z0-9.-]+_[a-z0-9]+`). Desktop names the
/// moved folder and the markers with this form, and refuses to move at
/// all when the family fails the check.
pub(crate) fn normalized_family(family: &str) -> Option<String> {
    let lower = family.to_ascii_lowercase();
    let (name, publisher) = lower.rsplit_once('_')?;
    let name_ok = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    let publisher_ok = !publisher.is_empty()
        && publisher
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    (name_ok && publisher_ok).then_some(lower)
}

/// The package copy: where Desktop kept its data before the move, and
/// where it still keeps it when the move has not happened.
pub(crate) fn package_copy(local_app_data: &Path, family: &str) -> PathBuf {
    local_app_data
        .join("Packages")
        .join(family)
        .join("LocalCache")
        .join("Roaming")
        .join("Claude")
}

/// Desktop's `aUe`: a `%LOCALAPPDATA%` that starts with `\\` is taken
/// to be on a share, and Desktop never moves its data off the package
/// there. A string test, so a verbatim `\\?\` prefix counts too.
fn is_share(local_app_data: &Path) -> bool {
    local_app_data.to_string_lossy().starts_with(r"\\")
}

/// The data directory Desktop is using, given `%LOCALAPPDATA%` and the
/// package family.
///
/// Returns the moved folder when Desktop has recorded the move as done,
/// otherwise the package copy. It never returns a directory Desktop
/// would not use, so a missing `config.json` under the result still
/// means what it says.
pub(crate) fn resolve(local_app_data: &Path, family: &str) -> PathBuf {
    let package = package_copy(local_app_data, family);
    if is_share(local_app_data) {
        return package;
    }
    let Some(lower) = normalized_family(family) else {
        return package;
    };
    let marker = |state: &str| package.join(format!(".user-data-{state}.{lower}"));
    let moved = local_app_data.join(MOVED_ROOT).join(&lower);
    // `back` means Desktop moved the data back into the package.
    if !moved.is_dir() || marker("back").exists() {
        return package;
    }
    if marker("left").exists() || holds_nothing(&package) {
        return moved;
    }
    package
}

/// Desktop's `sWe(...) === "nothing"`: the package copy is missing, or
/// holds only Desktop's own `.user-data-*` markers. Desktop then counts
/// the move as done even without a `left` marker. An unreadable folder
/// is not "nothing": it may hold the data.
fn holds_nothing(package: &Path) -> bool {
    match std::fs::read_dir(package) {
        Ok(entries) => entries.into_iter().all(|entry| {
            entry.is_ok_and(|e| e.file_name().to_string_lossy().starts_with(".user-data-"))
        }),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const FAMILY: &str = "Claude_pzs8sxrjxfjjc";

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn normalized_family_lowercases_both_known_families() {
        assert_eq!(
            normalized_family("Claude_pzs8sxrjxfjjc").as_deref(),
            Some("claude_pzs8sxrjxfjjc")
        );
        assert_eq!(
            normalized_family("AnthropicPBC.Claude_fnn82j28hfe8t").as_deref(),
            Some("anthropicpbc.claude_fnn82j28hfe8t")
        );
    }

    #[test]
    fn normalized_family_rejects_what_desktop_rejects() {
        for bad in [
            "",
            "Claude",
            "_abc",
            "Claude_",
            "Claude_ab c",
            "Cla ude_abc",
            "Claude_a.b",
            "..\\x_abc",
        ] {
            assert_eq!(normalized_family(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn moved_folder_is_used_when_desktop_left_the_package() {
        // The reporter's layout in github#108: the package copy holds
        // only the `left` marker, the data lives in Claude-Data.
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        touch(&package_copy(local, FAMILY).join(".user-data-left.claude_pzs8sxrjxfjjc"));
        let moved = local.join("Claude-Data").join("claude_pzs8sxrjxfjjc");
        touch(&moved.join("config.json"));

        assert_eq!(resolve(local, FAMILY), moved);
    }

    #[test]
    fn package_copy_is_used_before_any_move() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        let package = package_copy(local, FAMILY);
        touch(&package.join("config.json"));
        // A moved folder without the marker is not Desktop's: it only
        // uses Claude-Data after recording the move.
        fs::create_dir_all(local.join("Claude-Data").join("claude_pzs8sxrjxfjjc")).unwrap();

        assert_eq!(resolve(local, FAMILY), package);
    }

    #[test]
    fn back_marker_returns_to_the_package_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        let package = package_copy(local, FAMILY);
        touch(&package.join(".user-data-left.claude_pzs8sxrjxfjjc"));
        touch(&package.join(".user-data-back.claude_pzs8sxrjxfjjc"));
        fs::create_dir_all(local.join("Claude-Data").join("claude_pzs8sxrjxfjjc")).unwrap();

        assert_eq!(resolve(local, FAMILY), package);
    }

    #[test]
    fn left_marker_without_a_moved_folder_stays_in_the_package() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        let package = package_copy(local, FAMILY);
        touch(&package.join(".user-data-left.claude_pzs8sxrjxfjjc"));

        assert_eq!(resolve(local, FAMILY), package);
    }

    #[test]
    fn marker_and_folder_use_the_lowercased_family() {
        // The marker is named for the lowercased family; one named for
        // the family as written is not Desktop's and must not count.
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        let package = package_copy(local, FAMILY);
        // Real data in the package copy, so only the marker can move it.
        touch(&package.join("config.json"));
        touch(&package.join(".user-data-left.Claude_pzs8sxrjxfjjc"));
        fs::create_dir_all(local.join("Claude-Data").join("claude_pzs8sxrjxfjjc")).unwrap();

        // A case-insensitive file system cannot tell the two names
        // apart, and neither can Desktop on it. Probe the volume rather
        // than guess from the OS: macOS volumes can be either.
        if package
            .join(".user-data-left.claude_pzs8sxrjxfjjc")
            .exists()
        {
            return;
        }
        assert_eq!(resolve(local, FAMILY), package);
    }

    #[test]
    fn moved_folder_is_used_when_the_package_copy_is_gone() {
        // "Reset app" in Windows Settings wipes LocalCache, marker and
        // all; Desktop still counts the move as done.
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        let moved = local.join("Claude-Data").join("claude_pzs8sxrjxfjjc");
        touch(&moved.join("config.json"));

        assert_eq!(resolve(local, FAMILY), moved);
    }

    #[test]
    fn moved_folder_is_used_when_the_package_copy_holds_only_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        touch(&package_copy(local, FAMILY).join(".user-data-moving.claude_pzs8sxrjxfjjc"));
        let moved = local.join("Claude-Data").join("claude_pzs8sxrjxfjjc");
        touch(&moved.join("config.json"));

        assert_eq!(resolve(local, FAMILY), moved);
    }

    #[test]
    fn missing_package_copy_without_a_moved_folder_names_the_package() {
        // A fresh install: nothing exists yet, and the error should name
        // the folder Desktop will create first.
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        assert_eq!(resolve(local, FAMILY), package_copy(local, FAMILY));
    }

    #[test]
    fn share_hosted_local_app_data_is_recognised_by_its_prefix() {
        // Desktop's `aUe` tests the string, so a verbatim prefix counts
        // as a share too.
        for share in [
            r"\\server\share\AppData\Local",
            r"\\?\C:\Users\x\AppData\Local",
        ] {
            assert!(is_share(Path::new(share)), "{share}");
        }
        for local in [r"C:\Users\x\AppData\Local", "/Users/x/AppData/Local"] {
            assert!(!is_share(Path::new(local)), "{local}");
        }
    }

    #[test]
    fn a_moved_path_that_is_a_file_is_not_the_moved_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        let package = package_copy(local, FAMILY);
        touch(&package.join(".user-data-left.claude_pzs8sxrjxfjjc"));
        touch(&local.join("Claude-Data").join("claude_pzs8sxrjxfjjc"));

        assert_eq!(resolve(local, FAMILY), package);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_package_copy_is_not_empty() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        let package = package_copy(local, FAMILY);
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(local.join("Claude-Data").join("claude_pzs8sxrjxfjjc")).unwrap();
        fs::set_permissions(&package, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read_dir(&package).is_ok(); // true as root
        let got = resolve(local, FAMILY);
        fs::set_permissions(&package, fs::Permissions::from_mode(0o755)).unwrap();

        if !readable {
            assert_eq!(got, package);
        }
    }

    // A verbatim prefix reaches a real local folder and still starts with
    // `\\`, so it exercises the share guard against a layout that would
    // otherwise resolve to the moved folder.
    #[cfg(target_os = "windows")]
    #[test]
    fn share_prefix_keeps_the_package_copy_despite_a_move() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path();
        touch(&package_copy(plain, FAMILY).join(".user-data-left.claude_pzs8sxrjxfjjc"));
        fs::create_dir_all(plain.join("Claude-Data").join("claude_pzs8sxrjxfjjc")).unwrap();
        assert_ne!(resolve(plain, FAMILY), package_copy(plain, FAMILY));

        let verbatim = PathBuf::from(format!(r"\\?\{}", plain.display()));
        assert_eq!(resolve(&verbatim, FAMILY), package_copy(&verbatim, FAMILY));
    }

    #[test]
    fn invalid_family_stays_in_the_package() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path();
        let family = "Claude";
        assert_eq!(resolve(local, family), package_copy(local, family));
    }

    // The four path shapes from `rules/paths.md`. `resolve` only joins
    // onto `%LOCALAPPDATA%`, so each shape must come back with its
    // prefix intact — a verbatim prefix in particular is kept, not
    // stripped, because the result is opened, never sanitized.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_path_shapes_keep_their_prefix() {
        for (local, want) in [
            (
                r"C:\Users\x\AppData\Local",
                r"C:\Users\x\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude",
            ),
            (
                r"\\server\share\x\AppData\Local",
                r"\\server\share\x\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude",
            ),
            (
                r"\\?\C:\Users\x\AppData\Local",
                r"\\?\C:\Users\x\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude",
            ),
        ] {
            assert_eq!(resolve(Path::new(local), FAMILY), PathBuf::from(want));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_path_shape_joins_the_package_copy() {
        assert_eq!(
            package_copy(Path::new("/Users/x/AppData/Local"), FAMILY),
            PathBuf::from(
                "/Users/x/AppData/Local/Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude"
            )
        );
    }
}
