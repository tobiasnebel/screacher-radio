use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use fltk::app::Sender;
use zbus::{
    blocking::{Connection, connection::Builder},
    fdo,
    zvariant::{ObjectPath, Value},
};

use crate::{media_keys::MediaKey, station::BoxError, ui::Event};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

pub struct MediaKeys {
    connection: Connection,
    playing: Arc<AtomicBool>,
    volume: Arc<AtomicU8>,
}

impl MediaKeys {
    pub fn new(messages: Sender<Event>) -> Result<Self, BoxError> {
        let playing = Arc::new(AtomicBool::new(false));
        let volume = Arc::new(AtomicU8::new(75));
        let connection = Builder::session()?
            .name("org.mpris.MediaPlayer2.screacher_radio")?
            .serve_at(PATH, MediaApplication { messages })?
            .serve_at(
                PATH,
                MediaPlayer {
                    messages,
                    playing: playing.clone(),
                    volume: volume.clone(),
                },
            )?
            .build()?;
        Ok(Self {
            connection,
            playing,
            volume,
        })
    }

    pub fn update(&mut self, playing: bool, volume: u8) -> Result<(), BoxError> {
        let mut changed = HashMap::new();
        if self.playing.swap(playing, Ordering::Relaxed) != playing {
            changed.insert("PlaybackStatus", Value::from(playback_status(playing)));
        }
        if self.volume.swap(volume, Ordering::Relaxed) != volume {
            changed.insert("Volume", Value::from(f64::from(volume) / 100.0));
        }
        if !changed.is_empty() {
            self.connection.emit_signal(
                None::<&str>,
                PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(PLAYER, changed, Vec::<&str>::new()),
            )?;
        }
        Ok(())
    }
}

fn playback_status(playing: bool) -> &'static str {
    if playing { "Playing" } else { "Stopped" }
}

struct MediaApplication {
    messages: Sender<Event>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl MediaApplication {
    fn raise(&self) {
        self.messages.send(Event::Activate);
    }
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "Screacher Radio"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "screacher-radio"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<&str> {
        Vec::new()
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<&str> {
        Vec::new()
    }
}

struct MediaPlayer {
    messages: Sender<Event>,
    playing: Arc<AtomicBool>,
    volume: Arc<AtomicU8>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl MediaPlayer {
    fn play_pause(&self) {
        self.messages.send(Event::MediaKey(MediaKey::Toggle));
    }
    fn play(&self) {
        self.messages.send(Event::MediaKey(MediaKey::Play));
    }
    fn pause(&self) {
        self.messages.send(Event::MediaKey(MediaKey::Pause));
    }
    fn stop(&self) {
        self.messages.send(Event::Stop);
    }
    fn next(&self) {}
    fn previous(&self) {}
    fn seek(&self, _offset: i64) {}
    fn set_position(&self, _track_id: ObjectPath<'_>, _position: i64) {}
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        playback_status(self.playing.load(Ordering::Relaxed))
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn set_rate(&self, _rate: f64) {}
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        0
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<&str, Value<'_>> {
        HashMap::from([
            (
                "mpris:trackid",
                Value::from(ObjectPath::from_static_str_unchecked(
                    "/org/mpris/MediaPlayer2/live",
                )),
            ),
            ("xesam:title", Value::from("Screacher Radio")),
        ])
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        f64::from(self.volume.load(Ordering::Relaxed)) / 100.0
    }
    #[zbus(property)]
    fn set_volume(&self, volume: f64) -> fdo::Result<()> {
        if !volume.is_finite() {
            return Err(fdo::Error::InvalidArgs("Volume must be finite".into()));
        }
        self.messages
            .send(Event::Volume((volume.clamp(0.0, 1.0) * 100.0).round() as u8));
        Ok(())
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use fltk::app;
    use zbus::{blocking::Proxy, proxy::CacheProperties};

    use super::*;

    #[test]
    #[ignore = "requires a private D-Bus session"]
    fn mpris_dispatches_media_requests_without_a_visible_window() {
        let (messages, events) = app::channel();
        let mut media = MediaKeys::new(messages).unwrap();
        let client = Connection::session().unwrap();
        // Read the server state directly; property-change signals update proxy caches asynchronously.
        let player = zbus::blocking::proxy::Builder::<Proxy<'_>>::new(&client)
            .destination("org.mpris.MediaPlayer2.screacher_radio")
            .unwrap()
            .path(PATH)
            .unwrap()
            .interface(PLAYER)
            .unwrap()
            .cache_properties(CacheProperties::No)
            .build()
            .unwrap();
        assert_eq!(
            player.get_property::<String>("PlaybackStatus").unwrap(),
            "Stopped"
        );
        let _: () = player.call("PlayPause", &()).unwrap();
        assert!(matches!(
            events.recv(),
            Some(Event::MediaKey(MediaKey::Toggle))
        ));
        media.update(true, 60).unwrap();
        assert_eq!(
            player.get_property::<String>("PlaybackStatus").unwrap(),
            "Playing"
        );
        let _: () = player.call("Pause", &()).unwrap();
        assert!(matches!(
            events.recv(),
            Some(Event::MediaKey(MediaKey::Pause))
        ));
        let _: () = player.call("Play", &()).unwrap();
        assert!(matches!(
            events.recv(),
            Some(Event::MediaKey(MediaKey::Play))
        ));
        assert!(!player.get_property::<bool>("CanGoNext").unwrap());
        assert!(!player.get_property::<bool>("CanSeek").unwrap());
        player.set_property("Volume", 0.3f64).unwrap();
        assert!(matches!(events.recv(), Some(Event::Volume(30))));
        media.update(false, 30).unwrap();
        assert_eq!(
            player.get_property::<String>("PlaybackStatus").unwrap(),
            "Stopped"
        );
    }
}
