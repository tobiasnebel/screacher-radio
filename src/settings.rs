use std::{
    fs::{self, File},
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    i18n::{Language, Texts},
    station::station_url,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Station {
    pub name: String,
    pub url: Url,
    #[serde(default)]
    pub favorite: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Standard,
    Dark,
}

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    #[serde(rename = "stations", skip_serializing_if = "Vec::is_empty")]
    pub legacy_stations: Vec<Station>,
    pub last_station: Option<Url>,
    pub volume: u8,
    pub minimize_to_tray: bool,
    pub always_on_top: bool,
    pub theme: Theme,
    pub language: Language,
    pub audio_device: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            legacy_stations: Vec::new(),
            last_station: None,
            volume: 75,
            minimize_to_tray: false,
            always_on_top: false,
            theme: Theme::Standard,
            language: Language::default(),
            audio_device: None,
        }
    }
}

impl Settings {
    pub fn path(texts: &Texts) -> io::Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "Screacher Radio")
            .ok_or_else(|| io::Error::other(texts.config_folder_missing))?;
        Ok(dirs.config_dir().join("settings.json"))
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let mut settings: Self = serde_json::from_reader(BufReader::new(file))?;
        settings.volume = settings.volume.min(100);
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let dir = path.parent().expect("settings path has a parent");
        fs::create_dir_all(dir)?;
        write_atomic(path, true, |file| {
            serde_json::to_writer_pretty(&mut *file, self)?;
            file.write_all(b"\n")
        })
    }
}

pub fn stations_path() -> io::Result<PathBuf> {
    Ok(std::env::current_exe()?.with_file_name("stations.yaml"))
}

pub fn load_stations(path: &Path, legacy: &[Station], texts: &Texts) -> io::Result<Vec<Station>> {
    match File::open(path) {
        Ok(file) => read_stations(file, texts),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if legacy.is_empty() {
                return Ok(Vec::new());
            }
            match write_stations(path, legacy, false) {
                Ok(()) => Ok(legacy.to_vec()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    read_stations(File::open(path)?, texts)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn read_stations(file: File, texts: &Texts) -> io::Result<Vec<Station>> {
    let mut stations: Vec<Station> = serde_saphyr::from_reader(BufReader::new(file))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    for (index, station) in stations.iter_mut().enumerate() {
        station.name = station.name.trim().replace(['\t', '\n', '\r', '\0'], " ");
        if station.name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                (texts.station_name_missing)(index + 1),
            ));
        }
        station.url = station_url(station.url.as_str(), texts).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                (texts.station_url_invalid)(index + 1, &error.to_string()),
            )
        })?;
    }
    Ok(stations)
}

pub fn save_stations(path: &Path, stations: &[Station]) -> io::Result<()> {
    write_stations(path, stations, true)
}

fn write_stations(path: &Path, stations: &[Station], replace: bool) -> io::Result<()> {
    write_atomic(path, replace, |file| {
        serde_saphyr::to_io_writer(file, &stations)
            .map_err(|error| io::Error::other(error.to_string()))
    })
}

fn write_atomic(
    path: &Path,
    replace: bool,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let dir = path.parent().expect("configuration path has a parent");
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    write(file.as_file_mut())?;
    file.as_file().sync_all()?;
    if replace {
        file.persist(path)?;
    } else {
        file.persist_noclobber(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_persists_preferences() {
        let mut settings: Settings = serde_json::from_str(r#"{"volume":60}"#).unwrap();
        assert_eq!(settings.theme, Theme::Standard);
        assert_eq!(settings.language, Language::English);
        assert_eq!(settings.audio_device, None);
        assert!(!settings.always_on_top);

        settings.theme = Theme::Dark;
        settings.language = Language::German;
        settings.audio_device = Some("wasapi:test-device".into());
        settings.always_on_top = true;
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.theme, Theme::Dark);
        assert_eq!(restored.language, Language::German);
        assert_eq!(restored.audio_device.as_deref(), Some("wasapi:test-device"));
        assert!(restored.always_on_top);
    }

    #[test]
    fn migrates_and_round_trips_stations_without_reviving_deleted_entries() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("settings.json");
        fs::write(&json_path, r#"{"stations":[{"name":"Café","url":"https://example.org/live","favorite":true}],"volume":33}"#).unwrap();
        let mut settings = Settings::load(&json_path).unwrap();
        assert_eq!(settings.volume, 33);
        let path = dir.path().join("stations.yaml");
        let migrated =
            load_stations(&path, &settings.legacy_stations, Language::English.texts()).unwrap();
        assert_eq!(migrated, settings.legacy_stations);
        assert_eq!(
            load_stations(&path, &[], Language::English.texts()).unwrap(),
            migrated
        );
        save_stations(&path, &[]).unwrap();
        assert!(
            load_stations(&path, &settings.legacy_stations, Language::English.texts())
                .unwrap()
                .is_empty()
        );
        settings.legacy_stations.clear();
        settings.save(&json_path).unwrap();
        assert!(!fs::read_to_string(&json_path).unwrap().contains("stations"));
    }

    #[test]
    fn leaves_an_invalid_yaml_file_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stations.yaml");
        let invalid_yaml = "stations: [";
        fs::write(&path, invalid_yaml).unwrap();
        assert!(load_stations(&path, &[], Language::English.texts()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid_yaml);
    }
}
