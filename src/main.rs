#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod playback;
mod settings;
mod station;
mod tray;
mod ui;

fn main() {
    ui::run();
}
