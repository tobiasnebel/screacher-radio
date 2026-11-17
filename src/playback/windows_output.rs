use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
            IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
    },
    core::{PCWSTR, Result as WindowsResult, implement},
};

pub(super) struct EndpointNotifications {
    enumerator: IMMDeviceEnumerator,
    callback: IMMNotificationClient,
    changed: Arc<AtomicBool>,
    _apartment: ComApartment,
}

impl EndpointNotifications {
    pub(super) fn register() -> WindowsResult<Self> {
        let apartment = ComApartment::initialize()?;
        // COM is initialized on this worker and both interfaces are owned until unregistration.
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)? };
        let changed = Arc::new(AtomicBool::new(false));
        let callback: IMMNotificationClient = DeviceChange(changed.clone()).into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&callback)? };
        Ok(Self {
            enumerator,
            callback,
            changed,
            _apartment: apartment,
        })
    }

    pub(super) fn take_change(&self) -> bool {
        self.changed.swap(false, Ordering::Relaxed)
    }
}

impl Drop for EndpointNotifications {
    fn drop(&mut self) {
        // Unregister on the worker before dropping the callback or shutting down COM.
        let _ = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.callback)
        };
    }
}

struct ComApartment(std::marker::PhantomData<*mut ()>);

impl ComApartment {
    fn initialize() -> WindowsResult<Self> {
        // The playback worker does not have a Windows message loop.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self(std::marker::PhantomData))
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // This thread owns one successful call to CoInitializeEx.
        unsafe { CoUninitialize() };
    }
}

#[implement(IMMNotificationClient)]
struct DeviceChange(Arc<AtomicBool>);

#[allow(non_snake_case)]
impl IMMNotificationClient_Impl for DeviceChange_Impl {
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _: &PCWSTR,
    ) -> WindowsResult<()> {
        // CPAL uses the eConsole role for its default WASAPI output.
        if flow == eRender && role == eConsole {
            self.0.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> WindowsResult<()> {
        self.0.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> WindowsResult<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> WindowsResult<()> {
        self.0.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> WindowsResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_default_playback_changes_trigger_a_switch() {
        use windows::Win32::Media::Audio::{eCapture, eCommunications};

        let changed = Arc::new(AtomicBool::new(false));
        let callback: IMMNotificationClient = DeviceChange(changed.clone()).into();
        let no_device = PCWSTR::null();
        unsafe {
            callback
                .OnDefaultDeviceChanged(eCapture, eConsole, no_device)
                .unwrap();
            callback
                .OnDefaultDeviceChanged(eRender, eCommunications, no_device)
                .unwrap();
            assert!(!changed.load(Ordering::Relaxed));
            callback
                .OnDefaultDeviceChanged(eRender, eConsole, no_device)
                .unwrap();
        }
        assert!(changed.load(Ordering::Relaxed));
    }
}
