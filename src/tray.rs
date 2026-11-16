use fltk::app::Sender;

use crate::ui::Event;

const SIZE: u32 = 32;

pub fn icon_rgba() -> Vec<u8> {
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let radius_squared = (2 * x - 31).pow(2) + (2 * y - 31).pow(2);
            let color = if radius_squared >= 30 * 30 {
                [0, 0, 0, 0]
            } else if (10..=22).contains(&x) && (y - 16).abs() * 2 <= 30 - x {
                [255, 255, 255, 255]
            } else {
                [36, 90, 128, 255]
            };
            pixels.extend(color);
        }
    }
    pixels
}

#[cfg(windows)]
pub struct Tray {
    _icon: tray_icon::TrayIcon,
}

#[cfg(windows)]
impl Tray {
    pub fn new(messages: Sender<Event>) -> Option<Self> {
        use tray_icon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
            menu::{Menu, MenuEvent, MenuItem},
        };

        let icon = Icon::from_rgba(icon_rgba(), SIZE, SIZE).ok()?;
        let menu = Menu::new();
        let show = MenuItem::new("Player öffnen", true, None);
        let quit = MenuItem::new("Beenden", true, None);
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
struct LinuxTray(Sender<Event>);

#[cfg(target_os = "linux")]
impl Tray {
    pub fn new(messages: Sender<Event>) -> Option<Self> {
        use ksni::blocking::TrayMethods;

        LinuxTray(messages).spawn().map(Self).ok()
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
        self.0.send(Event::Show);
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let mut data = icon_rgba();
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width: SIZE as i32,
            height: SIZE as i32,
            data,
        }]
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Player öffnen".into(),
                activate: Box::new(|tray: &mut Self| tray.0.send(Event::Show)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Beenden".into(),
                activate: Box::new(|tray: &mut Self| tray.0.send(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_online(&self) {
        self.0.send(Event::TrayAvailable(true));
    }

    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.0.send(Event::TrayAvailable(false));
        true
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub struct Tray;

#[cfg(not(any(windows, target_os = "linux")))]
impl Tray {
    pub fn new(_: Sender<Event>) -> Option<Self> {
        None
    }
}
