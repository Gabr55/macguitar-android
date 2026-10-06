//! The file browser, where there is no file dialog: a panel listing a
//! folder, its folders to go into and the files to pick.

use iced::advanced::text::Shaping;
use iced::widget::text::Wrapping;
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::platform;
use crate::ui::app::message::Message;
use crate::ui::browser::{BrowserEntry, FileBrowser};
use crate::ui::icons::{Icon, icon};
use crate::ui::theme::{self, Tokens};
use crate::ui::widgets::{UI_FONT_BOLD, icon_button, muted};

/// Widest the panel gets; a narrow screen gives it all its width.
const BROWSER_WIDTH: f32 = 560.0;

/// The panel of the file browser on its folder.
pub(super) fn file_browser<'a>(browser: &'a FileBrowser, tokens: Tokens) -> Element<'a, Message> {
    let top = platform::storage_root();
    let up = browser.parent(&top).map(Message::BrowseTo);
    let folder_name = if browser.folder == top {
        "Storage".to_string()
    } else {
        browser
            .folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };

    let heading = row![
        icon_button(Icon::Up, "Up a folder", up, tokens),
        column![
            text(browser.pick.title()).size(16).font(UI_FONT_BOLD),
            muted(folder_name)
                .size(12)
                .shaping(Shaping::Advanced)
                .wrapping(Wrapping::None),
        ]
        .width(Length::Fill),
        icon_button(Icon::Close, "Close", Some(Message::CloseBrowser), tokens),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let body: Element<'a, Message> = if !browser.has_access {
        no_access(tokens)
    } else if let Some(problem) = &browser.problem {
        container(muted(format!("This folder cannot be read: {problem}")).size(13))
            .padding(16)
            .into()
    } else if browser.entries.is_empty() {
        container(muted("Nothing to open here").size(13))
            .padding(16)
            .into()
    } else {
        scrollable(column(browser.entries.iter().map(|e| entry(e, tokens))).spacing(2))
            .height(Length::Fill)
            .into()
    };

    container(column![heading, body].spacing(10))
        .padding(12)
        .max_width(BROWSER_WIDTH)
        .height(Length::Fill)
        .style(theme::menu_panel)
        .into()
}

/// A folder to go into, or a file to pick.
fn entry(entry: &BrowserEntry, tokens: Tokens) -> Element<'_, Message> {
    let (glyph, message) = if entry.is_folder {
        (Icon::Open, Message::BrowseTo(entry.path.clone()))
    } else {
        (Icon::Tab, Message::BrowserPicked(entry.path.clone()))
    };
    let label = row![
        icon(glyph, 20.0, tokens.muted),
        text(&entry.name)
            .size(15)
            .shaping(Shaping::Advanced)
            .wrapping(Wrapping::None),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    // rows tall enough for a finger
    button(container(label).width(Length::Fill).clip(true))
        .width(Length::Fill)
        .padding([12, 12])
        .style(theme::menu_item(false))
        .on_press(message)
        .into()
}

/// Asking for the access to the files, the browser's only way to list them.
fn no_access<'a>(tokens: Tokens) -> Element<'a, Message> {
    let allow = button(text("Allow access").size(14).font(UI_FONT_BOLD))
        .padding([10, 16])
        .style(theme::primary_button)
        .on_press(Message::GrantStorageAccess);
    let retry = button(text("Check again").size(14).color(tokens.text))
        .padding([10, 16])
        .style(theme::ghost_button)
        .on_press(Message::BrowseTo(platform::default_folder()));
    column![
        muted(
            "MacGuitar needs the access to all files to list the tabs on this \
             device. Allow it in the settings, then come back."
        )
        .size(14),
        Space::new().height(4),
        row![allow, retry].spacing(10),
    ]
    .spacing(8)
    .padding(16)
    .into()
}
