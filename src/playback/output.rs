use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
    time::{Duration, Instant},
};

use rodio::{
    ChannelCount, Device, DeviceSinkBuilder, DeviceSinkError, DeviceTrait, MixerDeviceSink, Player,
    SampleRate, Source,
    cpal::{self, DeviceId, DevicesError, traits::HostTrait},
};

#[cfg(windows)]
use super::windows_output::EndpointNotifications;
use super::{BoxError, PlaybackEvent, RadioSource};

const DEVICE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const RETRY_INTERVAL: Duration = Duration::from_millis(500);

#[derive(PartialEq, Eq)]
pub struct OutputDevice {
    pub id: String,
    pub label: String,
}

pub fn output_devices() -> Result<Vec<OutputDevice>, DevicesError> {
    let mut devices: Vec<_> = cpal::default_host()
        .output_devices()?
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            let label = device
                .description()
                .map_or_else(|_| id.clone(), |description| description.name().to_owned());
            Some(OutputDevice { id, label })
        })
        .collect();
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    devices.dedup_by(|a, b| a.id == b.id);
    devices.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(devices)
}

pub(super) struct AudioOutput {
    active: Option<ActiveOutput>,
    attached: bool,
    source: Arc<Mutex<Option<RadioSource>>>,
    preferred: Option<String>,
    selections: Receiver<Option<String>>,
    check_at: Option<Instant>,
    #[cfg(windows)]
    notifications: Option<EndpointNotifications>,
}

impl AudioOutput {
    pub(super) fn open(
        source: RadioSource,
        preferred: Option<String>,
        selections: Receiver<Option<String>>,
        notify: &impl Fn(PlaybackEvent),
    ) -> Result<Self, BoxError> {
        let preferred = selections.try_iter().last().unwrap_or(preferred);
        let mut output = Self {
            active: None,
            attached: false,
            source: Arc::new(Mutex::new(Some(source))),
            preferred,
            selections,
            check_at: None,
            // Subscribe first to catch changes while opening the initial output.
            #[cfg(windows)]
            notifications: EndpointNotifications::register().ok(),
        };
        output.select_device(notify)?;
        output.schedule_device_check();
        output.attach_source();
        Ok(output)
    }

    pub(super) fn ready(&mut self, notify: &impl Fn(PlaybackEvent)) -> bool {
        let now = Instant::now();
        if let Some(preferred) = self.selections.try_iter().last() {
            self.preferred = preferred;
            self.check_at = Some(now);
        }
        #[cfg(windows)]
        if self
            .notifications
            .as_ref()
            .is_some_and(EndpointNotifications::take_change)
        {
            self.check_at = Some(now);
        }
        if self.active.as_ref().is_some_and(ActiveOutput::failed) {
            self.active = None;
            self.attached = false;
            self.check_at = Some(now);
        }
        if self.check_at.is_some_and(|at| now >= at) {
            match self.select_device(notify) {
                Ok(()) => self.schedule_device_check(),
                Err(_) => self.check_at = Some(Instant::now() + RETRY_INTERVAL),
            }
        }
        if self.active.is_none()
            && let Some(source) = self.source.lock().unwrap().as_mut()
        {
            // Discard audio produced during an outage so playback resumes near the live edge.
            let queued = source.samples.slots();
            source.samples.read_chunk(queued).unwrap().commit_all();
            source.meter.levels.store(0, Ordering::Relaxed);
        }
        self.attach_source()
    }

    pub(super) fn drained(&self) -> bool {
        self.active
            .as_ref()
            .is_none_or(|output| output.player.empty())
    }

    fn schedule_device_check(&mut self) {
        #[cfg(windows)]
        let needs_polling = self.notifications.is_none();
        #[cfg(not(windows))]
        let needs_polling = self.preferred.is_some();
        self.check_at = needs_polling.then(|| Instant::now() + DEVICE_CHECK_INTERVAL);
    }

    fn select_device(&mut self, notify: &impl Fn(PlaybackEvent)) -> Result<(), BoxError> {
        let host = cpal::default_host();
        if let Some(preferred) = &self.preferred {
            // Use the active output list so an unplugged device is not treated as available.
            let selected = preferred.parse::<DeviceId>().ok();
            if let Ok(mut available) = host.output_devices() {
                if let Some(device) = selected.and_then(|id| {
                    available.find(|device| device.id().is_ok_and(|candidate| candidate == id))
                }) && self.use_device(device).is_ok()
                {
                    return Ok(());
                }
                if let Some(unavailable) = self.preferred.take() {
                    notify(PlaybackEvent::AudioDeviceUnavailable(unavailable));
                }
            }
        }
        let device = host
            .default_output_device()
            .ok_or(DeviceSinkError::NoDevice)?;
        self.use_device(device).map_err(Into::into)
    }

    fn use_device(&mut self, device: Device) -> Result<(), DeviceSinkError> {
        let id = device.id().ok();
        if id.is_some() && self.active.as_ref().is_some_and(|active| active.id == id) {
            return Ok(());
        }
        self.active = Some(ActiveOutput::open(device, id)?);
        self.attached = false;
        Ok(())
    }

    fn attach_source(&mut self) -> bool {
        let Some(output) = &self.active else {
            return false;
        };
        if !self.attached {
            let Some(source) = self.source.lock().unwrap().take() else {
                return false;
            };
            output.player.append(SourceLease {
                source: Some(source),
                shared: self.source.clone(),
            });
            self.attached = true;
        }
        true
    }
}

struct ActiveOutput {
    player: Player,
    _stream: MixerDeviceSink,
    id: Option<DeviceId>,
    failed: Arc<AtomicBool>,
}

impl ActiveOutput {
    fn open(device: Device, id: Option<DeviceId>) -> Result<Self, DeviceSinkError> {
        let failed = Arc::new(AtomicBool::new(false));
        let report_error = failed.clone();
        let mut stream = DeviceSinkBuilder::from_device(device)?
            .with_error_callback(move |_| report_error.store(true, Ordering::Relaxed))
            .open_sink_or_fallback()?;
        stream.log_on_drop(false);
        Ok(Self {
            player: Player::connect_new(stream.mixer()),
            _stream: stream,
            id,
            failed,
        })
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

// The audio callback holds the consumer directly. Only the handover uses the mutex.
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

    fn channels(&self) -> ChannelCount {
        self.source.as_ref().unwrap().channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.source.as_ref().unwrap().rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU8;

    use rtrb::RingBuffer;

    use super::*;
    use crate::playback::PeakMeter;

    fn stereo_source(samples: rtrb::Consumer<f32>) -> RadioSource {
        RadioSource {
            samples,
            volume: Arc::new(AtomicU8::new(100)),
            meter: PeakMeter::new(Default::default(), 44_100),
            cancelled: Default::default(),
            ended: Default::default(),
            channels: ChannelCount::new(2).unwrap(),
            rate: SampleRate::new(44_100).unwrap(),
            frame_remaining: 0,
            silent_frame: false,
        }
    }

    #[test]
    fn changing_output_keeps_the_consumer_at_a_frame_boundary() {
        let (mut producer, samples) = RingBuffer::new(8);
        producer.push_entire_slice(&[0.1, 0.2, 0.3, 0.4]).unwrap();
        let shared = Arc::new(Mutex::new(None));
        let mut old_output = SourceLease {
            source: Some(stereo_source(samples)),
            shared: shared.clone(),
        };
        assert_eq!(old_output.next(), Some(0.1));
        drop(old_output);

        producer.push_entire_slice(&[0.5, 0.6]).unwrap();
        let new_output = SourceLease {
            source: shared.lock().unwrap().take(),
            shared: shared.clone(),
        };
        assert_eq!(new_output.channels().get(), 2);
        assert_eq!(new_output.take(4).collect::<Vec<_>>(), [0.3, 0.4, 0.5, 0.6]);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a working Windows playback device"]
    fn selects_an_output_and_falls_back_to_default() {
        use std::{cell::RefCell, sync::mpsc};

        let host = cpal::default_host();
        let default = host.default_output_device().unwrap().id().unwrap();
        let devices = output_devices().unwrap();
        assert!(
            devices
                .iter()
                .any(|device| device.id == default.to_string())
        );

        let (mut producer, samples) = RingBuffer::new(4096);
        producer.push_entire_slice(&[0.0; 16]).unwrap();
        let (selection, selections) = mpsc::channel();
        let unavailable = RefCell::new(Vec::new());
        let notify = |event| {
            if let PlaybackEvent::AudioDeviceUnavailable(id) = event {
                unavailable.borrow_mut().push(id);
            }
        };
        let mut output = AudioOutput::open(
            stereo_source(samples),
            Some("wasapi:missing-on-start".into()),
            selections,
            &notify,
        )
        .unwrap();
        assert_eq!(unavailable.borrow().as_slice(), ["wasapi:missing-on-start"]);
        assert_eq!(output.active.as_ref().unwrap().id.as_ref(), Some(&default));

        selection.send(Some(default.to_string())).unwrap();
        assert!(output.ready(&notify));
        assert_eq!(
            output.preferred.as_deref(),
            Some(default.to_string().as_str())
        );
        selection
            .send(Some("wasapi:missing-during-playback".into()))
            .unwrap();
        assert!(output.ready(&notify));
        assert_eq!(output.preferred, None);
        assert_eq!(
            unavailable.borrow().as_slice(),
            ["wasapi:missing-on-start", "wasapi:missing-during-playback"]
        );
        assert_eq!(output.active.as_ref().unwrap().id.as_ref(), Some(&default));

        let before = output.active.as_ref().unwrap().failed.clone();
        before.store(true, Ordering::Relaxed);
        assert!(output.ready(&notify));
        assert!(!Arc::ptr_eq(
            &output.active.as_ref().unwrap().failed,
            &before
        ));
        assert!(!producer.is_abandoned());
    }
}
