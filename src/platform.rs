//! What differs between the desktop and Android: where the files are, and
//! the permission to read them.

use std::path::PathBuf;

/// Whether the app runs on a touch device without a keyboard, which
/// leaves out the keyboard hints and the window controls.
pub const IS_MOBILE: bool = cfg!(target_os = "android");

/// Whether the app browses the storage itself, without a file dialog.
pub const BROWSES_FILES: bool = cfg!(target_os = "android");

/// The top of the storage the browser shows.
pub fn storage_root() -> PathBuf {
    #[cfg(target_os = "android")]
    {
        PathBuf::from("/storage/emulated/0")
    }
    #[cfg(not(target_os = "android"))]
    {
        std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    }
}

/// Where the browser opens first: the downloads.
pub fn default_folder() -> PathBuf {
    storage_root().join("Download")
}

/// Whether the app may read the files of the storage. On Android it needs
/// the "all files access" permission, granted in the settings.
pub fn has_storage_access() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::android::has_storage_access()
    }
    #[cfg(not(target_os = "android"))]
    {
        true
    }
}

/// Leave the app for what was on screen before, as the back key does on
/// Android; the app keeps running behind. Nothing on a desktop.
pub fn send_to_background() {
    #[cfg(target_os = "android")]
    crate::android::send_to_background();
}

/// The app came on screen or left it, for the journal of a phone to tell a
/// crash from the system ending an app behind.
pub fn on_screen(shown: bool) {
    #[cfg(target_os = "android")]
    crate::android::journal::on_screen(shown);
    #[cfg(not(target_os = "android"))]
    let _ = shown;
}

/// Open the settings where the access to the files is granted.
pub fn request_storage_access() {
    #[cfg(target_os = "android")]
    crate::android::request_storage_access();
}
