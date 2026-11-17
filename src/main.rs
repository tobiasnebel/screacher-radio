#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod i18n;
mod icon;
mod metadata;
mod playback;
mod settings;
mod station;
mod theme;
mod tray;
mod ui;
mod window_order;

fn main() {
    ui::run();
}
