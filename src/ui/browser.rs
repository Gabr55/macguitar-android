//! Browsing the storage for a file, inside the app: Android has no file
//! dialog to lend, so the player lists the folders itself.

use std::path::{Path, PathBuf};

use crate::ui::files::is_tablature;

/// What the browser is looking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Tablature,
    SoundFont,
}

impl Pick {
    /// Whether `path`, a file, is one of those looked for.
    pub fn accepts(self, path: &Path) -> bool {
        match self {
            Self::Tablature => is_tablature(path),
            Self::SoundFont => path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sf2")),
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Tablature => "Open a tab",
            Self::SoundFont => "Choose a SoundFont",
        }
    }
}

/// A folder or a file of the folder being browsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_folder: bool,
}

/// A folder, listed for picking a file in it or going further.
#[derive(Debug, Clone)]
pub struct FileBrowser {
    pub pick: Pick,
    pub folder: PathBuf,
    /// Its folders first, then the files looked for, by name.
    pub entries: Vec<BrowserEntry>,
    /// Why the folder could not be listed.
    pub problem: Option<String>,
    /// The app may read the storage, as of the last listing: asking the
    /// system is no thing to do on every frame.
    pub has_access: bool,
}

impl FileBrowser {
    /// The browser on `folder`, listed.
    pub fn new(pick: Pick, folder: PathBuf) -> Self {
        let mut browser = Self {
            pick,
            folder,
            entries: Vec::new(),
            problem: None,
            has_access: true,
        };
        browser.refresh();
        browser
    }

    /// Go into `folder`.
    pub fn enter(&mut self, folder: PathBuf) {
        self.folder = folder;
        self.refresh();
    }

    /// The folder above, unless at the top of the storage.
    pub fn parent(&self, top: &Path) -> Option<PathBuf> {
        if self.folder == top {
            return None;
        }
        self.folder.parent().map(Path::to_path_buf)
    }

    /// List the folder again.
    pub fn refresh(&mut self) {
        self.has_access = crate::platform::has_storage_access();
        match list(&self.folder, self.pick) {
            Ok(entries) => {
                log::info!(
                    "Browsing {}: {} entries",
                    self.folder.display(),
                    entries.len()
                );
                self.entries = entries;
                self.problem = None;
            }
            Err(error) => {
                log::warn!("Could not list {}: {error}", self.folder.display());
                self.entries.clear();
                self.problem = Some(error.to_string());
            }
        }
    }
}

/// The folders of `folder`, then its files `pick` accepts, each by name,
/// hidden ones left out.
fn list(folder: &Path, pick: Pick) -> std::io::Result<Vec<BrowserEntry>> {
    let mut entries: Vec<BrowserEntry> = std::fs::read_dir(folder)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let is_folder = path.is_dir();
            (is_folder || pick.accepts(&path)).then_some(BrowserEntry {
                path,
                name,
                is_folder,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_folder
            .cmp(&a.is_folder)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_come_first_then_the_files_looked_for() {
        let browser = FileBrowser::new(Pick::Tablature, PathBuf::from("test-files"));
        assert!(browser.problem.is_none());
        let first = &browser.entries[0];
        assert!(first.is_folder, "gold-generated-midi lists first");
        assert!(browser.entries.iter().skip(1).all(|entry| !entry.is_folder));
        // the tabs are listed, the licence and the readme are not
        assert!(
            browser
                .entries
                .iter()
                .any(|entry| entry.name == "effects.gp5")
        );
        assert!(
            !browser
                .entries
                .iter()
                .any(|entry| entry.name.ends_with(".txt"))
        );
        let names: Vec<_> = browser.entries.iter().skip(1).map(|e| &e.name).collect();
        assert!(
            names
                .windows(2)
                .all(|w| w[0].to_lowercase() <= w[1].to_lowercase())
        );
    }

    #[test]
    fn a_missing_folder_says_why() {
        let browser = FileBrowser::new(Pick::Tablature, PathBuf::from("no/such/folder"));
        assert!(browser.entries.is_empty());
        assert!(browser.problem.is_some());
    }

    #[test]
    fn the_top_has_no_parent() {
        let top = PathBuf::from("test-files");
        let mut browser = FileBrowser::new(Pick::SoundFont, top.clone());
        assert_eq!(browser.parent(&top), None);
        browser.enter(top.join("gold-generated-midi"));
        assert_eq!(browser.parent(&top), Some(top));
    }
}
