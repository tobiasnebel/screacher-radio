use std::{
    fs::{self, File},
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Deserialize, Serialize)]
pub struct Station {
    pub name: String,
    pub url: Url,
    #[serde(default)]
    pub favorite: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub stations: Vec<Station>,
    pub last_station: Option<Url>,
    pub volume: u8,
    pub minimize_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stations: Vec::new(),
            last_station: None,
            volume: 75,
            minimize_to_tray: false,
        }
    }
}

impl Settings {
    pub fn path() -> io::Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "Screacher Radio")
            .ok_or_else(|| io::Error::other("Konfigurationsordner nicht gefunden"))?;
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
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
}
