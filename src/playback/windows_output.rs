use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rodio::{Source, StreamError};
use windows::{
    Win32::{
        Media::Audio::{
            DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
            IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Shell::PropertiesSystem::PROPERTYKEY,
    },
    core::{PCWSTR, Result as WindowsResult, implement},
};

use super::{ActiveOutput, RadioSource, Texts};

pub(super) struct AudioOutput {
    active: Option<ActiveOutput>,
    attached: bool,
    source: Arc<Mutex<Option<RadioSource>>>,
    notifications: Option<EndpointNotifications>,
    retry_at: Option<Instant>,
}

impl AudioOutput {
    pub(super) fn open(source: RadioSource) -> Result<Self, StreamError> {
        // Subscribe first so a change while opening the initial output is also observed.
        let notifications = EndpointNotifications::register().ok();
        let mut output = Self {
            active: Some(ActiveOutput::open()?),
            attached: false,
            source: Arc::new(Mutex::new(Some(source))),
            notifications,
            retry_at: None,
        };
        output.attach_source();
        Ok(output)
    }

    pub(super) fn ready(&mut self, _: &Texts) -> Result<bool, &'static str> {
        let now = Instant::now();
        if self
            .notifications
            .as_ref()
            .is_some_and(EndpointNotifications::take_change)
        {
            self.retry_at = Some(now);
        }
        if self.active.as_ref().is_some_and(ActiveOutput::failed) {
            self.active = None;
            self.attached = false;
            self.retry_at.get_or_insert(now);
        }
        if self.retry_at.is_some_and(|at| now >= at) {
            match ActiveOutput::open() {
                Ok(output) => {
                    self.active = Some(output);
                    self.attached = false;
                    self.retry_at = None;
                }
                Err(_) => self.retry_at = Some(Instant::now() + Duration::from_millis(500)),
            }
        }
        if self.active.is_none() {
            // Discard audio produced during an outage so playback resumes near the live edge.
            if let Some(source) = self.source.lock().unwrap().as_mut() {
                let queued = source.samples.slots();
                source.samples.read_chunk(queued).unwrap().commit_all();
                source.meter.levels.store(0, Ordering::Relaxed);
            }
        }
        Ok(self.attach_source())
    }

    pub(super) fn drained(&self) -> bool {
        self.active
            .as_ref()
            .is_none_or(|output| output.sink.empty())
    }

    fn attach_source(&mut self) -> bool {
        let Some(output) = &self.active else {
            return false;
        };
        if !self.attached {
            let Some(source) = self.source.lock().unwrap().take() else {
                return false;
            };
            output.sink.append(SourceLease {
                source: Some(source),
                shared: self.source.clone(),
            });
            self.attached = true;
        }
        true
    }
}

// Each audio callback holds the consumer directly. The mutex is used only when handing it over.
struct SourceLease {
    source: Option<RadioSource>,
    shared: Arc<Mutex<Option<RadioSource>>>,
}

impl Drop for SourceLease {
    fn drop(&mut self) {
        if let Some(mut source) = self.source.take() {
            // A resampler can stop between channels. Start the next output at a frame boundary.
            for _ in 0..source.frame_remaining {
                let _ = source.next();
            }
            *self.shared.lock().unwrap() = Some(source);
        }
    }
}

impl Iterator for SourceLease {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        self.source.as_mut()?.next()
    }
}

impl Source for SourceLease {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.source.as_ref().unwrap().channels
    }

    fn sample_rate(&self) -> u32 {
        self.source.as_ref().unwrap().rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct EndpointNotifications {
    enumerator: IMMDeviceEnumerator,
    callback: IMMNotificationClient,
    changed: Arc<AtomicBool>,
    _apartment: ComApartment,
}

impl EndpointNotifications {
    fn register() -> WindowsResult<Self> {
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

    fn take_change(&self) -> bool {
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
impl IMMNotificationClient_Impl for DeviceChange {
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
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> WindowsResult<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> WindowsResult<()> {
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> WindowsResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rodio::Source;
    use rtrb::RingBuffer;
    use std::sync::atomic::AtomicU8;

    use super::*;
    use crate::playback::PeakMeter;

    #[test]
    #[ignore = "requires a working Windows playback device"]
    fn opens_wasapi_output_again_after_a_notification() {
        let (mut producer, samples) = RingBuffer::new(4096);
        producer.push_entire_slice(&[0.0; 16]).unwrap();
        let mut output = AudioOutput::open(RadioSource {
            samples,
            volume: Default::default(),
            meter: PeakMeter::new(Default::default(), 44_100),
            cancelled: Default::default(),
            ended: Default::default(),
            channels: 2,
            rate: 44_100,
            frame_remaining: 0,
            silent_frame: false,
        })
        .expect("open WASAPI device");
        let before = output.active.as_ref().unwrap().failed.clone();
        let notifications = output
            .notifications
            .as_ref()
            .expect("register MMDevice callback");
        unsafe {
            notifications
                .callback
                .OnDefaultDeviceChanged(eRender, eConsole, PCWSTR::null())
                .unwrap();
        }
        let texts = crate::i18n::Language::English.texts();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < deadline, "WASAPI reopening timed out");
            assert!(output.ready(texts).unwrap());
            if !Arc::ptr_eq(&output.active.as_ref().unwrap().failed, &before) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!producer.is_abandoned());
    }

    #[test]
    fn changing_output_keeps_the_consumer_at_a_frame_boundary() {
        let (mut producer, samples) = RingBuffer::new(8);
        producer.push_entire_slice(&[0.1, 0.2, 0.3, 0.4]).unwrap();
        let shared = Arc::new(Mutex::new(None));
        let mut old_output = SourceLease {
            source: Some(RadioSource {
                samples,
                volume: Arc::new(AtomicU8::new(100)),
                meter: PeakMeter::new(Default::default(), 44_100),
                cancelled: Default::default(),
                ended: Default::default(),
                channels: 2,
                rate: 44_100,
                frame_remaining: 0,
                silent_frame: false,
            }),
            shared: shared.clone(),
        };
        assert_eq!(old_output.next(), Some(0.1));
        drop(old_output);

        producer.push_entire_slice(&[0.5, 0.6]).unwrap();
        let mut new_output = SourceLease {
            source: shared.lock().unwrap().take(),
            shared: shared.clone(),
        };
        assert_eq!(new_output.channels(), 2);
        assert_eq!(
            new_output.by_ref().take(4).collect::<Vec<_>>(),
            [0.3, 0.4, 0.5, 0.6]
        );
    }

    #[test]
    fn only_default_playback_changes_trigger_a_switch() {
        use windows::Win32::Media::Audio::{eCapture, eCommunications};

        let changed = Arc::new(AtomicBool::new(false));
        let callback = DeviceChange(changed.clone());
        let no_device = PCWSTR::null();
        callback
            .OnDefaultDeviceChanged(eCapture, eConsole, &no_device)
            .unwrap();
        callback
            .OnDefaultDeviceChanged(eRender, eCommunications, &no_device)
            .unwrap();
        assert!(!changed.load(Ordering::Relaxed));
        callback
            .OnDefaultDeviceChanged(eRender, eConsole, &no_device)
            .unwrap();
        assert!(changed.load(Ordering::Relaxed));
    }
}
