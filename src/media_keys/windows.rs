use fltk::app::Sender;
use windows::{
    Foundation::TypedEventHandler,
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
    },
    Win32::{Foundation::HWND, System::WinRT::ISystemMediaTransportControlsInterop},
    core::{HSTRING, w},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow};

use crate::{media_keys::MediaKey, station::BoxError, ui::Event};

pub struct MediaKeys {
    controls: SystemMediaTransportControls,
    button_handler: i64,
    playing: Option<bool>,
    _window: MediaWindow,
}

impl MediaKeys {
    pub fn new(messages: Sender<Event>) -> Result<Self, BoxError> {
        // Keep this HWND independent of the main window, which FLTK destroys when hiding to tray.
        let window = MediaWindow::new()?;
        let interop: ISystemMediaTransportControlsInterop =
            windows::core::factory::<SystemMediaTransportControls, _>()?;
        let controls: SystemMediaTransportControls =
            unsafe { interop.GetForWindow(HWND(window.0))? };
        controls.SetIsPlayEnabled(true)?;
        controls.SetIsPauseEnabled(true)?;
        controls.SetIsStopEnabled(true)?;
        controls.SetIsEnabled(true)?;
        let display = controls.DisplayUpdater()?;
        display.SetType(MediaPlaybackType::Music)?;
        display
            .MusicProperties()?
            .SetTitle(&HSTRING::from("Screacher Radio"))?;
        display.Update()?;
        let button_handler = controls.ButtonPressed(&TypedEventHandler::new(
            move |_,
                  args: windows::core::Ref<
                '_,
                SystemMediaTransportControlsButtonPressedEventArgs,
            >| {
                let button = args.ok()?.Button()?;
                let event = match button {
                    SystemMediaTransportControlsButton::Play => {
                        Some(Event::MediaKey(MediaKey::Play))
                    }
                    SystemMediaTransportControlsButton::Pause => {
                        Some(Event::MediaKey(MediaKey::Pause))
                    }
                    SystemMediaTransportControlsButton::Stop => Some(Event::Stop),
                    _ => None,
                };
                if let Some(event) = event {
                    messages.send(event);
                }
                Ok(())
            },
        ))?;
        Ok(Self {
            controls,
            button_handler,
            playing: None,
            _window: window,
        })
    }

    pub fn update(&mut self, playing: bool, _: u8) -> Result<(), BoxError> {
        if self.playing != Some(playing) {
            // Retain a resumable media session when radio playback is stopped.
            self.controls.SetPlaybackStatus(if playing {
                MediaPlaybackStatus::Playing
            } else {
                MediaPlaybackStatus::Paused
            })?;
            self.playing = Some(playing);
        }
        Ok(())
    }
}

impl Drop for MediaKeys {
    fn drop(&mut self) {
        let _ = self.controls.SetIsEnabled(false);
        let _ = self.controls.RemoveButtonPressed(self.button_handler);
    }
}

struct MediaWindow(*mut std::ffi::c_void);

impl MediaWindow {
    fn new() -> Result<Self, BoxError> {
        // STATIC is a built-in window class. This window is never shown.
        let window = unsafe {
            CreateWindowExW(
                0,
                w!("STATIC").as_ptr(),
                w!("Screacher Radio media controls").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if window.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(window))
    }
}

impl Drop for MediaWindow {
    fn drop(&mut self) {
        // Creation and destruction both happen on the FLTK UI thread.
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use fltk::{app, prelude::*, window::Window};
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows media session"]
    fn media_session_survives_minimizing_and_hiding_the_main_window() {
        let _app = app::App::default();
        let (messages, events) = app::channel();
        let mut window = Window::default()
            .with_size(160, 60)
            .with_label("Media key test");
        window.end();
        window.show();
        let mut media = MediaKeys::new(messages).unwrap();
        media.update(false, 75).unwrap();
        let title = format!("Screacher Radio media test {}", std::process::id());
        let display = media.controls.DisplayUpdater().unwrap();
        display
            .MusicProperties()
            .unwrap()
            .SetTitle(&HSTRING::from(&title))
            .unwrap();
        display.Update().unwrap();
        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
            .unwrap()
            .join()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let session = loop {
            assert!(
                Instant::now() < deadline,
                "Windows did not publish the media session"
            );
            let sessions = manager.GetSessions().unwrap();
            let found = (0..sessions.Size().unwrap()).find_map(|index| {
                let session = sessions.GetAt(index).ok()?;
                let metadata = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
                (metadata.Title().ok()? == title).then_some(session)
            });
            if let Some(session) = found {
                break session;
            }
            let _ = app::wait_for(0.03);
        };
        let receive = || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let Some(event) = events.recv() {
                    break event;
                }
                assert!(
                    Instant::now() < deadline,
                    "No event from Windows media controls"
                );
                let _ = app::wait_for(0.02);
            }
        };
        window.iconize();
        assert!(session.TryPlayAsync().unwrap().join().unwrap());
        assert!(matches!(receive(), Event::MediaKey(MediaKey::Play)));
        media.update(true, 75).unwrap();
        assert!(session.TryPauseAsync().unwrap().join().unwrap());
        assert!(matches!(receive(), Event::MediaKey(MediaKey::Pause)));
        media.update(false, 75).unwrap();
        window.hide();
        assert!(!window.shown());
        assert!(session.TryPlayAsync().unwrap().join().unwrap());
        assert!(matches!(receive(), Event::MediaKey(MediaKey::Play)));
        media.update(true, 75).unwrap();
        assert!(session.TryPauseAsync().unwrap().join().unwrap());
        assert!(matches!(receive(), Event::MediaKey(MediaKey::Pause)));
    }
}
