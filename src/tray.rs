use fltk::app::Sender;

#[cfg(any(windows, target_os = "linux"))]
use crate::icon::{SMALL_RGBA, SMALL_SIZE};
use crate::{i18n::Texts, ui::Event};

#[cfg(windows)]
pub struct Tray {
    _icon: tray_icon::TrayIcon,
}

#[cfg(windows)]
impl Tray {
    pub fn new(messages: Sender<Event>, texts: &'static Texts) -> Option<Self> {
        use tray_icon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
            menu::{Menu, MenuEvent, MenuItem},
        };

        let icon = Icon::from_rgba(SMALL_RGBA.to_vec(), SMALL_SIZE, SMALL_SIZE).ok()?;
        let menu = Menu::new();
        let show = MenuItem::new(texts.show_player, true, None);
        let quit = MenuItem::new(texts.quit, true, None);
        menu.append_items(&[&show, &quit]).ok()?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Screacher Radio")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()?;
        let show_id = show.id().clone();
        let quit_id = quit.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                messages.send(Event::Show);
            } else if event.id == quit_id {
                messages.send(Event::Quit);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                messages.send(Event::Show);
            }
        }));
        Some(Self { _icon: tray })
    }
}

#[cfg(target_os = "linux")]
pub struct Tray(ksni::blocking::Handle<LinuxTray>);

#[cfg(target_os = "linux")]
struct LinuxTray {
    messages: Sender<Event>,
    texts: &'static Texts,
}

#[cfg(target_os = "linux")]
impl Tray {
    pub fn new(messages: Sender<Event>, texts: &'static Texts) -> Option<Self> {
        use ksni::blocking::TrayMethods;

        LinuxTray { messages, texts }.spawn().map(Self).ok()
    }
}

#[cfg(target_os = "linux")]
impl Drop for Tray {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "Screacher Radio".into()
    }

    fn activate(&mut self, _: i32, _: i32) {
        self.messages.send(Event::Show);
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let mut data = SMALL_RGBA.to_vec();
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width: SMALL_SIZE as i32,
            height: SMALL_SIZE as i32,
            data,
        }]
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: self.texts.show_player.into(),
                activate: Box::new(|tray: &mut Self| tray.messages.send(Event::Show)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.texts.quit.into(),
                activate: Box::new(|tray: &mut Self| tray.messages.send(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_online(&self) {
        self.messages.send(Event::TrayAvailable(true));
    }

    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.messages.send(Event::TrayAvailable(false));
        true
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub struct Tray;

#[cfg(not(any(windows, target_os = "linux")))]
impl Tray {
    pub fn new(_: Sender<Event>, _: &'static Texts) -> Option<Self> {
        None
    }
}
