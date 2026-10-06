//! What the app did last, kept on a phone where no console shows it: the
//! log lines worth reading, and whether the app was on screen, to tell an
//! unexpected end (a crash, the system killing it) from a close.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, Log, Metadata, Record};

/// The last lines, saved as they come.
const LOG_FILE: &str = "last-log.txt";
/// Present while the app is on screen: found at a start, the app ended
/// there without closing.
const ON_SCREEN_FILE: &str = "on-screen";
/// How many lines are kept.
const KEPT_LINES: usize = 25;

static JOURNAL: OnceLock<Journal> = OnceLock::new();

struct Journal {
    folder: PathBuf,
    lines: Mutex<VecDeque<String>>,
}

impl Journal {
    fn push(&self, line: String) {
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.len() == KEPT_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    fn save(&self) {
        let lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        let text = lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(self.folder.join(LOG_FILE), text);
    }
}

/// Android's log, with the lines of the player and every warning kept.
struct Logger {
    android: android_logger::AndroidLogger,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.android.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        self.android.log(record);
        if !worth_keeping(record.target(), record.level()) {
            return;
        }
        if let Some(journal) = JOURNAL.get() {
            // one short line each, for the lot to fit a phone's screen
            let message = record.args().to_string();
            let first_line = message.lines().next().unwrap_or_default();
            let line: String = first_line.chars().take(LINE_CHARS).collect();
            journal.push(format!("{} {} {line}", clock(), record.level()));
            // a crash or a kill gives no time to save later
            journal.save();
        }
    }

    fn flush(&self) {}
}

/// Longest a kept line gets.
const LINE_CHARS: usize = 140;

/// The lines of the player itself, and the warnings of the rest but for the
/// graphics stack, which warns of its features at every start: only its
/// errors are kept.
fn worth_keeping(target: &str, level: Level) -> bool {
    let graphics = ["wgpu", "naga", "iced_wgpu"]
        .iter()
        .any(|prefix| target.starts_with(prefix));
    if target.starts_with("macguitar") {
        level <= Level::Info
    } else if graphics {
        level <= Level::Error
    } else {
        level <= Level::Warn
    }
}

/// The time of day, UTC, for the lines.
fn clock() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
        % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Log to Android and to the journal kept in `folder`. Returns what the
/// last run left to tell, when it ended on screen without closing.
pub fn start(folder: PathBuf) -> Option<String> {
    let android = android_logger::AndroidLogger::new(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("macguitar"),
    );
    if log::set_boxed_logger(Box::new(Logger { android })).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let ended_on_screen = folder.join(ON_SCREEN_FILE).exists();
    let report = ended_on_screen.then(|| {
        let tail = std::fs::read_to_string(folder.join(LOG_FILE)).unwrap_or_default();
        format!("It closed while on screen. Its last lines:\n{tail}")
    });
    // this run writes its own
    let _ = std::fs::remove_file(folder.join(LOG_FILE));
    let _ = JOURNAL.set(Journal {
        folder,
        lines: Mutex::new(VecDeque::with_capacity(KEPT_LINES)),
    });
    on_screen(true);
    report
}

/// Keep a line, at once: the reason of a crash.
pub fn keep(line: String) {
    if let Some(journal) = JOURNAL.get() {
        journal.push(line);
        journal.save();
    }
}

/// The app came on screen, or left it: off screen, the system may end it
/// at any time, and that is no crash.
pub fn on_screen(shown: bool) {
    let Some(journal) = JOURNAL.get() else {
        return;
    };
    let marker = journal.folder.join(ON_SCREEN_FILE);
    if shown != marker.exists() {
        log::info!(
            "{}",
            if shown {
                "On screen"
            } else {
                "Left the screen"
            }
        );
    }
    if shown {
        let _ = std::fs::write(marker, "");
    } else {
        journal.save();
        let _ = std::fs::remove_file(marker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_graphics_warnings_are_left_out() {
        assert!(worth_keeping("macguitar::ui::files", Level::Info));
        assert!(!worth_keeping("macguitar::ui::files", Level::Debug));
        assert!(!worth_keeping("wgpu_hal::vulkan::adapter", Level::Warn));
        assert!(worth_keeping("wgpu_core::device", Level::Error));
        assert!(worth_keeping("cpal::host::aaudio", Level::Warn));
        assert!(!worth_keeping("cpal::host::aaudio", Level::Info));
    }
}
