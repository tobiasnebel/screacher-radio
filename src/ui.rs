use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use fltk::{
    app::{self, Sender},
    browser::HoldBrowser,
    button::{Button, CheckButton},
    dialog, draw,
    enums::{Align, CallbackTrigger, Color, ColorDepth, Event as FltkEvent},
    frame::Frame,
    image::RgbImage,
    input::{Input, IntInput},
    prelude::*,
    valuator::{SliderType, ValueSlider},
    window::Window,
};
use url::Url;

use crate::{
    playback::{Playback, PlaybackEvent},
    settings::{Settings, Station},
    station::station_url,
    tray::{self, Tray},
};

#[derive(Clone)]
pub enum Event {
    Quit,
    Show,
    Minimized,
    #[cfg(target_os = "linux")]
    TrayAvailable(bool),
    Search,
    Select(i32, bool),
    Add,
    SaveStation,
    Remove,
    Favorite,
    Play,
    Stop,
    Volume(u8),
    Timer,
    StartTimer,
    CancelTimer,
    Settings,
    MinimizeToTray(bool),
    Playback(u64, PlaybackEvent),
}

pub fn run() {
    let _app = app::App::default().with_scheme(app::Scheme::Gtk);
    let (messages, events) = app::channel::<Event>();
    let (path, settings, warning) = match Settings::path()
        .and_then(|path| Settings::load(&path).map(|settings| (path, settings)))
    {
        Ok((path, settings)) => (Some(path), settings, None),
        Err(error) => (None, Settings::default(), Some(error)),
    };
    let selected = settings.last_station.as_ref().and_then(|last| {
        settings
            .stations
            .iter()
            .position(|station| &station.url == last)
    });
    let mut radio = Radio {
        view: View::new(messages, &settings),
        messages,
        path,
        settings,
        visible_stations: Vec::new(),
        selected,
        playing: None,
        playback_id: 0,
        timer: None,
        save_at: None,
        tray_available: false,
        hidden_in_tray: false,
    };
    radio.refresh_stations();
    radio.refresh_playback_buttons();
    radio.view.window.show();
    let tray = Tray::new(messages);
    radio.set_tray_available(tray.is_some());
    if let Some(error) = warning {
        dialog::alert_default(&format!(
            "Einstellungen konnten nicht geladen werden. Änderungen gelten nur für diese Sitzung.\n{error}"
        ));
    }

    'event_loop: loop {
        while let Some(event) = events.recv() {
            if matches!(event, Event::Quit) {
                break 'event_loop;
            }
            radio.handle(event);
        }
        let wait = radio.update(Instant::now());
        let _ = app::wait_for(wait);
    }
    radio.playing = None;
    if radio.save_at.is_some() {
        radio.save();
    }
}

struct Playing {
    url: Url,
    name: String,
    playback: Playback,
    meter_due: Option<Instant>,
}

struct Radio {
    view: View,
    messages: Sender<Event>,
    path: Option<PathBuf>,
    settings: Settings,
    visible_stations: Vec<usize>,
    selected: Option<usize>,
    playing: Option<Playing>,
    playback_id: u64,
    timer: Option<Instant>,
    save_at: Option<Instant>,
    tray_available: bool,
    hidden_in_tray: bool,
}

impl Radio {
    fn handle(&mut self, event: Event) {
        match event {
            Event::Quit => unreachable!(),
            Event::Show => self.show(),
            Event::Minimized if self.settings.minimize_to_tray && self.tray_available => {
                if self.view.window.shown() && !self.view.window.visible() {
                    self.view.window.hide();
                    self.hidden_in_tray = true;
                }
            }
            Event::Minimized => {}
            #[cfg(target_os = "linux")]
            Event::TrayAvailable(available) => self.set_tray_available(available),
            Event::Search => self.refresh_stations(),
            Event::Select(row, play) => {
                if row > 0
                    && let Some(index) = self.visible_stations.get(row as usize - 1)
                {
                    self.selected = Some(*index);
                    self.refresh_selection();
                    if play {
                        self.play();
                    }
                }
            }
            Event::Add => {
                self.view.name.set_value("");
                self.view.url.set_value("");
                self.view.add_window.show();
                let _ = self.view.name.take_focus();
            }
            Event::SaveStation => self.add_station(),
            Event::Remove => self.remove_station(),
            Event::Favorite => {
                if let Some(index) = self.selected_visible() {
                    let station = &mut self.settings.stations[index];
                    station.favorite = !station.favorite;
                    self.refresh_stations();
                    self.save();
                }
            }
            Event::Play => self.play(),
            Event::Stop => self.stop("Wiedergabe gestoppt"),
            Event::Volume(volume) => {
                self.settings.volume = volume;
                if let Some(playing) = &self.playing {
                    playing.playback.set_volume(volume);
                }
                self.save_at = Some(Instant::now() + Duration::from_millis(350));
            }
            Event::Timer => {
                self.view.timer_window.show();
                let _ = self.view.minutes.take_focus();
            }
            Event::StartTimer => self.start_timer(),
            Event::CancelTimer => {
                self.timer = None;
                self.view.timer_window.hide();
            }
            Event::Settings => self.view.settings_window.show(),
            Event::MinimizeToTray(value) => {
                self.settings.minimize_to_tray = value;
                self.save();
            }
            Event::Playback(id, state) if id == self.playback_id => self.playback_event(state),
            Event::Playback(..) => {}
        }
    }

    fn show(&mut self) {
        self.hidden_in_tray = false;
        self.view.window.show();
    }

    fn set_tray_available(&mut self, available: bool) {
        self.tray_available = available;
        if available {
            self.view.minimize.activate();
            self.view.tray_note.set_label("");
        } else {
            self.view.minimize.deactivate();
            self.view
                .tray_note
                .set_label("Der Desktop stellt kein Tray bereit.");
            if self.hidden_in_tray {
                self.show();
            }
        }
    }

    fn selected_visible(&self) -> Option<usize> {
        self.selected
            .filter(|index| self.visible_stations.contains(index))
    }

    fn refresh_stations(&mut self) {
        let query = self.view.search.value().trim().to_lowercase();
        let favorites = self.view.favorites.value();
        self.visible_stations.clear();
        self.view.list.clear();
        for (index, station) in self.settings.stations.iter().enumerate() {
            if (!favorites || station.favorite)
                && (station.name.to_lowercase().contains(&query)
                    || station.url.as_str().to_lowercase().contains(&query))
            {
                let marker = if station.favorite { "★  " } else { "     " };
                self.view.list.add(&format!("{marker}{}", station.name));
                self.visible_stations.push(index);
                if self.selected == Some(index) {
                    self.view.list.select(self.visible_stations.len() as i32);
                }
            }
        }
        self.view
            .empty
            .set_label(if self.settings.stations.is_empty() {
                "Füge einen Sender per URL hinzu."
            } else {
                "Keine passenden Sender"
            });
        if self.visible_stations.is_empty() {
            self.view.empty.show();
        } else {
            self.view.empty.hide();
        }
        self.refresh_selection();
        self.refresh_playback_buttons();
    }

    fn refresh_selection(&mut self) {
        if let Some(index) = self.selected_visible() {
            let station = &self.settings.stations[index];
            self.view.list.set_tooltip(station.url.as_str());
            self.view.favorite.set_label(if station.favorite {
                "Favorit entfernen"
            } else {
                "Als Favorit"
            });
            self.view.favorite.activate();
            self.view.remove.activate();
        } else {
            self.view.list.set_tooltip("");
            self.view.favorite.set_label("Als Favorit");
            self.view.favorite.deactivate();
            self.view.remove.deactivate();
        }
    }

    fn refresh_playback_buttons(&mut self) {
        let has_last = self.settings.last_station.as_ref().is_some_and(|last| {
            self.settings
                .stations
                .iter()
                .any(|station| &station.url == last)
        });
        if has_last || !self.visible_stations.is_empty() {
            self.view.play.activate();
        } else {
            self.view.play.deactivate();
        }
        if self.playing.is_some() {
            self.view.stop.activate();
        } else {
            self.view.stop.deactivate();
        }
    }

    fn add_station(&mut self) {
        let name = self
            .view
            .name
            .value()
            .trim()
            .replace(['\t', '\n', '\r'], " ");
        if name.is_empty() {
            dialog::alert_default("Gib einen Namen für den Sender ein.");
            return;
        }
        let Ok(url) = station_url(&self.view.url.value()) else {
            dialog::alert_default("Gib eine gültige HTTP- oder HTTPS-URL ein.");
            return;
        };
        if self
            .settings
            .stations
            .iter()
            .any(|station| station.url == url)
        {
            dialog::alert_default("Diese URL ist bereits in der Senderliste.");
            return;
        }
        self.selected = Some(self.settings.stations.len());
        self.settings.stations.push(Station {
            name,
            url,
            favorite: false,
        });
        self.view.search.set_value("");
        self.view.favorites.set_value(false);
        self.view.add_window.hide();
        self.refresh_stations();
        self.save();
    }

    fn remove_station(&mut self) {
        let Some(index) = self.selected_visible() else {
            return;
        };
        let station = &self.settings.stations[index];
        let prompt = format!("„{}“ aus der Senderliste löschen?", station.name);
        if dialog::choice2_default(&prompt, "Abbrechen", "Löschen", "") != Some(1) {
            return;
        }
        let station = self.settings.stations.remove(index);
        if self
            .playing
            .as_ref()
            .is_some_and(|playing| playing.url == station.url)
        {
            self.stop("Wiedergabe gestoppt");
        }
        if self.settings.last_station.as_ref() == Some(&station.url) {
            self.settings.last_station = None;
        }
        self.selected = None;
        self.refresh_stations();
        self.save();
    }

    fn play(&mut self) {
        let stations = &self.settings.stations;
        let selected = self.selected_visible().map(|index| &stations[index]);
        let last = self
            .settings
            .last_station
            .as_ref()
            .and_then(|url| stations.iter().find(|station| &station.url == url));
        let first = self.visible_stations.first().map(|&index| &stations[index]);
        let Some(station) = selected.or(last).or(first) else {
            return;
        };
        if self
            .playing
            .as_ref()
            .is_some_and(|playing| playing.url == station.url)
        {
            return;
        }
        let url = station.url.clone();
        let name = station.name.clone();
        self.playing = None;
        self.view.meter.set_levels([0, 0]);
        self.playback_id += 1;
        match Playback::start(
            url.clone(),
            self.settings.volume,
            self.playback_id,
            self.messages,
        ) {
            Ok(playback) => {
                self.view.status(&format!("Verbinde mit {name} …"));
                self.playing = Some(Playing {
                    url,
                    name,
                    playback,
                    meter_due: None,
                });
            }
            Err(error) => self.view.status(&format!(
                "Wiedergabe konnte nicht gestartet werden ({error})."
            )),
        }
        self.refresh_playback_buttons();
    }

    fn stop(&mut self, status: &str) {
        self.playing = None;
        self.view.meter.set_levels([0, 0]);
        self.view.status(status);
        self.refresh_playback_buttons();
    }

    fn playback_event(&mut self, event: PlaybackEvent) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        match event {
            PlaybackEvent::Started => {
                playing.meter_due = Some(Instant::now());
                self.view.status(&format!("Spielt: {}", playing.name));
                self.settings.last_station = Some(playing.url.clone());
                self.save();
            }
            PlaybackEvent::Ended => self.stop("Stream beendet"),
            PlaybackEvent::Failed(error) => {
                let status = format!("{}: {error}", playing.name);
                self.stop(&status);
            }
        }
    }

    fn start_timer(&mut self) {
        let minutes = self.view.minutes.value().trim().parse::<u64>();
        let deadline = minutes
            .ok()
            .filter(|&minutes| minutes > 0)
            .and_then(|minutes| {
                let seconds = minutes.checked_mul(60)?;
                Instant::now().checked_add(Duration::from_secs(seconds))
            });
        match deadline {
            Some(at) => {
                self.timer = Some(at);
                self.view.timer_window.hide();
            }
            None => dialog::alert_default("Gib eine gültige, positive Anzahl Minuten ein."),
        }
    }

    fn update(&mut self, now: Instant) -> f64 {
        let mut wait: f64 = 3600.0;
        if self.view.window.visible()
            && let Some(playing) = &mut self.playing
            && let Some(due) = &mut playing.meter_due
        {
            if now >= *due {
                self.view.meter.set_levels(playing.playback.stereo_levels());
                *due = now + Duration::from_millis(40);
            }
            wait = wait.min(due.duration_since(now).as_secs_f64());
        }
        if let Some(at) = self.timer {
            if now >= at {
                self.timer = None;
                self.stop("Sleeptimer abgelaufen");
            } else {
                let remaining = at.duration_since(now).as_secs_f64();
                let seconds = remaining.ceil() as u64;
                let text = format!("Timer {:02}:{:02}", seconds / 60, seconds % 60);
                if self.view.timer_button.label() != text {
                    self.view.timer_button.set_label(&text);
                }
                wait = wait.min((remaining - (seconds - 1) as f64).max(0.01));
            }
        }
        if self.timer.is_none() && self.view.timer_button.label() != "Sleeptimer…" {
            self.view.timer_button.set_label("Sleeptimer…");
        }
        if let Some(at) = self.save_at {
            if now >= at {
                self.save();
            } else {
                wait = wait.min(at.duration_since(now).as_secs_f64());
            }
        }
        wait
    }

    fn save(&mut self) {
        self.save_at = None;
        if let Some(path) = &self.path
            && let Err(error) = self.settings.save(path)
        {
            self.view
                .status(&format!("Speichern fehlgeschlagen: {error}"));
        }
    }
}

struct View {
    window: Window,
    search: Input,
    favorites: CheckButton,
    list: HoldBrowser,
    empty: Frame,
    play: Button,
    stop: Button,
    favorite: Button,
    remove: Button,
    timer_button: Button,
    meter: StereoMeter,
    status: Frame,
    add_window: Window,
    name: Input,
    url: Input,
    timer_window: Window,
    minutes: IntInput,
    settings_window: Window,
    minimize: CheckButton,
    tray_note: Frame,
}

impl View {
    fn new(messages: Sender<Event>, settings: &Settings) -> Self {
        let mut window = Window::default()
            .with_size(600, 452)
            .center_screen()
            .with_label("Screacher Radio");
        Button::new(468, 12, 120, 28, "Einstellungen…").emit(messages, Event::Settings);
        let mut search = Input::new(62, 52, 390, 28, "Suche");
        search.set_trigger(CallbackTrigger::Changed);
        search.set_callback(move |_| messages.send(Event::Search));
        let mut favorites = CheckButton::new(460, 52, 128, 28, "Nur Favoriten");
        favorites.set_callback(move |_| messages.send(Event::Search));
        let mut list = HoldBrowser::new(12, 90, 576, 205, None);
        list.set_format_char('\0');
        list.set_column_char('\0');
        list.set_callback(move |list| {
            messages.send(Event::Select(list.value(), app::event_clicks()))
        });
        let mut empty = Frame::new(24, 158, 550, 36, None);
        empty.set_label_color(fltk::enums::Color::Dark3);
        let mut add = Button::new(12, 305, 118, 28, "Hinzufügen…");
        add.emit(messages, Event::Add);
        let mut remove = Button::new(138, 305, 100, 28, "Löschen…");
        remove.emit(messages, Event::Remove);
        let mut favorite = Button::new(246, 305, 133, 28, "Als Favorit");
        favorite.emit(messages, Event::Favorite);
        let mut play = Button::new(12, 344, 76, 28, "Play");
        play.emit(messages, Event::Play);
        let mut stop = Button::new(96, 344, 76, 28, "Stop");
        stop.emit(messages, Event::Stop);
        let mut volume = ValueSlider::new(262, 344, 166, 28, "Lautstärke");
        volume.set_align(Align::Left);
        volume.set_type(SliderType::Horizontal);
        volume.set_range(0.0, 100.0);
        volume.set_precision(0);
        volume.set_value(f64::from(settings.volume));
        volume
            .set_callback(move |slider| messages.send(Event::Volume(slider.value().round() as u8)));
        let mut timer_button = Button::new(468, 344, 120, 28, "Sleeptimer…");
        timer_button.emit(messages, Event::Timer);
        let meter = StereoMeter::new();
        let mut status = Frame::new(12, 421, 576, 22, "Bereit");
        status.set_align(Align::Inside | Align::Left | Align::Clip);
        window.end();
        window.resizable(&list);
        window.size_range(600, 452, 0, 0);
        window.emit(messages, Event::Quit);
        window.handle(move |_, event| {
            if event == FltkEvent::Hide {
                messages.send(Event::Minimized);
            }
            false
        });
        if let Ok(icon) = RgbImage::new(&tray::icon_rgba(), 32, 32, ColorDepth::Rgba8) {
            window.set_icon(Some(icon));
        }

        let mut add_window = Window::default()
            .with_size(475, 143)
            .center_screen()
            .with_label("Sender hinzufügen");
        let name = Input::new(68, 14, 393, 28, "Name");
        let mut url = Input::new(68, 50, 393, 28, "URL");
        url.set_tooltip("HTTP(S)-Adresse eines MP3-Streams oder einer M3U-/PLS-Playlist");
        let mut save = Button::new(341, 100, 120, 28, "Hinzufügen");
        save.emit(messages, Event::SaveStation);
        add_window.end();
        add_window.make_modal(true);

        let mut timer_window = Window::default()
            .with_size(342, 105)
            .center_screen()
            .with_label("Sleeptimer");
        let mut minutes = IntInput::new(82, 14, 65, 28, "Minuten");
        minutes.set_value("30");
        let mut clear = Button::new(94, 63, 130, 28, "Timer abbrechen");
        clear.emit(messages, Event::CancelTimer);
        let mut start = Button::new(232, 63, 96, 28, "Starten");
        start.emit(messages, Event::StartTimer);
        timer_window.end();
        timer_window.make_modal(true);

        let mut settings_window = Window::default()
            .with_size(370, 112)
            .center_screen()
            .with_label("Einstellungen");
        let mut minimize =
            CheckButton::new(12, 12, 345, 28, "Beim Minimieren in den Tray verschieben");
        minimize.set_value(settings.minimize_to_tray);
        minimize.set_callback(move |button| messages.send(Event::MinimizeToTray(button.value())));
        let mut tray_note = Frame::new(14, 44, 342, 20, None);
        tray_note.set_align(Align::Inside | Align::Left);
        let mut close = Button::new(255, 72, 101, 28, "Schließen");
        let mut settings_dialog = settings_window.clone();
        close.set_callback(move |_| settings_dialog.hide());
        settings_window.end();
        settings_window.make_modal(true);

        Self {
            window,
            search,
            favorites,
            list,
            empty,
            play,
            stop,
            favorite,
            remove,
            timer_button,
            meter,
            status,
            add_window,
            name,
            url,
            timer_window,
            minutes,
            settings_window,
            minimize,
            tray_note,
        }
    }

    fn status(&mut self, status: &str) {
        self.status.set_label(status);
        self.status.set_tooltip(status);
    }
}

struct StereoMeter {
    frame: Frame,
    segments: Rc<Cell<[i32; 2]>>,
}

impl StereoMeter {
    const SEGMENTS: i32 = 24;

    fn new() -> Self {
        Frame::new(12, 381, 16, 15, "L").set_tooltip("Linker Kanal");
        Frame::new(12, 398, 16, 15, "R").set_tooltip("Rechter Kanal");
        let mut frame = Frame::new(34, 381, 554, 32, None);
        frame.set_tooltip("Stereo-Spitzenpegel nach Lautstärke (−48 bis 0 dBFS)");
        let segments = Rc::new(Cell::new([0, 0]));
        let visible_segments = segments.clone();
        frame.draw(move |frame| {
            let [left, right] = visible_segments.get();
            let height = (frame.h() - 2) / 2;
            for (y, active_segments) in [(frame.y(), left), (frame.y() + height + 2, right)] {
                for segment in 0..Self::SEGMENTS {
                    let start = frame.x() + segment * frame.w() / Self::SEGMENTS;
                    let end = frame.x() + (segment + 1) * frame.w() / Self::SEGMENTS;
                    let color = if segment >= active_segments {
                        Color::from_rgb(214, 220, 222)
                    } else {
                        match segment {
                            22.. => Color::from_rgb(194, 62, 51),
                            19..=21 => Color::from_rgb(208, 151, 32),
                            _ => Color::from_rgb(49, 138, 87),
                        }
                    };
                    draw::draw_rect_fill(start, y, end - start - 2, height, color);
                }
            }
        });
        Self { frame, segments }
    }

    fn set_levels(&mut self, levels: [u8; 2]) {
        let segments = levels.map(|level| (i32::from(level) * Self::SEGMENTS + 50) / 100);
        if self.segments.replace(segments) != segments {
            self.frame.redraw();
        }
    }
}
