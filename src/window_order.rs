use fltk::{prelude::*, window::Window};

pub fn set_on_top(window: &Window, enabled: bool) {
    if window.shown() {
        set_native(window, enabled);
    }
}

#[cfg(windows)]
fn set_native(window: &Window, enabled: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };

    let order = if enabled {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };
    // The FLTK window owns this handle and is shown on the UI thread.
    unsafe {
        SetWindowPos(
            window.raw_handle(),
            order,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        );
    }
}

#[cfg(target_os = "linux")]
fn set_native(window: &Window, enabled: bool) {
    use std::{mem::MaybeUninit, os::raw::c_ulong};
    use x11::xlib;

    // The Linux build of FLTK uses the X11 backend, including on XWayland.
    let display = fltk::app::display().cast::<xlib::Display>();
    let xid = window.raw_handle() as xlib::Window;
    // The shown FLTK window and its display connection are live on this thread.
    unsafe {
        let mut attributes = MaybeUninit::<xlib::XWindowAttributes>::uninit();
        if xlib::XGetWindowAttributes(display, xid, attributes.as_mut_ptr()) == 0 {
            return;
        }
        let root = attributes.assume_init().root;
        let state = xlib::XInternAtom(display, c"_NET_WM_STATE".as_ptr(), xlib::True);
        let above = xlib::XInternAtom(display, c"_NET_WM_STATE_ABOVE".as_ptr(), xlib::True);
        if state == 0 || above == 0 {
            return;
        }

        // EWMH asks clients to send an add/remove request to the window manager.
        let data = xlib::ClientMessageData::from([c_ulong::from(enabled), above, 0, 1, 0]);
        let mut event = xlib::XEvent {
            client_message: xlib::XClientMessageEvent {
                type_: xlib::ClientMessage,
                serial: 0,
                send_event: xlib::True,
                display,
                window: xid,
                message_type: state,
                format: 32,
                data,
            },
        };
        xlib::XSendEvent(
            display,
            root,
            xlib::False,
            xlib::SubstructureNotifyMask | xlib::SubstructureRedirectMask,
            &mut event,
        );
        xlib::XFlush(display);
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn set_native(_: &Window, _: bool) {}
