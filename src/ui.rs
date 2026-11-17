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
    enums::{
        Align, CallbackReason, CallbackTrigger, Color, Event as FltkEvent, Font, FrameType,
        Shortcut,
    },
    frame::Frame,
    group::Group,
    input::{Input, IntInput},
    menu::{Choice, MenuBar, MenuFlag},
    output::Output,
    prelude::*,
    valuator::{SliderType, ValueSlider},
    window::Window,
};
use url::Url;

use crate::{
    i18n::{Language, Texts},
    icon,
    metadata::TrackInfo,
    playback::{OutputDevice, Playback, PlaybackEvent, output_devices},
    settings::{self, Settings, Station, Theme},
    station::station_url,
    theme::Palette,
    tray::Tray,
    window_order,
};

#[derive(Clone)]
pub enum Event {
    Quit,
    Show,
    Minimized,
    Escape,
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
    StartTimer(Option<u64>),
    ExtendTimer(u64),
    CancelTimer,
    Settings,
    About,
    MinimizeToTray(bool),
    AlwaysOnTop(bool),
    ThemeChanged(Theme),
    LanguageChanged(Language),
    SelectAudioDevice(i32),
    Playback(u64, PlaybackEvent),
}

pub fn run() {
    let _app = app::App::default().with_scheme(app::Scheme::Gtk);
    let (messages, events) = app::channel::<Event>();
    let (path, mut settings, warning) = match Settings::path(Language::English.texts())
        .and_then(|path| Settings::load(&path).map(|settings| (path, settings)))
    {
        Ok((path, settings)) => (Some(path), settings, None),
        Err(error) => (None, Settings::default(), Some(error)),
    };
    let texts = settings.language.texts();
    let (stations_path, stations, station_warning) =
        match settings::stations_path().and_then(|path| {
            settings::load_stations(&path, &settings.legacy_stations, texts)
                .map(|stations| (path, stations))
        }) {
            Ok((path, stations)) => {
                settings.legacy_stations.clear();
                (Some(path), stations, None)
            }
            Err(error) => (None, settings.legacy_stations.clone(), Some(error)),
        };
    let selected = settings
        .last_station
        .as_ref()
        .and_then(|last| stations.iter().position(|station| &station.url == last));
    let mut radio = Radio {
        view: View::new(messages, &settings, texts),
        messages,
        path,
        settings,
        stations_path,
        stations,
        visible_stations: Vec::new(),
        selected,
        playing: None,
        playback_id: 0,
        timer: None,
        save_at: None,
        tray_available: false,
        tray_position: None,
        next_device_check: Instant::now(),
    };
    radio.refresh_stations();
    radio.refresh_playback_buttons();
    radio.show();
    let tray = Tray::new(messages, texts);
    radio.set_tray_available(tray.is_some());
    if let Some(error) = warning {
        dialog::alert_default(&(texts.settings_load_failed)(&error.to_string()));
    }
    if let Some(error) = station_warning {
        dialog::alert_default(&(texts.stations_load_failed)(&error.to_string()));
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
    started_at: Option<Instant>,
    meter_due: Option<Instant>,
}

struct Radio {
    view: View,
    messages: Sender<Event>,
    path: Option<PathBuf>,
    settings: Settings,
    stations_path: Option<PathBuf>,
    stations: Vec<Station>,
    visible_stations: Vec<usize>,
    selected: Option<usize>,
    playing: Option<Playing>,
    playback_id: u64,
    timer: Option<Instant>,
    save_at: Option<Instant>,
    tray_available: bool,
    tray_position: Option<(i32, i32, i32)>,
    next_device_check: Instant,
}

impl Radio {
    fn handle(&mut self, event: Event) {
        match event {
            Event::Quit => unreachable!(),
            Event::Show => self.show(),
            Event::Minimized => {
                if self.view.window.shown() && !self.view.window.visible() {
                    self.hide_to_tray();
                }
            }
            Event::Escape => self.hide_to_tray(),
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
                show_dialog(
                    &self.view.window,
                    &mut self.view.add_window,
                    self.settings.always_on_top,
                );
                let _ = self.view.name.take_focus();
            }
            Event::SaveStation => self.add_station(),
            Event::Remove => self.remove_station(),
            Event::Favorite => {
                if let Some(index) = self.selected_visible() {
                    let mut stations = self.stations.clone();
                    stations[index].favorite = !stations[index].favorite;
                    if self.replace_stations(stations) {
                        self.refresh_stations();
                    }
                }
            }
            Event::Play => self.play(),
            Event::Stop => self.stop(self.view.texts.playback_stopped),
            Event::Volume(volume) => {
                self.settings.volume = volume;
                if let Some(playing) = &self.playing {
                    playing.playback.set_volume(volume);
                }
                self.save_at = Some(Instant::now() + Duration::from_millis(350));
            }
            Event::Timer => {
                show_dialog(
                    &self.view.window,
                    &mut self.view.timer_window,
                    self.settings.always_on_top,
                );
                let _ = self.view.minutes.take_focus();
            }
            Event::StartTimer(preset) => self.start_timer(preset),
            Event::ExtendTimer(minutes) => {
                if let Some(deadline) = extend_deadline(self.timer, Instant::now(), minutes) {
                    self.timer = Some(deadline);
                }
            }
            Event::CancelTimer => {
                self.timer = None;
                self.view.timer_window.hide();
            }
            Event::Settings => {
                self.refresh_audio_devices(Instant::now());
                show_dialog(
                    &self.view.window,
                    &mut self.view.settings_window,
                    self.settings.always_on_top,
                );
            }
            Event::About => show_dialog(
                &self.view.window,
                &mut self.view.about_window,
                self.settings.always_on_top,
            ),
            Event::MinimizeToTray(value) => {
                self.settings.minimize_to_tray = value;
                self.save();
            }
            Event::AlwaysOnTop(value) => {
                self.settings.always_on_top = value;
                self.view.set_on_top(value);
                self.save();
            }
            Event::ThemeChanged(theme) => {
                self.settings.theme = theme;
                self.view.apply_theme(theme);
                self.save();
            }
            Event::LanguageChanged(language) => {
                self.settings.language = language;
                self.save();
            }
            Event::SelectAudioDevice(0) => self.set_audio_device(None),
            Event::SelectAudioDevice(index) => {
                if index > 0
                    && let Some(device) = self.view.audio_devices.get((index - 1) as usize)
                {
                    self.set_audio_device(Some(device.id.clone()));
                }
            }
            Event::Playback(id, state) if id == self.playback_id => self.playback_event(state),
            Event::Playback(..) => {}
        }
    }

    fn refresh_audio_devices(&mut self, now: Instant) {
        self.next_device_check = now + Duration::from_secs(1);
        if app::grab().is_some() {
            return;
        }
        if let Ok(devices) = output_devices() {
            if let Some(preferred) = &self.settings.audio_device
                && !devices.iter().any(|device| &device.id == preferred)
            {
                self.set_audio_device(None);
            }
            self.view
                .set_audio_devices(devices, self.settings.audio_device.as_deref());
        }
    }

    fn set_audio_device(&mut self, device: Option<String>) {
        if self.settings.audio_device != device {
            if let Some(playing) = &self.playing {
                playing.playback.set_output_device(device.clone());
            }
            self.settings.audio_device = device;
            self.view
                .select_audio_device(self.settings.audio_device.as_deref());
            self.save();
        }
    }

    fn show(&mut self) {
        if let Some((x, y, screen)) = self.tray_position.take() {
            self.view.window.set_pos(x, y);
            // FLTK recreates the native window and otherwise may choose the screen under the mouse.
            self.view.window.set_screen_num(screen);
            self.view.window.force_position(true);
        }
        show_window(&mut self.view.window, self.settings.always_on_top);
    }

    fn hide_to_tray(&mut self) {
        if self.settings.minimize_to_tray && self.tray_available && self.view.window.shown() {
            let window = &self.view.window;
            self.tray_position = Some((window.x(), window.y(), window.screen_num()));
            self.view.timer_window.hide();
            self.view.settings_window.hide();
            self.view.window.hide();
        }
    }

    fn set_tray_available(&mut self, available: bool) {
        self.tray_available = available;
        if available {
            self.view.minimize.activate();
            self.view.tray_note.set_label("");
        } else {
            self.view.minimize.deactivate();
            self.view.tray_note.set_label(self.view.texts.no_tray);
            if self.tray_position.is_some() {
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
        for (index, station) in self.stations.iter().enumerate() {
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
        self.view.empty.set_label(if self.stations.is_empty() {
            self.view.texts.no_stations
        } else {
            self.view.texts.no_matches
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
            let station = &self.stations[index];
            self.view.list.set_tooltip(station.url.as_str());
            let (icon, tooltip) = if station.favorite {
                ("@radio_star", self.view.texts.unfavorite)
            } else {
                ("@radio_star_outline", self.view.texts.favorite)
            };
            self.view.favorite.set_label(icon);
            self.view.favorite.set_tooltip(tooltip);
            self.view.favorite.activate();
            self.view.remove.activate();
        } else {
            self.view.list.set_tooltip("");
            self.view.favorite.set_label("@radio_star_outline");
            self.view.favorite.set_tooltip(self.view.texts.favorite);
            self.view.favorite.deactivate();
            self.view.remove.deactivate();
        }
    }

    fn refresh_playback_buttons(&mut self) {
        let has_last = self
            .settings
            .last_station
            .as_ref()
            .is_some_and(|last| self.stations.iter().any(|station| &station.url == last));
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
            dialog::alert_default(self.view.texts.missing_name);
            return;
        }
        let Ok(url) = station_url(&self.view.url.value(), self.view.texts) else {
            dialog::alert_default(self.view.texts.invalid_url);
            return;
        };
        if self.stations.iter().any(|station| station.url == url) {
            dialog::alert_default(self.view.texts.duplicate_url);
            return;
        }
        let index = self.stations.len();
        let mut stations = self.stations.clone();
        stations.push(Station {
            name,
            url,
            favorite: false,
        });
        if !self.replace_stations(stations) {
            return;
        }
        self.selected = Some(index);
        self.view.search.set_value("");
        self.view.favorites.set_value(false);
        self.view.add_window.hide();
        self.refresh_stations();
    }

    fn remove_station(&mut self) {
        let Some(index) = self.selected_visible() else {
            return;
        };
        let station = &self.stations[index];
        let prompt = (self.view.texts.confirm_delete)(&station.name);
        if dialog::choice2_default(&prompt, self.view.texts.cancel, self.view.texts.delete, "")
            != Some(1)
        {
            return;
        }
        let mut stations = self.stations.clone();
        let station = stations.remove(index);
        if !self.replace_stations(stations) {
            return;
        }
        if self
            .playing
            .as_ref()
            .is_some_and(|playing| playing.url == station.url)
        {
            self.stop(self.view.texts.playback_stopped);
        }
        if self.settings.last_station.as_ref() == Some(&station.url) {
            self.settings.last_station = None;
            self.save();
        }
        self.selected = None;
        self.refresh_stations();
    }

    fn replace_stations(&mut self, stations: Vec<Station>) -> bool {
        let Some(path) = &self.stations_path else {
            dialog::alert_default(self.view.texts.fix_stations);
            return false;
        };
        if let Err(error) = settings::save_stations(path, &stations) {
            dialog::alert_default(&(self.view.texts.stations_save_failed)(
                &path.to_string_lossy(),
                &error.to_string(),
            ));
            return false;
        }
        self.stations = stations;
        true
    }

    fn play(&mut self) {
        let stations = &self.stations;
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
        self.view.clear_playback();
        self.playback_id += 1;
        match Playback::start(
            url.clone(),
            self.settings.volume,
            self.playback_id,
            self.messages,
            self.view.texts,
            self.settings.audio_device.clone(),
        ) {
            Ok(playback) => {
                self.view.status(&(self.view.texts.connecting)(&name));
                self.playing = Some(Playing {
                    url,
                    name,
                    playback,
                    started_at: None,
                    meter_due: None,
                });
            }
            Err(error) => self
                .view
                .status(&(self.view.texts.playback_failed)(&error.to_string())),
        }
        self.refresh_playback_buttons();
    }

    fn stop(&mut self, status: &str) {
        self.playing = None;
        self.view.clear_playback();
        self.view.status(status);
        self.refresh_playback_buttons();
    }

    fn playback_event(&mut self, event: PlaybackEvent) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        match event {
            PlaybackEvent::Started(at) => {
                playing.started_at = Some(at);
                playing.meter_due = Some(at);
                self.view.status(&(self.view.texts.playing)(&playing.name));
                self.settings.last_station = Some(playing.url.clone());
                self.save();
            }
            PlaybackEvent::TrackChanged(track) => self.view.stream_info.set_track(&track),
            PlaybackEvent::AudioDeviceUnavailable(device) => {
                if self.settings.audio_device.as_ref() == Some(&device) {
                    self.set_audio_device(None);
                }
            }
            PlaybackEvent::Ended => self.stop(self.view.texts.stream_ended),
            PlaybackEvent::Failed(error) => {
                let status = (self.view.texts.station_error)(&playing.name, &error);
                self.stop(&status);
            }
        }
    }

    fn start_timer(&mut self, preset: Option<u64>) {
        let minutes = preset.or_else(|| self.view.minutes.value().trim().parse().ok());
        let deadline = minutes.and_then(|minutes| timer_deadline(Instant::now(), minutes));
        match deadline {
            Some(at) => {
                self.timer = Some(at);
                self.view.timer_window.hide();
            }
            None => dialog::alert_default(self.view.texts.invalid_minutes),
        }
    }

    fn update(&mut self, now: Instant) -> f64 {
        let mut wait: f64 = 3600.0;
        if self.view.settings_window.visible()
            || (self.playing.is_none() && self.settings.audio_device.is_some())
        {
            if now >= self.next_device_check {
                self.refresh_audio_devices(now);
            }
            wait = wait.min(self.next_device_check.duration_since(now).as_secs_f64());
        }
        if self.view.window.visible()
            && let Some(playing) = &mut self.playing
            && let Some(due) = &mut playing.meter_due
        {
            if now >= *due {
                self.view.meter.set_levels(playing.playback.stereo_levels());
                self.view.stream_info.set_elapsed(
                    playing
                        .started_at
                        .map(|started| now.duration_since(started).as_secs()),
                );
                *due = now + Duration::from_millis(40);
            }
            wait = wait.min(due.duration_since(now).as_secs_f64());
        }
        let timer_seconds = self.timer.and_then(|at| {
            if now >= at {
                self.timer = None;
                self.stop(self.view.texts.timer_expired);
                return None;
            }
            let remaining = at.duration_since(now).as_secs_f64();
            let seconds = remaining.ceil() as u64;
            wait = wait.min((remaining - (seconds - 1) as f64).max(0.01));
            Some(seconds)
        });
        self.view.set_timer_countdown(timer_seconds);
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
                .status(&(self.view.texts.settings_save_failed)(&error.to_string()));
        }
    }
}

fn show_window(window: &mut Window, on_top: bool) {
    window.show();
    if on_top {
        window_order::set_on_top(window, true);
    }
}

fn show_dialog(parent: &Window, dialog: &mut Window, on_top: bool) {
    // FLTK positions and dimensions are in the same logical screen coordinates.
    let x = parent.x() + (parent.w() - dialog.w()) / 2;
    let y = parent.y() + (parent.h() - dialog.h()) / 2;
    dialog.set_pos(x, y);
    show_window(dialog, on_top);
}

fn timer_deadline(from: Instant, minutes: u64) -> Option<Instant> {
    if minutes == 0 {
        return None;
    }
    from.checked_add(Duration::from_secs(minutes.checked_mul(60)?))
}

fn extend_deadline(deadline: Option<Instant>, now: Instant, minutes: u64) -> Option<Instant> {
    timer_deadline(deadline.filter(|&at| at > now)?, minutes)
}

const TIMER_ICON: &str = "@radio_clock";

struct View {
    texts: &'static Texts,
    window: Window,
    palette: Palette,
    search: Input,
    favorites: CheckButton,
    list: HoldBrowser,
    empty: Frame,
    stream_info: StreamInfo,
    play: Button,
    stop: Button,
    volume: ValueSlider,
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
    extend_label: Frame,
    extend_buttons: [Button; 3],
    timer_seconds: Option<u64>,
    settings_window: Window,
    audio_device: Choice,
    audio_devices: Vec<OutputDevice>,
    minimize: CheckButton,
    tray_note: Frame,
    about_window: Window,
}

impl View {
    fn new(messages: Sender<Event>, settings: &Settings, texts: &'static Texts) -> Self {
        let palette = Palette::current();
        app::add_symbol("radio_clock", true, draw_clock).expect("register clock symbol");
        app::add_symbol("radio_speaker", true, draw_speaker).expect("register speaker symbol");
        app::add_symbol("radio_play", true, draw_play).expect("register play symbol");
        app::add_symbol("radio_stop", true, draw_stop).expect("register stop symbol");
        app::add_symbol("radio_gear", true, draw_gear).expect("register gear symbol");
        app::add_symbol("radio_star", true, |color| draw_star(color, true))
            .expect("register star symbol");
        app::add_symbol("radio_star_outline", true, |color| draw_star(color, false))
            .expect("register outline star symbol");
        let mut window = Window::default()
            .with_size(600, 452)
            .center_screen()
            .with_label("Screacher Radio");
        let mut menu = MenuBar::new(12, 12, 530, 28, None);
        for (label, event) in [
            (texts.menu_add, Event::Add),
            (texts.menu_settings, Event::Settings),
            (texts.menu_quit, Event::Quit),
        ] {
            menu.add(label, Shortcut::None, MenuFlag::Normal, move |_| {
                messages.send(event.clone());
            });
        }
        menu.add(
            texts.menu_about,
            Shortcut::None,
            MenuFlag::Normal,
            move |_| {
                messages.send(Event::About);
            },
        );
        let mut settings_button = Button::new(550, 12, 38, 28, "@radio_gear");
        settings_button.set_tooltip(texts.settings);
        settings_button.emit(messages, Event::Settings);
        let mut search = Input::new(62, 52, 390, 28, texts.search);
        search.set_trigger(CallbackTrigger::Changed);
        search.set_callback(move |_| messages.send(Event::Search));
        let mut favorites = CheckButton::new(460, 52, 128, 28, texts.only_favorites);
        favorites.set_callback(move |_| messages.send(Event::Search));
        let station_area = Group::new(12, 90, 576, 205, None);
        let mut list = HoldBrowser::new(12, 90, 282, 205, None);
        // Station names are stored as single-line text, so neither marker can appear in a row.
        list.set_format_char('\r');
        list.set_column_char('\t');
        list.set_callback(move |list| {
            messages.send(Event::Select(list.value(), app::event_clicks()))
        });
        let mut empty = Frame::new(24, 158, 258, 36, None);
        empty.set_label_color(Color::Dark3);
        let stream_info = StreamInfo::new(texts);
        station_area.end();
        let mut add = Button::new(12, 305, 118, 28, texts.add_action);
        add.emit(messages, Event::Add);
        let mut remove = Button::new(138, 305, 100, 28, texts.remove_action);
        remove.emit(messages, Event::Remove);
        let mut favorite = Button::new(246, 305, 38, 28, "@radio_star_outline");
        favorite.emit(messages, Event::Favorite);
        let mut play = Button::new(12, 344, 76, 28, "@radio_play");
        play.set_tooltip(texts.play);
        play.emit(messages, Event::Play);
        let mut stop = Button::new(96, 344, 76, 28, "@radio_stop");
        stop.set_tooltip(texts.stop);
        stop.emit(messages, Event::Stop);
        Frame::new(207, 344, 24, 28, "@radio_speaker").set_tooltip(texts.volume);
        let mut volume = ValueSlider::new(237, 344, 198, 28, None);
        volume.set_tooltip(texts.volume_wheel);
        volume.set_type(SliderType::Horizontal);
        volume.set_range(0.0, 100.0);
        volume.set_precision(0);
        volume.set_value(f64::from(settings.volume));
        volume
            .set_callback(move |slider| messages.send(Event::Volume(slider.value().round() as u8)));
        volume.handle(|slider, event| {
            if event != FltkEvent::MouseWheel || !app::event_inside_widget(slider) {
                return false;
            }
            let delta = app::event_dy_value();
            if delta == 0 {
                return false;
            }
            let value = (slider.value().round() - 5.0 * f64::from(delta)).clamp(0.0, 100.0);
            if slider.value() != value {
                slider.set_value(value);
                slider.do_callback();
            }
            true
        });
        let mut timer_button = Button::new(468, 344, 120, 28, TIMER_ICON);
        timer_button.set_tooltip(texts.open_timer);
        timer_button.emit(messages, Event::Timer);
        let meter = StereoMeter::new(texts);
        let mut status = Frame::new(12, 421, 576, 22, texts.ready);
        status.set_align(Align::Inside | Align::Left | Align::Clip);
        window.end();
        window.resizable(&station_area);
        window.size_range(600, 452, 0, 0);
        window.set_callback(move |_| {
            messages.send(match app::callback_reason() {
                CallbackReason::Cancelled => Event::Escape,
                _ => Event::Quit,
            });
        });
        window.handle(move |_, event| {
            if event == FltkEvent::Hide {
                messages.send(Event::Minimized);
            }
            false
        });
        window.set_icon(Some(icon::window()));

        let mut add_window = Window::default()
            .with_size(475, 143)
            .with_label(texts.add_station);
        let name = Input::new(68, 14, 393, 28, texts.name);
        let mut url = Input::new(68, 50, 393, 28, "URL");
        url.set_tooltip(texts.url_hint);
        let mut save = Button::new(341, 100, 120, 28, texts.add);
        save.emit(messages, Event::SaveStation);
        add_window.end();
        add_window.make_modal(true);

        let mut timer_window = Window::default()
            .with_size(342, 181)
            .with_label(texts.sleep_timer);
        let mut preset_label = Frame::new(14, 10, 314, 20, texts.start_for);
        preset_label.set_align(Align::Inside | Align::Left);
        for (column, minutes) in [15, 30, 45, 60].into_iter().enumerate() {
            let label = (texts.timer_preset)(minutes);
            Button::new(14 + column as i32 * 80, 34, 74, 28, label.as_str())
                .emit(messages, Event::StartTimer(Some(minutes)));
        }
        let mut extend_label = Frame::new(14, 70, 314, 20, texts.extend);
        extend_label.set_align(Align::Inside | Align::Left);
        let extend_buttons = std::array::from_fn(|column| {
            let minutes = [5, 10, 15][column];
            let label = (texts.timer_extension)(minutes);
            let mut button = Button::new(14 + column as i32 * 80, 94, 74, 28, label.as_str());
            button.emit(messages, Event::ExtendTimer(minutes));
            button.deactivate();
            button
        });
        let mut minutes = IntInput::new(70, 139, 46, 28, texts.minutes);
        minutes.set_value("30");
        let mut clear = Button::new(205, 139, 123, 28, texts.cancel_timer);
        clear.emit(messages, Event::CancelTimer);
        let mut start = Button::new(124, 139, 73, 28, texts.start);
        start.emit(messages, Event::StartTimer(None));
        timer_window.end();
        timer_window.set_callback(|window| window.hide());

        let mut settings_window = Window::default()
            .with_size(465, 288)
            .with_label(texts.settings);
        let mut minimize = CheckButton::new(12, 12, 345, 28, texts.minimize_to_tray);
        minimize.set_value(settings.minimize_to_tray);
        minimize.set_callback(move |button| messages.send(Event::MinimizeToTray(button.value())));
        let mut tray_note = Frame::new(14, 44, 342, 20, None);
        tray_note.set_align(Align::Inside | Align::Left);
        let mut on_top = CheckButton::new(12, 67, 344, 26, texts.always_on_top);
        on_top.set_value(settings.always_on_top);
        on_top.set_callback(move |button| messages.send(Event::AlwaysOnTop(button.value())));
        let mut theme = Choice::new(90, 104, 154, 28, texts.theme);
        theme.add_choice(&format!("{}|{}", texts.theme_standard, texts.theme_dark));
        theme.set_value(i32::from(settings.theme == Theme::Dark));
        theme.set_callback(move |choice| {
            let theme = if choice.value() == 1 {
                Theme::Dark
            } else {
                Theme::Standard
            };
            messages.send(Event::ThemeChanged(theme));
        });
        let mut language = Choice::new(90, 143, 154, 28, texts.language);
        for (index, option) in Language::OPTIONS.into_iter().enumerate() {
            language.add_choice(option.label(texts));
            if option == settings.language {
                language.set_value(index as i32);
            }
        }
        language.set_callback(move |choice| {
            if let Some(&language) = Language::OPTIONS.get(choice.value() as usize) {
                messages.send(Event::LanguageChanged(language));
            }
        });
        let mut restart = Frame::new(14, 175, 342, 20, texts.restart_language);
        restart.set_align(Align::Inside | Align::Left);
        let mut audio_device = Choice::new(154, 205, 297, 28, texts.audio_device);
        audio_device.add_choice(texts.default_audio_device);
        audio_device.set_value(0);
        audio_device
            .set_callback(move |choice| messages.send(Event::SelectAudioDevice(choice.value())));
        let mut close = Button::new(350, 249, 101, 28, texts.close);
        let mut settings_dialog = settings_window.clone();
        close.set_callback(move |_| settings_dialog.hide());
        settings_window.end();
        settings_window.make_modal(false);
        settings_window.set_callback(|window| window.hide());

        let mut about_window = Window::default()
            .with_size(360, 188)
            .with_label(texts.about);
        about_window.set_icon(Some(icon::window()));
        let logo_size = icon::ABOUT_SIZE;
        Frame::new((360 - logo_size) / 2, 14, logo_size, logo_size, None)
            .set_image(Some(icon::about()));
        let version = (texts.version)(env!("SCREACHER_VERSION"));
        let mut info = Frame::new(12, 84, 336, 52, version.as_str());
        info.set_align(Align::Inside | Align::Wrap);
        let mut close = Button::new(130, 146, 100, 28, texts.close);
        let mut about_dialog = about_window.clone();
        close.set_callback(move |_| about_dialog.hide());
        about_window.end();
        about_window.make_modal(true);

        let mut view = Self {
            texts,
            window,
            palette,
            search,
            favorites,
            list,
            empty,
            stream_info,
            play,
            stop,
            volume,
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
            extend_label,
            extend_buttons,
            timer_seconds: None,
            settings_window,
            audio_device,
            audio_devices: Vec::new(),
            minimize,
            tray_note,
            about_window,
        };
        if settings.theme != Theme::Standard {
            view.apply_theme(settings.theme);
        }
        view
    }

    fn set_audio_devices(&mut self, devices: Vec<OutputDevice>, preferred: Option<&str>) {
        if self.audio_devices != devices {
            self.audio_device.clear();
            self.audio_device
                .add_choice(self.texts.default_audio_device);
            for (index, device) in devices.iter().enumerate() {
                let label = if device.label == self.texts.default_audio_device
                    || devices
                        .get(index.wrapping_sub(1))
                        .is_some_and(|previous| previous.label == device.label)
                    || devices
                        .get(index + 1)
                        .is_some_and(|next| next.label == device.label)
                {
                    format!("{} ({})", device.label, index + 1)
                } else {
                    device.label.clone()
                };
                // FLTK treats these characters as menu syntax.
                let mut label = label
                    .replace('\\', "\\\\")
                    .replace('/', "\\/")
                    .replace('&', "&&")
                    .replace('|', "｜")
                    .replace(['\r', '\n', '\t'], " ");
                if label.starts_with('_') {
                    label.insert(0, '\\');
                }
                self.audio_device.add_choice(&label);
            }
            self.audio_devices = devices;
        }
        self.select_audio_device(preferred);
    }

    fn select_audio_device(&mut self, preferred: Option<&str>) {
        let index = preferred
            .and_then(|id| self.audio_devices.iter().position(|device| device.id == id))
            .map_or(0, |index| (index + 1) as i32);
        self.audio_device.set_value(index);
    }

    fn set_on_top(&self, enabled: bool) {
        for window in [
            &self.window,
            &self.timer_window,
            &self.settings_window,
            &self.add_window,
            &self.about_window,
        ] {
            window_order::set_on_top(window, enabled);
        }
    }

    fn apply_theme(&mut self, theme: Theme) {
        self.palette.apply(theme);
        self.volume.set_color(if theme == Theme::Dark {
            Color::Background2
        } else {
            Color::Background
        });
        self.empty.set_label_color(if theme == Theme::Dark {
            Color::Foreground
        } else {
            Color::Dark3
        });
        self.meter.set_theme(theme);
        app::redraw();
    }

    fn status(&mut self, status: &str) {
        self.status.set_label(status);
        self.status.set_tooltip(status);
    }

    fn clear_playback(&mut self) {
        self.meter.set_levels([0, 0]);
        self.stream_info.set_track(&TrackInfo::default());
        self.stream_info.set_elapsed(None);
    }

    fn set_timer_countdown(&mut self, seconds: Option<u64>) {
        if self.timer_seconds == seconds {
            return;
        }
        let was_active = self.timer_seconds.is_some();
        let active = seconds.is_some();
        self.timer_seconds = seconds;
        if was_active != active {
            for button in &mut self.extend_buttons {
                if active {
                    button.activate();
                } else {
                    button.deactivate();
                }
            }
        }
        if let Some(seconds) = seconds {
            let time = format!("{:02}:{:02}", seconds / 60, seconds % 60);
            self.timer_button
                .set_label(&format!("{TIMER_ICON}  {time}"));
            self.extend_label
                .set_label(&(self.texts.timer_remaining)(&time));
        } else {
            self.timer_button.set_label(TIMER_ICON);
            self.extend_label.set_label(self.texts.extend);
        }
    }
}

struct StreamInfo {
    title: Output,
    artist: Output,
    elapsed: Output,
    elapsed_seconds: Option<u64>,
}

impl StreamInfo {
    fn new(texts: &Texts) -> Self {
        let mut panel = Group::new(306, 90, 282, 205, None);
        panel.set_frame(FrameType::BorderBox);
        let mut heading = Frame::new(318, 100, 258, 22, texts.stream_information);
        heading.set_align(Align::Inside | Align::Left);
        heading.set_label_font(Font::HelveticaBold);
        let [title, artist, elapsed] = [
            (texts.title, 129),
            (texts.artist, 183),
            (texts.elapsed, 237),
        ]
        .map(|(label, y)| {
            Frame::new(318, y, 258, 19, label).set_align(Align::Inside | Align::Left);
            let mut value = Output::new(318, y + 20, 258, 25, None);
            value.set_value("—");
            value
        });
        panel.end();
        Self {
            title,
            artist,
            elapsed,
            elapsed_seconds: None,
        }
    }

    fn set_track(&mut self, track: &TrackInfo) {
        for (field, value) in [
            (&mut self.title, &track.title),
            (&mut self.artist, &track.artist),
        ] {
            field.set_value(if value.is_empty() { "—" } else { value });
            field.set_tooltip(value);
        }
    }

    fn set_elapsed(&mut self, seconds: Option<u64>) {
        if self.elapsed_seconds == seconds {
            return;
        }
        self.elapsed_seconds = seconds;
        if let Some(seconds) = seconds {
            self.elapsed.set_value(&format!(
                "{:02}:{:02}:{:02}",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            ));
        } else {
            self.elapsed.set_value("—");
        }
    }
}

// FLTK scales symbol paths from the coordinate range -1 to 1.
fn draw_play(color: Color) {
    draw::set_draw_color(color);
    draw::begin_polygon();
    for (x, y) in [(-0.49, -0.65), (0.66, 0.0), (-0.49, 0.65)] {
        draw::vertex(x, y);
    }
    draw::end_polygon();
}

fn draw_stop(color: Color) {
    draw::set_draw_color(color);
    draw::begin_polygon();
    for (x, y) in [(-0.51, -0.51), (0.51, -0.51), (0.51, 0.51), (-0.51, 0.51)] {
        draw::vertex(x, y);
    }
    draw::end_polygon();
}

fn draw_star(color: Color, filled: bool) {
    draw::set_draw_color(color);
    if filled {
        draw::begin_complex_polygon();
    } else {
        draw::begin_loop();
    }
    for point in 0..10 {
        let angle = f64::from(point) * std::f64::consts::PI / 5.0;
        let radius = if point % 2 == 0 { 0.8 } else { 0.37 };
        draw::vertex(radius * angle.sin(), -radius * angle.cos());
    }
    if filled {
        draw::end_complex_polygon();
    } else {
        draw::end_loop();
    }
}

fn draw_clock(color: Color) {
    draw::set_draw_color(color);
    draw::begin_line();
    draw::draw_arc2(0.0, 0.0, 0.76, 0.0, 360.0);
    draw::end_line();
    draw::begin_line();
    draw::vertex(0.0, -0.47);
    draw::vertex(0.0, 0.0);
    draw::vertex(0.35, 0.22);
    draw::end_line();
}

fn draw_gear(color: Color) {
    draw::set_draw_color(color);
    draw::begin_loop();
    for point in 0..32 {
        let angle = (f64::from(point) - 1.5) * std::f64::consts::PI / 16.0;
        let radius = if matches!(point % 4, 1 | 2) {
            0.82
        } else {
            0.62
        };
        draw::vertex(radius * angle.sin(), -radius * angle.cos());
    }
    draw::end_loop();
    draw::begin_line();
    draw::draw_arc2(0.0, 0.0, 0.27, 0.0, 360.0);
    draw::end_line();
}

fn draw_speaker(color: Color) {
    draw::set_draw_color(color);
    draw::begin_polygon();
    for (x, y) in [
        (-0.85, -0.28),
        (-0.55, -0.28),
        (0.0, -0.77),
        (0.0, 0.77),
        (-0.55, 0.28),
        (-0.85, 0.28),
    ] {
        draw::vertex(x, y);
    }
    draw::end_polygon();
    for radius in [0.39, 0.66] {
        draw::begin_line();
        draw::draw_arc2(0.15, 0.0, radius, -55.0, 55.0);
        draw::end_line();
    }
}

struct StereoMeter {
    frame: Frame,
    segments: Rc<Cell<[i32; 2]>>,
    inactive_color: Rc<Cell<Color>>,
}

impl StereoMeter {
    const SEGMENTS: usize = 24;

    fn new(texts: &Texts) -> Self {
        Frame::new(12, 381, 16, 15, "L").set_tooltip(texts.left_channel);
        Frame::new(12, 398, 16, 15, "R").set_tooltip(texts.right_channel);
        let mut frame = Frame::new(34, 381, 554, 32, None);
        frame.set_tooltip(texts.meter_hint);
        let segments = Rc::new(Cell::new([0, 0]));
        let visible_segments = segments.clone();
        let inactive_color = Rc::new(Cell::new(Color::from_rgb(214, 220, 222)));
        let unlit = inactive_color.clone();
        let green = Color::from_rgb(49, 138, 87);
        let yellow = Color::from_rgb(224, 178, 28);
        let red = Color::from_rgb(194, 62, 51);
        let gradient: [Color; Self::SEGMENTS] = std::array::from_fn(|segment| {
            let position = segment as f32 / (Self::SEGMENTS - 1) as f32;
            if position <= 0.8 {
                let blend = ((position - 0.5) / 0.3).clamp(0.0, 1.0);
                Color::color_average(yellow, green, blend)
            } else {
                Color::color_average(red, yellow, (position - 0.8) / 0.2)
            }
        });
        frame.draw(move |frame| {
            let [left, right] = visible_segments.get();
            let height = (frame.h() - 2) / 2;
            for (y, active_segments) in [(frame.y(), left), (frame.y() + height + 2, right)] {
                for (segment, color) in gradient.iter().copied().enumerate() {
                    let segment = segment as i32;
                    let start = frame.x() + segment * frame.w() / Self::SEGMENTS as i32;
                    let end = frame.x() + (segment + 1) * frame.w() / Self::SEGMENTS as i32;
                    let color = if segment >= active_segments {
                        unlit.get()
                    } else {
                        color
                    };
                    draw::draw_rect_fill(start, y, end - start - 2, height, color);
                }
            }
        });
        Self {
            frame,
            segments,
            inactive_color,
        }
    }

    fn set_theme(&mut self, theme: Theme) {
        self.inactive_color.set(match theme {
            Theme::Standard => Color::from_rgb(214, 220, 222),
            Theme::Dark => Color::from_rgb(58, 68, 63),
        });
    }

    fn set_levels(&mut self, levels: [u8; 2]) {
        let segments = levels.map(|level| (i32::from(level) * Self::SEGMENTS as i32 + 50) / 100);
        if self.segments.replace(segments) != segments {
            self.frame.redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_minutes_to_the_running_deadline() {
        let now = Instant::now();
        let deadline = timer_deadline(now, 30).unwrap();
        let after_two_minutes = now + Duration::from_secs(120);

        for minutes in [5, 10, 15] {
            let extended = extend_deadline(Some(deadline), after_two_minutes, minutes).unwrap();
            assert_eq!(
                extended.duration_since(after_two_minutes).as_secs(),
                (28 + minutes) * 60
            );
        }
        assert_eq!(extend_deadline(Some(deadline), deadline, 5), None);
        assert_eq!(extend_deadline(None, now, 5), None);
    }
}
