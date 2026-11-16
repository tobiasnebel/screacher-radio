use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use fltk::app::Sender;
use rodio::{Decoder, OutputStreamBuilder, Sink, Source};
use rtrb::{Consumer, RingBuffer};
use url::Url;

use crate::{
    metadata::TrackInfo,
    station::{self, BoxError},
    ui::Event,
};

#[derive(Clone)]
pub enum PlaybackEvent {
    Started(Instant),
    TrackChanged(TrackInfo),
    Ended,
    Failed(String),
}

pub struct Playback {
    cancelled: Arc<AtomicBool>,
    volume: Arc<AtomicU8>,
    levels: Arc<AtomicU16>,
}

impl Playback {
    pub fn start(url: Url, volume: u8, id: u64, messages: Sender<Event>) -> io::Result<Self> {
        let playback = Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU8::new(volume)),
            levels: Arc::new(AtomicU16::new(0)),
        };
        let cancelled = playback.cancelled.clone();
        let volume = playback.volume.clone();
        let levels = playback.levels.clone();
        thread::Builder::new()
            .name("radio-playback".into())
            .spawn(move || {
                let result = play(&url, volume, levels, &cancelled, move |event| {
                    messages.send(Event::Playback(id, event));
                });
                if !cancelled.load(Ordering::Relaxed) {
                    let event = match result {
                        Ok(()) => PlaybackEvent::Ended,
                        Err(error) => PlaybackEvent::Failed(error.to_string()),
                    };
                    messages.send(Event::Playback(id, event));
                }
            })?;
        Ok(playback)
    }

    pub fn set_volume(&self, volume: u8) {
        self.volume.store(volume, Ordering::Relaxed);
    }

    pub fn stereo_levels(&self) -> [u8; 2] {
        self.levels.load(Ordering::Relaxed).to_le_bytes()
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

fn play(
    url: &Url,
    volume: Arc<AtomicU8>,
    levels: Arc<AtomicU16>,
    cancelled: &Arc<AtomicBool>,
    notify: impl Fn(PlaybackEvent) + Clone + Send + 'static,
) -> Result<(), BoxError> {
    let opening_deadline = Instant::now() + Duration::from_secs(25);
    let opening = Arc::new(AtomicBool::new(true));
    let is_cancelled = || cancelled.load(Ordering::Relaxed);
    let notify_track = notify.clone();
    let reader = station::open_stream(
        url,
        || is_cancelled() || Instant::now() >= opening_deadline,
        move |track| notify_track(PlaybackEvent::TrackChanged(track)),
    )?;
    let input = StreamReader {
        inner: Mutex::new(reader),
        cancelled: cancelled.clone(),
        opening: opening.clone(),
        opening_deadline,
    };
    let mut decoder = decode_stream(input)
        .map_err(|error| format!("MP3-Stream konnte nicht geöffnet werden ({error})."))?;
    let channels = decoder.channels();
    let rate = decoder.sample_rate();
    let frame_len = usize::from(channels);
    let capacity = frame_len * rate as usize / 2;
    let (mut producer, consumer) = RingBuffer::new(capacity);
    let mut buffer = vec![0.0; 1152 * frame_len];
    let initial = decode_block(&mut decoder, &mut buffer);
    if is_cancelled() {
        return Ok(());
    }
    if initial < frame_len {
        return Err("Der MP3-Stream enthält keine Audiodaten.".into());
    }
    opening.store(false, Ordering::Relaxed);

    // Only the decoder thread reads from the network. The audio callback uses a bounded queue.
    let output_failed = Arc::new(AtomicBool::new(false));
    let report_output_error = output_failed.clone();
    let mut output = OutputStreamBuilder::from_default_device()
        .and_then(|builder| {
            builder
                .with_error_callback(move |_| {
                    report_output_error.store(true, Ordering::Relaxed);
                })
                .open_stream_or_fallback()
        })
        .map_err(|error| format!("Audioausgabe konnte nicht geöffnet werden ({error})."))?;
    output.log_on_drop(false);
    let sink = Sink::connect_new(output.mixer());
    let ended = Arc::new(AtomicBool::new(false));
    producer.push_entire_slice(&buffer[..initial - initial % frame_len])?;
    sink.append(RadioSource {
        samples: consumer,
        volume,
        meter: PeakMeter::new(levels, rate),
        cancelled: cancelled.clone(),
        ended: ended.clone(),
        channels,
        rate,
        frame_remaining: 0,
        silent_frame: false,
    });
    notify(PlaybackEvent::Started(Instant::now()));

    let is_running = || !is_cancelled() && !output_failed.load(Ordering::Relaxed);
    while is_running() {
        let count = decode_block(&mut decoder, &mut buffer);
        let mut remaining = &buffer[..count - count % frame_len];
        while !remaining.is_empty() && is_running() {
            let writable = producer.slots();
            let len = remaining.len().min(writable - writable % frame_len);
            if len == 0 {
                thread::sleep(Duration::from_millis(8));
                continue;
            }
            producer.push_entire_slice(&remaining[..len])?;
            remaining = &remaining[len..];
        }
        if count < buffer.len() {
            break;
        }
    }
    ended.store(true, Ordering::Release);
    while is_running() && !sink.empty() {
        thread::sleep(Duration::from_millis(15));
    }
    if output_failed.load(Ordering::Relaxed) {
        return Err("Verbindung zum Audioausgabegerät verloren.".into());
    }
    Ok(())
}

fn decode_stream<R: Read + Send + 'static>(
    input: StreamReader<R>,
) -> Result<Decoder<StreamReader<R>>, rodio::decoder::DecoderError> {
    Decoder::builder()
        .with_data(input)
        .with_hint("mp3")
        .with_seekable(false)
        .build()
}

fn decode_block(decoder: &mut impl Iterator<Item = f32>, buffer: &mut [f32]) -> usize {
    let mut count = 0;
    for sample in buffer.iter_mut() {
        let Some(decoded) = decoder.next() else { break };
        *sample = decoded;
        count += 1;
    }
    count
}

// Rodio requires Seek and Sync even for sources marked as non-seekable.
struct StreamReader<R> {
    inner: Mutex<R>,
    cancelled: Arc<AtomicBool>,
    opening: Arc<AtomicBool>,
    opening_deadline: Instant,
}

impl<R: Read> Read for StreamReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Ok(0);
        }
        if self.opening.load(Ordering::Relaxed) && Instant::now() >= self.opening_deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Zeitlimit beim Starten des Streams",
            ));
        }
        self.inner.get_mut().unwrap().read(buf)
    }
}

impl<R> Seek for StreamReader<R> {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

struct RadioSource {
    samples: Consumer<f32>,
    volume: Arc<AtomicU8>,
    meter: PeakMeter,
    cancelled: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
    channels: u16,
    rate: u32,
    frame_remaining: u16,
    silent_frame: bool,
}

impl Iterator for RadioSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.cancelled.load(Ordering::Relaxed) {
            return None;
        }
        if self.frame_remaining == 0 {
            let ended = self.ended.load(Ordering::Acquire);
            self.silent_frame = self.samples.slots() < usize::from(self.channels);
            if self.silent_frame && ended {
                return None;
            }
            self.frame_remaining = self.channels;
        }
        self.frame_remaining -= 1;
        let sample = if self.silent_frame {
            0.0
        } else {
            let volume = f32::from(self.volume.load(Ordering::Relaxed)) / 100.0;
            self.samples.pop().ok()? * volume
        };
        let channel = self.channels - self.frame_remaining - 1;
        self.meter.record(sample, channel, self.channels);
        Some(sample)
    }
}

impl Source for RadioSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct PeakMeter {
    levels: Arc<AtomicU16>,
    frames_per_update: u32,
    frames_remaining: u32,
    peaks: [f32; 2],
}

impl PeakMeter {
    fn new(levels: Arc<AtomicU16>, sample_rate: u32) -> Self {
        let frames_per_update = (sample_rate / 40).max(1);
        Self {
            levels,
            frames_per_update,
            frames_remaining: frames_per_update,
            peaks: [0.0; 2],
        }
    }

    fn record(&mut self, sample: f32, channel: u16, channels: u16) {
        if let Some(peak) = self.peaks.get_mut(usize::from(channel)) {
            *peak = peak.max(sample.abs().min(1.0));
        }
        if channel + 1 != channels {
            return;
        }

        self.frames_remaining -= 1;
        if self.frames_remaining == 0 {
            let left = meter_level(self.peaks[0]);
            let right = if channels == 1 {
                left
            } else {
                meter_level(self.peaks[1])
            };
            // Publish both channels together. Decay the peak between 25 ms windows.
            self.levels
                .store(u16::from_le_bytes([left, right]), Ordering::Relaxed);
            self.peaks = self.peaks.map(|peak| peak * 0.8);
            self.frames_remaining = self.frames_per_update;
        }
    }
}

fn meter_level(peak: f32) -> u8 {
    // Map -48 to 0 dBFS to the full width of the bars.
    let db = 20.0 * peak.log10();
    ((db + 48.0) * (100.0 / 48.0)).clamp(0.0, 100.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener};

    #[test]
    fn meters_follow_output_channels_and_volume() {
        for (channels, input, expected) in
            [(2, &[2.0, -0.5][..], [100, 75]), (1, &[-0.5][..], [75, 75])]
        {
            let levels = Arc::new(AtomicU16::new(0));
            let (mut producer, consumer) = RingBuffer::new(4);
            producer.push_entire_slice(input).unwrap();
            let mut source = RadioSource {
                samples: consumer,
                volume: Arc::new(AtomicU8::new(50)),
                meter: PeakMeter::new(levels.clone(), 40),
                cancelled: Arc::new(AtomicBool::new(false)),
                ended: Arc::new(AtomicBool::new(false)),
                channels,
                rate: 40,
                frame_remaining: 0,
                silent_frame: false,
            };
            for &sample in input {
                assert_eq!(source.next(), Some(sample * 0.5));
            }
            assert_eq!(levels.load(Ordering::Relaxed).to_le_bytes(), expected);
            for _ in 0..(26 * channels) {
                assert_eq!(source.next(), Some(0.0));
            }
            assert_eq!(levels.load(Ordering::Relaxed).to_le_bytes(), [0, 0]);
        }
    }

    #[test]
    fn decodes_first_playlist_entry_and_reports_icy_metadata_over_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let playlist = format!("http://{}/list.m3u", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            // MPEG-1 Layer III, 128 kbps, 44.1 kHz stereo. Zeroed side information yields silence.
            let mut frame = [0_u8; 417];
            frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
            for expected in ["/list.m3u", "/live"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(request.starts_with(format!("GET {expected} HTTP/1.1\r\n").as_bytes()));
                assert!(
                    String::from_utf8_lossy(&request)
                        .to_ascii_lowercase()
                        .contains("\r\nicy-metadata: 1\r\n")
                );
                if expected == "/list.m3u" {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/x-mpegurl\r\nConnection: close\r\n\r\n#EXTM3U\n/live\n/unused\n").unwrap();
                } else {
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nicy-metaint: {}\r\nConnection: close\r\n\r\n", frame.len()).unwrap();
                    let mut metadata = [0; 32];
                    let title = b"StreamTitle='Artist - Song';";
                    metadata[..title.len()].copy_from_slice(title);
                    let mut body = Vec::new();
                    for _ in 0..8 {
                        body.extend_from_slice(&frame);
                        body.push(2);
                        body.extend_from_slice(&metadata);
                    }
                    stream.write_all(&body).unwrap();
                }
            }
        });
        let (tracks, received) = std::sync::mpsc::channel();
        let reader = station::open_stream(
            &Url::parse(&playlist).unwrap(),
            || false,
            move |track| {
                tracks.send(track).unwrap();
            },
        )
        .unwrap();
        let mut decoder = decode_stream(StreamReader {
            inner: Mutex::new(reader),
            cancelled: Arc::new(AtomicBool::new(false)),
            opening: Arc::new(AtomicBool::new(true)),
            opening_deadline: Instant::now() + Duration::from_secs(5),
        })
        .unwrap();
        assert_eq!(decoder.sample_rate(), 44_100);
        assert_eq!(decoder.channels(), 2);
        assert_eq!(decoder.by_ref().take(4096).count(), 4096);
        assert_eq!(
            received.try_recv().unwrap(),
            TrackInfo {
                artist: "Artist".into(),
                title: "Song".into()
            }
        );
        server.join().unwrap();
    }
}
