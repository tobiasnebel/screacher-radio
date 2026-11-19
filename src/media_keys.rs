#[cfg(windows)]
#[path = "media_keys/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "media_keys/linux.rs"]
mod platform;

pub use platform::MediaKeys;
