use std::{
    borrow::Cow,
    io::{self, Read},
    num::NonZeroUsize,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
}

pub struct IcyReader<R, F> {
    inner: R,
    interval: Option<NonZeroUsize>,
    remaining: usize,
    track: TrackInfo,
    on_track: F,
}

impl<R, F> IcyReader<R, F> {
    pub fn new(inner: R, interval: Option<NonZeroUsize>, on_track: F) -> Self {
        Self {
            inner,
            interval,
            remaining: interval.map_or(0, NonZeroUsize::get),
            track: TrackInfo::default(),
            on_track,
        }
    }
}

impl<R: Read, F: FnMut(TrackInfo)> Read for IcyReader<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(interval) = self.interval else {
            return self.inner.read(buf);
        };
        if buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut length = [0];
            if self.inner.read(&mut length)? == 0 {
                return Ok(0);
            }
            if length[0] != 0 {
                let mut block = [0; 255 * 16];
                let data = &mut block[..usize::from(length[0]) * 16];
                self.inner.read_exact(data)?;
                if let Some(track) = parse_track(data)
                    && track != self.track
                {
                    self.track = track.clone();
                    (self.on_track)(track);
                }
            }
            self.remaining = interval.get();
        }
        let limit = buf.len().min(self.remaining);
        let count = self.inner.read(&mut buf[..limit])?;
        self.remaining -= count;
        Ok(count)
    }
}

fn parse_track(data: &[u8]) -> Option<TrackInfo> {
    let data = data.split(|&byte| byte == 0).next()?;
    let text = match std::str::from_utf8(data) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => Cow::Owned(data.iter().map(|&byte| char::from(byte)).collect()),
    };
    let (_, fields) = text.split_once("StreamTitle='")?;
    let (value, _) = fields.split_once("';")?;
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let (artist, title) = value.split_once(" - ").unwrap_or(("", &value));
    Some(TrackInfo {
        title: title.into(),
        artist: artist.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_icy_blocks_and_reports_track_changes() {
        let mut wire = Vec::new();
        for (audio, metadata) in [
            (
                b"abc",
                b"StreamTitle='Bj\xf6rk - J\xf3ga';StreamUrl='';".as_slice(),
            ),
            (b"def", b""),
            (b"ghi", b"StreamTitle='Bj\xf6rk - J\xf3ga';"),
            (b"jkl", b"StreamTitle='Nur Titel';"),
            (b"mno", b"StreamTitle='';"),
        ] {
            wire.extend_from_slice(audio);
            let length = metadata.len().div_ceil(16);
            wire.push(length as u8);
            wire.extend_from_slice(metadata);
            wire.resize(wire.len() + length * 16 - metadata.len(), 0);
        }
        wire.extend_from_slice(b"pq");
        let mut tracks = Vec::new();
        let mut reader = IcyReader::new(wire.as_slice(), NonZeroUsize::new(3), |track| {
            tracks.push(track);
        });
        let mut audio = Vec::new();
        reader.read_to_end(&mut audio).unwrap();
        assert_eq!(audio, b"abcdefghijklmnopq");
        assert_eq!(
            tracks,
            [
                TrackInfo {
                    artist: "Björk".into(),
                    title: "Jóga".into()
                },
                TrackInfo {
                    artist: String::new(),
                    title: "Nur Titel".into()
                },
                TrackInfo::default(),
            ]
        );
    }
}
