use serde::{Deserialize, Serialize};

mod de;
mod en;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    German,
    System,
}

impl Language {
    pub const OPTIONS: [Self; 3] = [Self::English, Self::German, Self::System];

    pub fn label(self, texts: &Texts) -> &'static str {
        match self {
            Self::English => "English",
            Self::German => "Deutsch",
            Self::System => texts.language_system,
        }
    }

    pub fn texts(self) -> &'static Texts {
        match self {
            Self::English => &en::TEXTS,
            Self::German => &de::TEXTS,
            Self::System => locale_language(&system_locale()).texts(),
        }
    }
}

fn locale_language(locale: &str) -> Language {
    if locale
        .split(['-', '_', '.'])
        .next()
        .is_some_and(|code| code.eq_ignore_ascii_case("de"))
    {
        Language::German
    } else {
        Language::English
    }
}

#[cfg(windows)]
fn system_locale() -> String {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;

    // Windows locale names fit in 85 UTF-16 code units including the terminator.
    let mut name = [0; 85];
    let len = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    if len > 0 {
        String::from_utf16_lossy(&name[..len as usize - 1])
    } else {
        String::new()
    }
}

#[cfg(not(windows))]
fn system_locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

pub struct Texts {
    pub menu_add: &'static str,
    pub menu_settings: &'static str,
    pub menu_quit: &'static str,
    pub menu_about: &'static str,
    pub settings: &'static str,
    pub search: &'static str,
    pub only_favorites: &'static str,
    pub add_action: &'static str,
    pub remove_action: &'static str,
    pub play: &'static str,
    pub stop: &'static str,
    pub volume: &'static str,
    pub volume_wheel: &'static str,
    pub mute: &'static str,
    pub unmute: &'static str,
    pub minimize_window: &'static str,
    pub open_timer: &'static str,
    pub ready: &'static str,
    pub add_station: &'static str,
    pub name: &'static str,
    pub url_hint: &'static str,
    pub add: &'static str,
    pub edit: &'static str,
    pub save: &'static str,
    pub station_info: &'static str,
    pub loading_station_info: &'static str,
    pub cancel_reconnect: &'static str,
    pub retry_wait: fn(&str, u64, u64, &str) -> String,
    pub retry_connecting: fn(&str, u64) -> String,
    pub sleep_timer: &'static str,
    pub start_for: &'static str,
    pub extend: &'static str,
    pub minutes: &'static str,
    pub cancel_timer: &'static str,
    pub start: &'static str,
    pub restart_language: &'static str,
    pub ignore: &'static str,
    pub minimize_to_tray: &'static str,
    pub always_on_top: &'static str,
    pub transparency: &'static str,
    pub transparency_hint: &'static str,
    pub auto_play_last_station: &'static str,
    pub media_key_action: &'static str,
    pub media_key_playback: &'static str,
    pub media_key_window: &'static str,
    pub media_keys_failed: fn(&str) -> String,
    pub theme: &'static str,
    pub theme_standard: &'static str,
    pub theme_dark: &'static str,
    pub theme_light: &'static str,
    pub language: &'static str,
    pub language_system: &'static str,
    pub close: &'static str,
    pub about: &'static str,
    pub stream_information: &'static str,
    pub title: &'static str,
    pub artist: &'static str,
    pub elapsed: &'static str,
    pub left_channel: &'static str,
    pub right_channel: &'static str,
    pub meter_hint: &'static str,
    pub playback_stopped: &'static str,
    pub no_tray: &'static str,
    pub no_stations: &'static str,
    pub no_matches: &'static str,
    pub unfavorite: &'static str,
    pub favorite: &'static str,
    pub missing_name: &'static str,
    pub invalid_url: &'static str,
    pub duplicate_url: &'static str,
    pub cancel: &'static str,
    pub delete: &'static str,
    pub fix_stations: &'static str,
    pub stream_ended: &'static str,
    pub invalid_minutes: &'static str,
    pub timer_expired: &'static str,
    pub show_player: &'static str,
    pub quit: &'static str,
    pub config_folder_missing: &'static str,
    pub http_required: &'static str,
    pub playback_cancelled: &'static str,
    pub nested_playlist: &'static str,
    pub playlist_timeout: &'static str,
    pub playlist_too_large: &'static str,
    pub hls_unsupported: &'static str,
    pub playlist_empty: &'static str,
    pub empty_audio: &'static str,
    pub audio_device: &'static str,
    pub default_audio_device: &'static str,
    pub opening_timeout: &'static str,
    pub settings_load_failed: fn(&str) -> String,
    pub instance_failed: fn(&str) -> String,
    pub stations_load_failed: fn(&str) -> String,
    pub confirm_delete: fn(&str) -> String,
    pub stations_save_failed: fn(&str, &str) -> String,
    pub connecting: fn(&str) -> String,
    pub playing: fn(&str) -> String,
    pub settings_save_failed: fn(&str) -> String,
    pub timer_preset: fn(u64) -> String,
    pub timer_extension: fn(u64) -> String,
    pub timer_remaining: fn(&str) -> String,
    pub version: fn(&str) -> String,
    pub station_name_missing: fn(usize) -> String,
    pub station_url_invalid: fn(usize, &str) -> String,
    pub mp3_open_failed: fn(&str) -> String,
    pub audio_open_failed: fn(&str) -> String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_messages_use_the_selected_language() {
        let english = Language::English.texts();
        let german = Language::German.texts();
        assert_eq!((english.playing)("Radio 1"), "Playing: Radio 1");
        assert_eq!((german.playing)("Radio 1"), "Spielt: Radio 1");
        assert_eq!(
            (english.timer_remaining)("12:30"),
            "Extend (12:30 remaining)"
        );
        assert_eq!(
            (german.timer_remaining)("12:30"),
            "Verlängern (Restzeit 12:30)"
        );
    }

    #[test]
    fn system_locale_selects_german_with_english_fallback() {
        for locale in ["de", "de-AT", "de_DE.UTF-8"] {
            assert_eq!(locale_language(locale), Language::German);
        }
        for locale in ["", "C", "en-US", "fr_FR.UTF-8"] {
            assert_eq!(locale_language(locale), Language::English);
        }
    }
}
