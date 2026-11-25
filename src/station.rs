use std::{
    error::Error,
    io::{self, BufRead, BufReader, Read},
    num::NonZeroUsize,
    time::Duration,
};

use ureq::{Agent, ResponseExt, http::header::CONTENT_TYPE};
use url::Url;

use crate::{
    i18n::Texts,
    metadata::{IcyReader, TrackInfo},
};

pub type BoxError = Box<dyn Error + Send + Sync>;
const PLAYLIST_LIMIT: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub struct StationInfo {
    pub streams: Vec<Url>,
    pub content: String,
}

pub fn inspect(url: &Url, texts: &Texts) -> Result<StationInfo, BoxError> {
    let agent: Agent = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_per_read(Some(Duration::from_secs(8)))
        .max_redirects(5)
        .user_agent("ScreacherRadio/0.1")
        .build()
        .into();
    let response = agent.get(url.as_str()).call()?;
    let base = station_url(&response.get_uri().to_string(), texts)?;
    let mime = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let declared = playlist_format(&base, mime).or_else(|| playlist_format(url, ""));
    let mut body = BufReader::new(response.into_body().into_reader());
    let format = match declared {
        Some(format) => Some(format),
        None => sniff_playlist(body.fill_buf()?),
    };
    let Some(format) = format else {
        return Ok(StationInfo {
            streams: vec![url.clone()],
            content: url.to_string(),
        });
    };
    let mut data = Vec::new();
    body.take((PLAYLIST_LIMIT + 1) as u64)
        .read_to_end(&mut data)?;
    if data.len() > PLAYLIST_LIMIT {
        return Err(texts.playlist_too_large.into());
    }
    let content = String::from_utf8_lossy(&data).into_owned();
    let streams = playlist_streams(&base, format, &content, texts)?;
    Ok(StationInfo { streams, content })
}

#[derive(Clone, Copy)]
enum Playlist {
    M3u,
    Pls,
}

pub fn station_url(input: &str, texts: &Texts) -> Result<Url, BoxError> {
    validate_url(Url::parse(input.trim())?, texts)
}

fn validate_url(mut url: Url, texts: &Texts) -> Result<Url, BoxError> {
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(texts.http_required.into());
    }
    url.set_fragment(None);
    Ok(url)
}

pub fn open_stream(
    url: &Url,
    texts: &Texts,
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
    let mut url = validate_url(url.clone(), texts)?;
    // A station can point at a playlist. Its first entry must resolve to audio.
    for allow_playlist in [true, false] {
        if cancelled() {
            return Err(io::Error::other(texts.playback_cancelled).into());
        }
        let response = agent.get(url.as_str()).header("icy-metadata", "1").call()?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()).into());
        }
        let base = station_url(&response.get_uri().to_string(), texts)?;
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
            return Err(texts.nested_playlist.into());
        }
        let mut data = Vec::new();
        let mut limited = body.take((PLAYLIST_LIMIT + 1) as u64);
        let mut chunk = [0; 4096];
        loop {
            if cancelled() {
                return Err(texts.playlist_timeout.into());
            }
            let read = limited.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            data.extend_from_slice(&chunk[..read]);
        }
        if data.len() > PLAYLIST_LIMIT {
            return Err(texts.playlist_too_large.into());
        }
        url = first_stream(&base, format, &String::from_utf8_lossy(&data), texts)?;
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

fn first_stream(base: &Url, format: Playlist, text: &str, texts: &Texts) -> Result<Url, BoxError> {
    let entries = playlist_entries(format, text, texts)?;
    validate_url(base.join(entries[0])?, texts)
}

fn playlist_entries<'a>(
    format: Playlist,
    text: &'a str,
    texts: &Texts,
) -> Result<Vec<&'a str>, BoxError> {
    let lines = text.trim_start_matches('\u{feff}').lines().map(str::trim);
    let entries: Vec<_> = match format {
        Playlist::M3u => {
            if lines.clone().any(|line| line.starts_with("#EXT-X-")) {
                return Err(texts.hls_unsupported.into());
            }
            lines
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect()
        }
        Playlist::Pls => {
            let mut entries: Vec<_> = lines
                .filter_map(|line| {
                    let (key, value) = line.split_once('=')?;
                    let key = key.trim().to_ascii_lowercase();
                    let index = key.strip_prefix("file")?.parse::<u32>().ok()?;
                    (index > 0 && !value.trim().is_empty()).then_some((index, value.trim()))
                })
                .collect();
            entries.sort_by_key(|&(index, _)| index);
            entries.dedup_by_key(|entry| entry.0);
            entries.into_iter().map(|(_, value)| value).collect()
        }
    };
    if entries.is_empty() {
        return Err(texts.playlist_empty.into());
    }
    Ok(entries)
}

fn playlist_streams(
    base: &Url,
    format: Playlist,
    text: &str,
    texts: &Texts,
) -> Result<Vec<Url>, BoxError> {
    // Match playback's first-stream validation; malformed fallback entries are not selectable streams.
    let first = first_stream(base, format, text, texts)?;
    let mut streams = vec![first];
    streams.extend(
        playlist_entries(format, text, texts)?
            .into_iter()
            .skip(1)
            .filter_map(|entry| {
                base.join(entry)
                    .ok()
                    .and_then(|url| validate_url(url, texts).ok())
            }),
    );
    Ok(streams)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn counts_playlist_streams_in_playback_order() {
        let texts = Language::English.texts();
        let base = Url::parse("https://example.org/radio/list.pls").unwrap();
        let streams = playlist_streams(&base, Playlist::Pls,
            "[playlist]\nFile4=/four\nFILE1=../one\nFile2=https://example.net/two\nNumberOfEntries=99\n", texts).unwrap();
        assert_eq!(
            streams.iter().map(Url::as_str).collect::<Vec<_>>(),
            [
                "https://example.org/one",
                "https://example.net/two",
                "https://example.org/four"
            ]
        );
        let streams = playlist_streams(
            &base,
            Playlist::M3u,
            "#EXTM3U\n#EXTINF:-1,One\n../one\n\n# Comment\n/two\n",
            texts,
        )
        .unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].as_str(), "https://example.org/one");
    }

    #[test]
    fn inspects_http_playlists_and_does_not_display_audio_as_text() {
        use std::{io::Write, net::TcpListener, thread};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fixtures = [
            ("/radio.m3u", "audio/x-mpegurl", "#EXTM3U\n/one\n/two\n"),
            (
                "/radio.pls",
                "audio/x-scpls",
                "[playlist]\nFile2=/two\nFile1=/one\n",
            ),
            ("/live", "audio/mpeg", "audio bytes"),
        ];
        let server = thread::spawn(move || {
            for (path, mime, body) in fixtures {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "missing HTTP request");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(&stream).read_line(&mut request).unwrap();
                assert!(request.starts_with(&format!("GET {path} ")));
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let texts = Language::English.texts();
        for (path, _, body) in fixtures {
            let url = Url::parse(&format!("{base}{path}")).unwrap();
            let info = inspect(&url, texts).unwrap();
            if path == "/live" {
                assert_eq!(info.streams, vec![url.clone()]);
                assert_eq!(info.content, url.as_str());
            } else {
                assert_eq!(info.streams.len(), 2);
                assert_eq!(info.streams[0].as_str(), format!("{base}/one"));
                assert_eq!(info.content, body);
            }
        }
        server.join().unwrap();
    }

    #[test]
    fn m3u_uses_first_entry_and_resolves_relative_urls() {
        let texts = Language::English.texts();
        let base = station_url("https://example.net/radio/list.m3u", texts).unwrap();
        let playlist = "\u{feff}#EXTM3U\r\n\r\n#EXTINF:-1, Station\r\n../live.mp3\r\nhttps://example.org/backup\r\n";
        assert_eq!(
            first_stream(&base, Playlist::M3u, playlist, texts)
                .unwrap()
                .as_str(),
            "https://example.net/live.mp3"
        );
    }

    #[test]
    fn pls_uses_lowest_file_index() {
        let texts = Language::English.texts();
        let base = station_url("http://example.net/list.pls", texts).unwrap();
        let playlist =
            "[playlist]\nFile2=http://example.net/backup\nFILE1 = https://example.org/live\n";
        assert_eq!(
            first_stream(&base, Playlist::Pls, playlist, texts)
                .unwrap()
                .as_str(),
            "https://example.org/live"
        );
    }

    #[test]
    fn rejects_hls_and_non_http_entries() {
        let texts = Language::English.texts();
        let base = station_url("http://example.net/radio", texts).unwrap();
        assert!(
            first_stream(
                &base,
                Playlist::M3u,
                "#EXTM3U\n#EXT-X-TARGETDURATION:6\nsegment.ts",
                texts
            )
            .is_err()
        );
        assert!(first_stream(&base, Playlist::M3u, "file:///tmp/audio.mp3", texts).is_err());
        assert!(station_url("file:///tmp/radio.m3u", texts).is_err());
    }
}
