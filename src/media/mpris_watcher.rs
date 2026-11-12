use futures_util::StreamExt;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackMetadata {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: String,
    pub source_url: String,
    pub duration: Duration,
    pub player_bus_name: String,
}

impl TrackMetadata {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.artist.is_empty()
    }

    /// Unique key used to detect track changes
    pub fn cache_key(&self) -> String {
        if !self.source_url.is_empty() {
            self.source_url.clone()
        } else if !self.track_id.is_empty() {
            format!("{}|{}|{}", self.track_id, self.title, self.artist)
        } else {
            format!("{}|{}|{}", self.artist, self.title, self.album)
        }
    }
}

/// High-precision playback clock interpolating playback position at 144 Hz
/// without polling D-Bus on every frame.
#[derive(Debug, Clone)]
pub struct PlaybackClock {
    pub base_position: Duration,
    /// Last raw `Position` value read from D-Bus (prevents backward jumps on 1 Hz D-Bus players)
    pub last_raw_dbus_position: Duration,
    pub sync_instant: Instant,
    pub rate: f64,
    pub status: PlaybackStatus,
    pub duration: Duration,
}

impl Default for PlaybackClock {
    fn default() -> Self {
        Self {
            base_position: Duration::ZERO,
            last_raw_dbus_position: Duration::ZERO,
            sync_instant: Instant::now(),
            rate: 1.0,
            status: PlaybackStatus::Stopped,
            duration: Duration::ZERO,
        }
    }
}

impl PlaybackClock {
    /// Computes the interpolated playback position at `now`:
    /// `t_current = t_mpris_last + (t_now - t_sync) * PlaybackRate`
    pub fn position_at(&self, now: Instant) -> Duration {
        let raw = match self.status {
            PlaybackStatus::Playing => {
                let elapsed = now.saturating_duration_since(self.sync_instant);
                let scaled = Duration::from_secs_f64(elapsed.as_secs_f64() * self.rate.max(0.0));
                self.base_position.saturating_add(scaled)
            }
            PlaybackStatus::Paused => self.base_position,
            PlaybackStatus::Stopped => Duration::ZERO,
        };
        if !self.duration.is_zero() {
            raw.min(self.duration)
        } else {
            raw
        }
    }

    /// Reconciles the local clock against a new D-Bus sample monotonically:
    /// - If the track is playing and D-Bus returns the exact same `Position` as the previous poll
    ///   (players like Sonora only update their D-Bus Position property at ~1 Hz), keep the local
    ///   interpolation running smoothly.
    /// - If the new D-Bus sample lags slightly (< 1.8s) due to MPRIS publishing latency, preserve
    ///   the predicted position to prevent backward yo-yo jumps in lyrics scrolling.
    pub fn reconcile_dbus_update(
        prev: Option<&PlaybackClock>,
        same_track: bool,
        new_raw_pos: Duration,
        new_status: PlaybackStatus,
        new_rate: f64,
        new_duration: Duration,
        now: Instant,
    ) -> Self {
        if let Some(prev_clock) = prev {
            if same_track
                && prev_clock.status == PlaybackStatus::Playing
                && new_status == PlaybackStatus::Playing
            {
                let predicted = prev_clock.position_at(now);

                if new_raw_pos == prev_clock.last_raw_dbus_position
                    && predicted >= new_raw_pos
                    && predicted.saturating_sub(new_raw_pos) < Duration::from_millis(1800)
                {
                    return Self {
                        base_position: prev_clock.base_position,
                        last_raw_dbus_position: prev_clock.last_raw_dbus_position,
                        sync_instant: prev_clock.sync_instant,
                        rate: new_rate,
                        status: new_status,
                        duration: new_duration,
                    };
                }

                if predicted > new_raw_pos
                    && predicted.saturating_sub(new_raw_pos) < Duration::from_millis(1800)
                {
                    return Self {
                        base_position: predicted,
                        last_raw_dbus_position: new_raw_pos,
                        sync_instant: now,
                        rate: new_rate,
                        status: new_status,
                        duration: new_duration,
                    };
                }
            }
        }

        Self {
            base_position: new_raw_pos,
            last_raw_dbus_position: new_raw_pos,
            sync_instant: now,
            rate: new_rate,
            status: new_status,
            duration: new_duration,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MprisState {
    pub metadata: TrackMetadata,
    pub clock: PlaybackClock,
    pub last_active: Option<Instant>,
}

/// Spawns the asynchronous D-Bus MPRIS listener task
pub fn spawn_mpris_watcher() -> watch::Receiver<MprisState> {
    let (tx, rx) = watch::channel(MprisState::default());

    tokio::spawn(async move {
        let Ok(conn) = Connection::session().await else {
            return;
        };

        let props_rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .path("/org/mpris/MediaPlayer2")
            .and_then(|b| b.interface("org.freedesktop.DBus.Properties"))
            .and_then(|b| b.member("PropertiesChanged"))
            .ok()
            .map(|b| b.build());

        let seek_rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .path("/org/mpris/MediaPlayer2")
            .and_then(|b| b.interface("org.mpris.MediaPlayer2.Player"))
            .and_then(|b| b.member("Seeked"))
            .ok()
            .map(|b| b.build());

        let mut msg_stream = if let (Some(r1), Some(r2)) = (props_rule, seek_rule) {
            if let Ok(p) = zbus::fdo::DBusProxy::new(&conn).await {
                let _ = p.add_match_rule(r1).await;
                let _ = p.add_match_rule(r2).await;
            }
            Some(MessageStream::from(&conn))
        } else {
            None
        };

        // Initial immediate query
        if let Some(state) = query_best_player(&conn, None).await {
            let _ = tx.send(state);
        }

        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let prev_state = tx.borrow().clone();
                    if let Some(state) = query_best_player(&conn, Some(&prev_state)).await {
                        let _ = tx.send(state);
                    }
                }
                maybe_msg = async {
                    match msg_stream.as_mut() {
                        Some(s) => s.next().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(Ok(msg)) = maybe_msg {
                        let is_mpris_signal = msg.primary_header().msg_type() == zbus::message::Type::Signal
                            && msg.header().path().map(|p| p.as_str()) == Some("/org/mpris/MediaPlayer2");
                        if is_mpris_signal {
                            let prev_state = tx.borrow().clone();
                            if let Some(state) = query_best_player(&conn, Some(&prev_state)).await {
                                let _ = tx.send(state);
                            }
                        }
                    }
                }
            }
        }
    });

    rx
}

async fn query_best_player(
    conn: &Connection,
    prev_state: Option<&MprisState>,
) -> Option<MprisState> {
    let dbus = zbus::fdo::DBusProxy::builder(conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;
    let names = dbus.list_names().await.ok()?;

    let mut mpris_names: Vec<String> = names
        .into_iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
        .collect();

    if mpris_names.is_empty() {
        return Some(MprisState::default());
    }

    // Prioritize Sonora at the front of the candidate list
    mpris_names.sort_by_key(|name| if name.contains("sonora") { 0 } else { 1 });

    let mut best_candidate: Option<MprisState> = None;

    for bus_name in mpris_names {
        let Some(state) = read_player_state(conn, &bus_name, prev_state).await else {
            continue;
        };

        if state.clock.status == PlaybackStatus::Playing {
            return Some(state);
        }

        if best_candidate.is_none() && !state.metadata.is_empty() {
            best_candidate = Some(state);
        }
    }

    best_candidate.or_else(|| Some(MprisState::default()))
}

async fn read_player_state(
    conn: &Connection,
    bus_name: &str,
    prev_state: Option<&MprisState>,
) -> Option<MprisState> {
    let props = zbus::fdo::PropertiesProxy::builder(conn)
        .destination(bus_name)
        .ok()?
        .path("/org/mpris/MediaPlayer2")
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;

    let iface = zbus::names::InterfaceName::try_from("org.mpris.MediaPlayer2.Player").ok()?;
    let all = props
        .get_all(zbus::zvariant::Optional::from(Some(iface)))
        .await
        .ok()?;
    let now = Instant::now();

    let status = all
        .get("PlaybackStatus")
        .and_then(|v| <&str>::try_from(v).ok())
        .map(|s| match s {
            "Playing" => PlaybackStatus::Playing,
            "Paused" => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        })
        .unwrap_or(PlaybackStatus::Stopped);

    let rate = all
        .get("Rate")
        .and_then(|v| f64::try_from(v).ok())
        .unwrap_or(1.0);

    let pos_micros = all
        .get("Position")
        .and_then(extract_i64)
        .unwrap_or(0)
        .max(0) as u64;

    let metadata = all
        .get("Metadata")
        .map(|v| parse_metadata_map(v, bus_name))
        .unwrap_or_default();

    let prev_last_active = prev_state.and_then(|s| s.last_active);
    let last_active = if status == PlaybackStatus::Playing {
        Some(now)
    } else {
        prev_last_active
    };

    let same_track = prev_state
        .map(|prev| !prev.metadata.is_empty() && prev.metadata.cache_key() == metadata.cache_key())
        .unwrap_or(false);

    let clock = PlaybackClock::reconcile_dbus_update(
        prev_state.map(|s| &s.clock),
        same_track,
        Duration::from_micros(pos_micros),
        status,
        rate,
        metadata.duration,
        now,
    );

    Some(MprisState {
        clock,
        metadata,
        last_active,
    })
}

fn parse_metadata_map(val: &OwnedValue, bus_name: &str) -> TrackMetadata {
    let mut meta = TrackMetadata {
        player_bus_name: bus_name.to_string(),
        ..Default::default()
    };

    let Value::Dict(dict) = &**val else {
        return meta;
    };

    for (k, v) in dict.iter() {
        let Value::Str(key_str) = k else {
            continue;
        };
        let unwrapped: &Value = match v {
            Value::Value(inner) => inner,
            other => other,
        };

        match key_str.as_str() {
            "xesam:title" => {
                if let Value::Str(s) = unwrapped {
                    meta.title = s.to_string();
                }
            }
            "xesam:album" => {
                if let Value::Str(s) = unwrapped {
                    meta.album = s.to_string();
                }
            }
            "mpris:artUrl" => {
                if let Value::Str(s) = unwrapped {
                    meta.art_url = s.to_string();
                }
            }
            "xesam:url" => {
                if let Value::Str(s) = unwrapped {
                    meta.source_url = s.to_string();
                }
            }
            "mpris:trackid" => match unwrapped {
                Value::ObjectPath(p) => meta.track_id = p.to_string(),
                Value::Str(s) => meta.track_id = s.to_string(),
                _ => {}
            },
            "xesam:artist" => match unwrapped {
                Value::Array(arr) => {
                    let artists: Vec<String> = arr
                        .iter()
                        .filter_map(|item| match item {
                            Value::Str(s) => Some(s.to_string()),
                            Value::Value(inner) => match &**inner {
                                Value::Str(s) => Some(s.to_string()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .collect();
                    meta.artist = artists.join(", ");
                }
                Value::Str(s) => meta.artist = s.to_string(),
                _ => {}
            },
            "mpris:length" => {
                let micros = match unwrapped {
                    Value::I64(n) => (*n).max(0) as u64,
                    Value::U64(n) => *n,
                    Value::I32(n) => (*n).max(0) as u64,
                    Value::U32(n) => *n as u64,
                    _ => 0,
                };
                meta.duration = Duration::from_micros(micros);
            }
            _ => {}
        }
    }

    meta
}

fn extract_i64(val: &OwnedValue) -> Option<i64> {
    match &**val {
        Value::I64(n) => Some(*n),
        Value::U64(n) => Some(*n as i64),
        Value::I32(n) => Some(*n as i64),
        Value::Value(inner) => match &**inner {
            Value::I64(n) => Some(*n),
            Value::U64(n) => Some(*n as i64),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playback_clock_high_precision_interpolation() {
        let t0 = Instant::now();
        let clock = PlaybackClock {
            base_position: Duration::from_secs(15),
            last_raw_dbus_position: Duration::from_secs(15),
            sync_instant: t0,
            rate: 1.0,
            status: PlaybackStatus::Playing,
            duration: Duration::from_secs(180),
        };

        let pos = clock.position_at(t0 + Duration::from_millis(500));
        assert_eq!(pos, Duration::from_millis(15_500));

        let clamped = clock.position_at(t0 + Duration::from_secs(300));
        assert_eq!(clamped, Duration::from_secs(180));
    }

    #[test]
    fn test_playback_clock_paused_remains_fixed() {
        let t0 = Instant::now();
        let clock = PlaybackClock {
            base_position: Duration::from_millis(45_862),
            last_raw_dbus_position: Duration::from_millis(45_862),
            sync_instant: t0,
            rate: 1.0,
            status: PlaybackStatus::Paused,
            duration: Duration::from_secs(176),
        };

        let pos = clock.position_at(t0 + Duration::from_secs(10));
        assert_eq!(pos, Duration::from_millis(45_862));
    }

    #[test]
    fn test_monotonic_reconcile_prevents_yoyo_on_1hz_sonora_position() {
        let t0 = Instant::now();
        let dur = Duration::from_secs(240);

        let c1 = PlaybackClock::reconcile_dbus_update(
            None,
            true,
            Duration::from_millis(174_188),
            PlaybackStatus::Playing,
            1.0,
            dur,
            t0,
        );

        let t1 = t0 + Duration::from_millis(500);
        let pos_before_poll = c1.position_at(t1);
        assert_eq!(pos_before_poll, Duration::from_millis(174_688));

        let c2 = PlaybackClock::reconcile_dbus_update(
            Some(&c1),
            true,
            Duration::from_millis(174_188),
            PlaybackStatus::Playing,
            1.0,
            dur,
            t1,
        );
        let pos_after_poll = c2.position_at(t1);
        assert_eq!(pos_after_poll, Duration::from_millis(174_688));
    }

    #[tokio::test]
    async fn test_live_dbus_query_sonora() {
        let rx = spawn_mpris_watcher();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let s1 = rx.borrow().clone();
        println!("AFTER 300ms: title={}", s1.metadata.title);
    }
}
