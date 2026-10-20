//! File dialogs and the words around them (`docs/design/files.md`).
//!
//! The dialogs are native and blocking. They run on the UI thread, which is
//! what the operating systems expect; audio runs on its own thread and keeps
//! playing while one is open.

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use std::path::{Path, PathBuf};

/// The file extension of a patch.
pub const EXTENSION: &str = "ron";

/// What the user chose when asked about unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Save,
    DontSave,
    Cancel,
    /// No dialog could be shown (Linux without `zenity` or `kdialog`).
    Unavailable,
}

/// The name shown for a patch: its file name, or "Untitled" before it has one.
pub fn display_name(file: Option<&Path>) -> String {
    file.and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string())
}

/// The window title: `* name - LoomDSP` with unsaved changes.
pub fn window_title(file: Option<&Path>, unsaved: bool) -> String {
    titled(&display_name(file), unsaved)
}

/// The window title for a patch shown under `name`.
pub fn titled(name: &str, unsaved: bool) -> String {
    format!("{}{name} - LoomDSP", if unsaved { "* " } else { "" })
}

/// `path` with the patch extension added when it has none.
pub fn with_extension(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension(EXTENSION);
    }
    path
}

/// Where a dialog starts: the folder of the current file, otherwise the
/// Documents folder. `None` lets the system choose.
fn start_dir(file: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = file.and_then(Path::parent).filter(|d| d.is_dir()) {
        return Some(dir.to_path_buf());
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join("Documents")).filter(|d| d.is_dir())
}

fn dialog(file: Option<&Path>) -> FileDialog {
    let dialog = FileDialog::new().add_filter("LoomDSP patch", &[EXTENSION]);
    match start_dir(file) {
        Some(dir) => dialog.set_directory(dir),
        None => dialog,
    }
}

/// Asks for a patch to open.
pub fn pick_open(current: Option<&Path>) -> Option<PathBuf> {
    dialog(current).set_title("Open patch").pick_file()
}

/// Asks where to save. The result always has an extension.
pub fn pick_save(current: Option<&Path>) -> Option<PathBuf> {
    let picked = dialog(current)
        .set_title("Save patch")
        .set_file_name(display_name(current))
        .save_file()?;
    let path = with_extension(picked.clone());
    // The dialog checked the name it returned. If adding the extension made
    // a different file, that one needs its own confirmation.
    if path != picked && path.exists() {
        // With no way to ask, keep the name the dialog confirmed.
        if !dialogs_available() {
            return Some(picked);
        }
        if !confirm_replace(&path) {
            return None;
        }
    }
    Some(path)
}

fn confirm_replace(path: &Path) -> bool {
    let result = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(format!("Replace \"{}\"?", display_name(Some(path))))
        .set_description("A file with this name already exists. Saving replaces it.")
        .set_buttons(MessageButtons::YesNo)
        .show();
    result == MessageDialogResult::Yes
}

/// Asks what to do with unsaved changes.
pub fn confirm_discard(name: &str) -> Choice {
    if !dialogs_available() {
        return Choice::Unavailable;
    }
    let result = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(format!("Save changes to \"{name}\"?"))
        .set_description("Your changes will be lost if you do not save them.")
        .set_buttons(MessageButtons::YesNoCancelCustom(
            "Save".to_string(),
            "Don't Save".to_string(),
            "Cancel".to_string(),
        ))
        .show();
    match result {
        MessageDialogResult::Custom(label) if label == "Save" => Choice::Save,
        MessageDialogResult::Custom(label) if label == "Don't Save" => Choice::DontSave,
        MessageDialogResult::Yes => Choice::Save,
        MessageDialogResult::No => Choice::DontSave,
        _ => Choice::Cancel,
    }
}

/// False on Linux when no program that draws message boxes is installed.
/// Then the dialog would return "cancel" without showing anything.
pub fn dialogs_available() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| ["zenity", "kdialog"].iter().any(|p| dir.join(p).is_file()))
}

/// Tells the user that something failed. One button, so it cannot be a
/// question.
pub fn show_error(message: &str) {
    MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title("LoomDSP")
        .set_description(message)
        .set_buttons(MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_shows_file_name_and_mark() {
        let file = Path::new("/music/patches/bass.ron");
        assert_eq!(window_title(Some(file), false), "bass.ron - LoomDSP");
        assert_eq!(window_title(Some(file), true), "* bass.ron - LoomDSP");
        assert_eq!(window_title(None, false), "Untitled - LoomDSP");
    }

    #[test]
    fn save_adds_the_extension_only_when_missing() {
        assert_eq!(with_extension(PathBuf::from("a/bass")), PathBuf::from("a/bass.ron"));
        assert_eq!(with_extension(PathBuf::from("a/bass.ron")), PathBuf::from("a/bass.ron"));
        assert_eq!(with_extension(PathBuf::from("a/bass.txt")), PathBuf::from("a/bass.txt"));
    }
}
