use crate::audio::fft::{AudioTelemetry, FftAnalyzer, HOP_SIZE, SAMPLE_RATE};
use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use std::thread;
use std::time::Duration;
use tokio::sync::watch;

/// Spawns a dedicated real-time audio capture thread listening on `@DEFAULT_MONITOR@`
/// via `libpulse-simple` (natively supported by `pipewire-pulse`).
pub fn spawn_audio_capture_thread(sensitivity: f32) -> watch::Receiver<AudioTelemetry> {
    let (tx, rx) = watch::channel(AudioTelemetry::default());

    thread::Builder::new()
        .name("kymat-audio-fft".to_string())
        .spawn(move || {
            let mut analyzer = FftAnalyzer::new();
            let mut byte_buf = vec![0u8; HOP_SIZE * std::mem::size_of::<f32>()];
            let mut sample_buf = vec![0.0_f32; HOP_SIZE];

            loop {
                let spec = Spec {
                    format: Format::F32le,
                    channels: 1,
                    rate: SAMPLE_RATE as u32,
                };

                let attr = BufferAttr {
                    maxlength: (HOP_SIZE * 8) as u32,
                    tlength: u32::MAX,
                    prebuf: u32::MAX,
                    minreq: u32::MAX,
                    fragsize: (HOP_SIZE * std::mem::size_of::<f32>()) as u32,
                };

                let connection = Simple::new(
                    None,
                    "kymat",
                    Direction::Record,
                    Some("@DEFAULT_MONITOR@"),
                    "audio-visualizer-monitor",
                    &spec,
                    None,
                    Some(&attr),
                )
                .or_else(|_| {
                    // Fallback to the default recording source if @DEFAULT_MONITOR@ is unavailable
                    Simple::new(
                        None,
                        "kymat",
                        Direction::Record,
                        None,
                        "audio-visualizer-default",
                        &spec,
                        None,
                        Some(&attr),
                    )
                });

                let Ok(pulse) = connection else {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                };

                while pulse.read(&mut byte_buf).is_ok() {
                    for (i, chunk) in byte_buf.as_chunks::<4>().0.iter().enumerate() {
                        sample_buf[i] = f32::from_le_bytes(*chunk);
                    }
                    let telemetry = analyzer.push_samples_telemetry(&sample_buf, sensitivity);
                    if tx.send(telemetry).is_err() {
                        return;
                    }
                }

                thread::sleep(Duration::from_millis(500));
            }
        })
        .ok();

    rx
}
