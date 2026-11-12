use crate::media::mpris_watcher::TrackMetadata;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricWord {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
    pub words: Vec<LyricWord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedLyrics {
    pub source: String,
    pub lines: Vec<LyricLine>,
}

impl SyncedLyrics {
    /// Binary search for the active lyric line at playback time `pos`
    pub fn active_line_index(&self, pos: Duration) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        match self.lines.binary_search_by(|line| line.start.cmp(&pos)) {
            Ok(idx) => Some(idx),
            Err(0) => Some(0),
            Err(insert_idx) => Some(insert_idx - 1),
        }
    }
}

// --- Sonora SQLite JSON schema structures ---

#[derive(Deserialize)]
struct SonoraLyricsRecord {
    #[serde(default)]
    hits: Vec<SonoraHit>,
}

#[derive(Deserialize)]
struct SonoraHit {
    #[serde(default)]
    source: String,
    #[serde(default)]
    trust: i64,
    lyrics: SonoraLyricsKind,
}

#[derive(Deserialize)]
enum SonoraLyricsKind {
    Synced { lines: Vec<SonoraLine> },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct SonoraDuration {
    secs: u64,
    nanos: u32,
}

impl SonoraDuration {
    fn into_std(self) -> Duration {
        Duration::new(self.secs, self.nanos)
    }
}

#[derive(Deserialize)]
struct SonoraWord {
    start: SonoraDuration,
    end: SonoraDuration,
    text: String,
}

#[derive(Deserialize)]
struct SonoraLine {
    start: SonoraDuration,
    end: SonoraDuration,
    text: String,
    #[serde(default)]
    words: Vec<SonoraWord>,
}

/// Extracts the Sonora SQLite cache key from `xesam:url` (e.g. `https://open.spotify.com/track/<id>` -> `spotify:<id>:lyrics`)
pub fn extract_sonora_cache_key(source_url: &str) -> Option<String> {
    if let Some(rest) = source_url.split("open.spotify.com/track/").nth(1) {
        let id = rest.split(['?', '/', '#']).next()?.trim();
        if !id.is_empty() {
            return Some(format!("spotify:{id}:lyrics"));
        }
    }
    if let Some(rest) = source_url.strip_prefix("spotify:track:") {
        let id = rest.trim();
        if !id.is_empty() {
            return Some(format!("spotify:{id}:lyrics"));
        }
    }
    None
}

/// Parses a JSON entry from the `lyrics` table in `~/.cache/sonora/cache.sqlite`
pub fn parse_sonora_lyrics_json(raw_json: &str) -> Option<SyncedLyrics> {
    let record: SonoraLyricsRecord = serde_json::from_str(raw_json).ok()?;

    let mut best_hit: Option<(i64, usize, SyncedLyrics)> = None;

    for hit in record.hits {
        let SonoraLyricsKind::Synced { lines } = hit.lyrics else {
            continue;
        };
        if lines.is_empty() {
            continue;
        }

        let mut parsed_lines = Vec::with_capacity(lines.len());
        let mut word_count = 0usize;

        for l in lines {
            let text = l.text.trim().to_string();
            let words: Vec<LyricWord> = l
                .words
                .into_iter()
                .map(|w| LyricWord {
                    start: w.start.into_std(),
                    end: w.end.into_std(),
                    text: w.text,
                })
                .collect();
            word_count += words.len();
            parsed_lines.push(LyricLine {
                start: l.start.into_std(),
                end: l.end.into_std(),
                text,
                words,
            });
        }

        parsed_lines.sort_by_key(|l| l.start);

        let candidate = SyncedLyrics {
            source: format!("Sonora ({})", hit.source),
            lines: parsed_lines,
        };

        let rank = (hit.trust, word_count);
        if best_hit
            .as_ref()
            .is_none_or(|(best_t, best_w, _)| rank > (*best_t, *best_w))
        {
            best_hit = Some((hit.trust, word_count, candidate));
        }
    }

    best_hit.map(|(_, _, synced)| synced)
}

/// Reads synchronized lyrics directly from `~/.cache/sonora/cache.sqlite` in READ_ONLY mode
pub fn query_sonora_sqlite(db_path: &Path, cache_key: &str) -> Option<SyncedLyrics> {
    if !db_path.exists() {
        return None;
    }
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;

    let raw_value: String = conn
        .query_row(
            "SELECT value FROM lyrics WHERE key = ?1 LIMIT 1",
            [cache_key],
            |row| row.get(0),
        )
        .ok()?;

    parse_sonora_lyrics_json(&raw_value)
}

pub fn default_sonora_db_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("sonora").join("cache.sqlite"))
}

/// Parses a standard `.lrc` file or response (`[mm:ss.xx] Text`)
pub fn parse_lrc(lrc_text: &str, source_label: &str) -> Option<SyncedLyrics> {
    let mut lines: Vec<LyricLine> = Vec::new();

    for raw_line in lrc_text.lines() {
        let trimmed = raw_line.trim();
        if !trimmed.starts_with('[') {
            continue;
        }
        let Some(close_bracket) = trimmed.find(']') else {
            continue;
        };
        let tag = &trimmed[1..close_bracket];
        let text = trimmed[close_bracket + 1..].trim().to_string();

        let Some(start) = parse_lrc_timestamp(tag) else {
            continue;
        };

        lines.push(LyricLine {
            start,
            end: start + Duration::from_secs(4),
            text,
            words: Vec::new(),
        });
    }

    if lines.is_empty() {
        return None;
    }

    lines.sort_by_key(|l| l.start);
    for i in 0..lines.len().saturating_sub(1) {
        lines[i].end = lines[i + 1].start;
    }

    Some(SyncedLyrics {
        source: source_label.to_string(),
        lines,
    })
}

fn parse_lrc_timestamp(tag: &str) -> Option<Duration> {
    let (mins_str, secs_str) = tag.split_once(':')?;
    let mins: u64 = mins_str.trim().parse().ok()?;
    let secs: f64 = secs_str.trim().replace(',', ".").parse().ok()?;
    if secs < 0.0 || !secs.is_finite() {
        return None;
    }
    Some(Duration::from_secs(mins * 60) + Duration::from_secs_f64(secs))
}

#[derive(Deserialize)]
struct LrcLibResponse {
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

/// Strips common YouTube / YouTube Music noise suffixes (e.g. " - Topic", "(Official Video)")
fn clean_youtube_metadata(artist: &str, title: &str) -> (String, String) {
    let clean_artist = artist
        .trim()
        .strip_suffix(" - Topic")
        .unwrap_or(artist.trim())
        .to_string();

    let base_title = if let Some((maybe_art, rest)) = title.split_once(" - ") {
        if clean_artist.is_empty()
            || clean_artist
                .to_lowercase()
                .contains(&maybe_art.trim().to_lowercase())
        {
            rest.trim()
        } else {
            title.trim()
        }
    } else {
        title.trim()
    };

    let mut cleaned_title = base_title.to_string();
    for noise in [
        "(Official Video)",
        "(Official Music Video)",
        "(Official Audio)",
        "(Lyrics)",
        "(Lyric Video)",
        "(Clip Officiel)",
        "[Official Video]",
        "[Clip Officiel]",
        "(Visualizer)",
        "(HD)",
        "(4K)",
    ] {
        if let Some(pos) = cleaned_title.to_lowercase().find(&noise.to_lowercase()) {
            cleaned_title.replace_range(pos..pos + noise.len(), "");
        }
    }

    (clean_artist, cleaned_title.trim().to_string())
}

async fn fetch_lrclib(client: &reqwest::Client, meta: &TrackMetadata) -> Option<SyncedLyrics> {
    if meta.title.is_empty() && meta.artist.is_empty() {
        return None;
    }

    let (clean_artist, clean_title) = clean_youtube_metadata(&meta.artist, &meta.title);

    // 1. Check local disk cache `~/.cache/kymat/lyrics/`
    let safe_name = format!(
        "{}_{}.lrc",
        sanitize_filename(&clean_artist),
        sanitize_filename(&clean_title)
    );
    let cache_file = dirs::cache_dir().map(|d| d.join("kymat").join("lyrics").join(&safe_name));

    if let Some(ref path) = cache_file {
        if let Ok(cached_lrc) = fs::read_to_string(path) {
            if let Some(parsed) = parse_lrc(&cached_lrc, "LRCLIB (Cache)") {
                return Some(parsed);
            }
        }
    }

    // 2. Exact HTTP lookup via https://lrclib.net/api/get
    let mut lrc_raw: Option<String> = None;
    if !clean_artist.is_empty() && !clean_title.is_empty() {
        let mut url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}",
            urlencoding::encode(&clean_artist),
            urlencoding::encode(&clean_title)
        );
        if !meta.album.is_empty() {
            url.push_str(&format!("&album_name={}", urlencoding::encode(&meta.album)));
        }
        if !meta.duration.is_zero() {
            url.push_str(&format!("&duration={}", meta.duration.as_secs()));
        }

        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(parsed) = resp.json::<LrcLibResponse>().await {
                lrc_raw = parsed.synced_lyrics.filter(|s| !s.trim().is_empty());
            }
        }
    }

    // 3. Universal `/api/search` fallback (for YouTube, YouTube Music, Deezer, or browser tabs)
    if lrc_raw.is_none() {
        let query = if clean_artist.is_empty() {
            clean_title.clone()
        } else {
            format!("{clean_artist} {clean_title}")
        };
        let search_url = format!(
            "https://lrclib.net/api/search?q={}",
            urlencoding::encode(&query)
        );
        if let Ok(resp) = client.get(&search_url).send().await {
            if let Ok(results) = resp.json::<Vec<LrcLibResponse>>().await {
                lrc_raw = results
                    .into_iter()
                    .find_map(|r| r.synced_lyrics.filter(|s| !s.trim().is_empty()));
            }
        }
    }

    let lrc_raw = lrc_raw?;

    if let Some(ref path) = cache_file {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, &lrc_raw);
    }

    parse_lrc(&lrc_raw, "LRCLIB")
}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(48)
        .collect()
}

/// Asynchronous lyrics worker (1. Sonora SQLite -> 2. LRCLIB)
pub fn spawn_lyrics_worker(
    mut meta_rx: watch::Receiver<TrackMetadata>,
) -> watch::Receiver<Option<SyncedLyrics>> {
    let (tx, rx) = watch::channel(None);

    tokio::spawn(async move {
        let mut memory_cache: HashMap<String, Option<SyncedLyrics>> = HashMap::new();
        let sonora_db = default_sonora_db_path();
        let http_client = reqwest::Client::builder()
            .user_agent("kymat/0.1.0 (https://github.com/Margul7803/kymat)")
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        loop {
            let meta = meta_rx.borrow_and_update().clone();
            if meta.is_empty() {
                let _ = tx.send(None);
            } else {
                let key = meta.cache_key();
                if let Some(Some(cached)) = memory_cache.get(&key) {
                    let _ = tx.send(Some(cached.clone()));
                } else {
                    let _ = tx.send(None);
                    let mut resolved = None;
                    let sonora_key = extract_sonora_cache_key(&meta.source_url);

                    // 1. Primary lookup in `~/.cache/sonora/cache.sqlite`
                    if let (Some(ref db), Some(ref s_key)) = (&sonora_db, &sonora_key) {
                        for attempt in 0..3 {
                            resolved = query_sonora_sqlite(db, s_key);
                            if resolved.is_some() || meta_rx.has_changed().unwrap_or(false) {
                                break;
                            }
                            if attempt < 2 {
                                tokio::time::sleep(Duration::from_millis(350)).await;
                            }
                        }
                    }

                    if meta_rx.has_changed().unwrap_or(false) {
                        continue;
                    }

                    // 2. Fallback to LRCLIB + final Sonora SQLite check
                    if resolved.is_none() {
                        resolved = fetch_lrclib(&http_client, &meta).await;
                    }
                    if resolved.is_none() && !meta_rx.has_changed().unwrap_or(false) {
                        if let (Some(ref db), Some(ref s_key)) = (&sonora_db, &sonora_key) {
                            resolved = query_sonora_sqlite(db, s_key);
                        }
                    }

                    if meta_rx.has_changed().unwrap_or(false) {
                        continue;
                    }

                    if memory_cache.len() > 32 {
                        memory_cache.clear();
                    }
                    if resolved.is_some() {
                        memory_cache.insert(key, resolved.clone());
                    }
                    let _ = tx.send(resolved);
                }
            }

            if meta_rx.changed().await.is_err() {
                break;
            }
        }
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_sonora_cache_key() {
        let url = "https://open.spotify.com/track/151pHp99ciTnpmdPCvgLbe";
        assert_eq!(
            extract_sonora_cache_key(url),
            Some("spotify:151pHp99ciTnpmdPCvgLbe:lyrics".to_string())
        );
    }

    #[test]
    fn test_parse_lrc_and_binary_search() {
        let sample = "[00:01.83] Hmm, hey, mmh\n[00:05.63] Don't think about it\n[00:10.00] Just move your body";
        let synced = parse_lrc(sample, "Test").expect("Should parse valid LRC");
        assert_eq!(synced.lines.len(), 3);
        assert_eq!(synced.active_line_index(Duration::from_secs(0)), Some(0));
        assert_eq!(synced.active_line_index(Duration::from_secs(3)), Some(0));
        assert_eq!(synced.active_line_index(Duration::from_secs(6)), Some(1));
        assert_eq!(synced.active_line_index(Duration::from_secs(15)), Some(2));
    }

    #[test]
    fn test_query_real_sonora_sqlite_if_present() {
        if let Some(db_path) = default_sonora_db_path() {
            if db_path.exists() {
                let res = query_sonora_sqlite(&db_path, "spotify:151pHp99ciTnpmdPCvgLbe:lyrics");
                if let Some(lyrics) = res {
                    assert!(!lyrics.lines.is_empty());
                    assert!(!lyrics.lines[0].words.is_empty());
                }
            }
        }
    }
}
