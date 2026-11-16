use std::{
    error::Error,
    io::{self, BufRead, BufReader, Read},
    num::NonZeroUsize,
    time::Duration,
};

use ureq::{Agent, ResponseExt, http::header::CONTENT_TYPE};
use url::Url;

use crate::metadata::{IcyReader, TrackInfo};

pub type BoxError = Box<dyn Error + Send + Sync>;
const PLAYLIST_LIMIT: usize = 256 * 1024;

#[derive(Clone, Copy)]
enum Playlist {
    M3u,
    Pls,
}

pub fn station_url(input: &str) -> Result<Url, BoxError> {
    validate_url(Url::parse(input.trim())?)
}

fn validate_url(mut url: Url) -> Result<Url, BoxError> {
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err("Die URL muss mit http:// oder https:// beginnen.".into());
    }
    url.set_fragment(None);
    Ok(url)
}

pub fn open_stream(
    url: &Url,
    cancelled: impl Fn() -> bool,
    on_track: impl FnMut(TrackInfo) + Send + 'static,
) -> Result<impl Read + Send + 'static, BoxError> {
    let agent: Agent = Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_resolve(Some(Duration::from_secs(8)))
        .timeout_recv_response(Some(Duration::from_secs(10)))
        .timeout_per_read(Some(Duration::from_secs(10)))
        .max_redirects(5)
        .user_agent("ScreacherRadio/0.1")
        .build()
        .into();
    let mut url = validate_url(url.clone())?;
    // A station can point at a playlist. Its first entry must resolve to audio.
    for allow_playlist in [true, false] {
        if cancelled() {
            return Err(io::Error::other("Wiedergabe abgebrochen").into());
        }
        let response = agent.get(url.as_str()).header("icy-metadata", "1").call()?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()).into());
        }
        let base = station_url(&response.get_uri().to_string())?;
        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let declared_format = playlist_format(&base, mime).or_else(|| playlist_format(&url, ""));
        let metadata_interval = response
            .headers()
            .get("icy-metaint")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<NonZeroUsize>().ok());
        let mut body = BufReader::new(response.into_body().into_reader());
        let format = match declared_format {
            Some(format) => Some(format),
            None => sniff_playlist(body.fill_buf()?),
        };
        let Some(format) = format else {
            return Ok(IcyReader::new(body, metadata_interval, on_track));
        };
        if !allow_playlist {
            return Err("Der erste Playlist-Eintrag verweist auf eine weitere Playlist.".into());
        }
        let mut data = Vec::new();
        let mut limited = body.take((PLAYLIST_LIMIT + 1) as u64);
        let mut chunk = [0; 4096];
        loop {
            if cancelled() {
                return Err(
                    "Zeitlimit beim Laden der Playlist oder Wiedergabe abgebrochen.".into(),
                );
            }
            let read = limited.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            data.extend_from_slice(&chunk[..read]);
        }
        if data.len() > PLAYLIST_LIMIT {
            return Err("Die Playlist ist größer als 256 KiB.".into());
        }
        url = first_stream(&base, format, &String::from_utf8_lossy(&data))?;
    }
    unreachable!("second request returns either audio or an error")
}

fn playlist_format(url: &Url, mime: &str) -> Option<Playlist> {
    match mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "audio/x-scpls" | "application/pls+xml" | "application/x-scpls" => {
            return Some(Playlist::Pls);
        }
        "audio/mpegurl"
        | "audio/x-mpegurl"
        | "application/x-mpegurl"
        | "application/vnd.apple.mpegurl" => return Some(Playlist::M3u),
        _ => {}
    }
    let (_, extension) = url.path().rsplit_once('.')?;
    match extension.to_ascii_lowercase().as_str() {
        "m3u" | "m3u8" => Some(Playlist::M3u),
        "pls" => Some(Playlist::Pls),
        _ => None,
    }
}

fn sniff_playlist(data: &[u8]) -> Option<Playlist> {
    let data = data
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(data)
        .trim_ascii_start();
    if data.starts_with(b"#EXTM3U") {
        Some(Playlist::M3u)
    } else if data
        .get(..10)
        .is_some_and(|start| start.eq_ignore_ascii_case(b"[playlist]"))
    {
        Some(Playlist::Pls)
    } else {
        None
    }
}

fn first_stream(base: &Url, format: Playlist, text: &str) -> Result<Url, BoxError> {
    let mut lines = text.trim_start_matches('\u{feff}').lines().map(str::trim);
    let entry = match format {
        Playlist::M3u => {
            if lines.clone().any(|line| line.starts_with("#EXT-X-")) {
                return Err("HLS-Playlists werden nicht unterstützt.".into());
            }
            lines.find(|line| !line.is_empty() && !line.starts_with('#'))
        }
        Playlist::Pls => lines
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                let key = key.trim().to_ascii_lowercase();
                let index = key.strip_prefix("file")?.parse::<u32>().ok()?;
                (index > 0 && !value.trim().is_empty()).then_some((index, value.trim()))
            })
            .min_by_key(|&(index, _)| index)
            .map(|(_, value)| value),
    };
    let entry = entry.ok_or("Die Playlist enthält keinen Stream.")?;
    validate_url(base.join(entry)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u_uses_first_entry_and_resolves_relative_urls() {
        let base = station_url("https://example.net/radio/list.m3u").unwrap();
        let playlist = "\u{feff}#EXTM3U\r\n\r\n#EXTINF:-1, Station\r\n../live.mp3\r\nhttps://example.org/backup\r\n";
        assert_eq!(
            first_stream(&base, Playlist::M3u, playlist)
                .unwrap()
                .as_str(),
            "https://example.net/live.mp3"
        );
    }

    #[test]
    fn pls_uses_lowest_file_index() {
        let base = station_url("http://example.net/list.pls").unwrap();
        let playlist =
            "[playlist]\nFile2=http://example.net/backup\nFILE1 = https://example.org/live\n";
        assert_eq!(
            first_stream(&base, Playlist::Pls, playlist)
                .unwrap()
                .as_str(),
            "https://example.org/live"
        );
    }

    #[test]
    fn rejects_hls_and_non_http_entries() {
        let base = station_url("http://example.net/radio").unwrap();
        assert!(
            first_stream(
                &base,
                Playlist::M3u,
                "#EXTM3U\n#EXT-X-TARGETDURATION:6\nsegment.ts"
            )
            .is_err()
        );
        assert!(first_stream(&base, Playlist::M3u, "file:///tmp/audio.mp3").is_err());
        assert!(station_url("file:///tmp/radio.m3u").is_err());
    }
}
