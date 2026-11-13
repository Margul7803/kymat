use crate::audio::fft::AudioTelemetry;
use crate::media::lyrics::SyncedLyrics;
use crate::ui::background::AmbientBackground;
use crate::ui::cover_widget::{best_quadrant_cell, Rgb};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::sync::OnceLock;
use std::time::Duration;

const SPRITE_W: usize = 80;
const SPRITE_H: usize = 120;
const FRAME_COUNT: usize = 143;
const BASE_FPS: f32 = 30.0;

static RAT_SPRITE_SHEET: OnceLock<Vec<[u8; 4]>> = OnceLock::new();

fn decoded_rat_frames() -> &'static [[u8; 4]] {
    RAT_SPRITE_SHEET.get_or_init(|| {
        let bytes = include_bytes!("../../assets/rat_spritesheet.png");
        match image::load_from_memory(bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let expected = SPRITE_W * SPRITE_H * FRAME_COUNT;
                let mut out = Vec::with_capacity(expected);
                for p in rgba.pixels().take(expected) {
                    out.push(p.0);
                }
                if out.len() < expected {
                    out.resize(expected, [0, 0, 0, 0]);
                }
                out
            }
            Err(_) => vec![[0, 0, 0, 0]; SPRITE_W * SPRITE_H * FRAME_COUNT],
        }
    })
}

/// 4-phase musical state machine for climax ("temps fort") detection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClimaxPhase {
    /// Initial track calibration (learns baseline loudness and bass floor without false triggers)
    Warmup,
    /// Monitoring verse/build-up dynamics, pre-drop bass troughs, spectral flux, and LRC chorus cues
    Monitoring,
    /// Active musical climax ("temps fort"): holds across the musical phrase with hysteresis
    ClimaxActive,
    /// Post-climax cooldown preventing rapid re-triggers right after a chorus ends
    Cooldown,
}

/// Per-frame context passed to the musical climax detector
pub struct ClimaxContext<'a> {
    pub telemetry: &'a AudioTelemetry,
    pub dt: f32,
    pub is_playing: bool,
    pub auto_enabled: bool,
    pub track_pos: Duration,
    pub track_duration: Duration,
    pub lyrics: Option<&'a SyncedLyrics>,
}

/// High-definition 144 FPS dancing rat easter egg state and multi-criteria musical climax detector
pub struct RatEasterEgg {
    /// Manual toggle triggered via the `r` key (acts like `Tab`, independent of climax detection)
    pub manual_override: bool,
    frame_cursor: f32,
    opacity: f32,

    // --- Solution 1: Short/Long intra-song dynamic envelopes (volume-invariant via ratios) ---
    short_rms: f32,
    long_rms: f32,
    short_bass: f32,
    long_bass: f32,
    /// Tracks recent pre-drop bass trough over ~2.2s to detect build-up -> drop impact
    pre_drop_bass_min: f32,
    envelope_initialized: bool,

    // --- Solution 2: Smoothed spectral onset flux & broadband fullness ---
    flux_smooth: f32,
    fullness_smooth: f32,

    // --- Solution 3: 4-phase musical state machine with hysteresis & cooldown ---
    phase: ClimaxPhase,
    phase_timer: f32,
    active_climax_secs: f32,
    onset_confirm_secs: f32,
    breakdown_confirm_secs: f32,
}

impl Default for RatEasterEgg {
    fn default() -> Self {
        Self::new()
    }
}

impl RatEasterEgg {
    pub fn new() -> Self {
        Self {
            manual_override: false,
            frame_cursor: 0.0,
            opacity: 0.0,
            short_rms: 0.0,
            long_rms: 0.0,
            short_bass: 0.0,
            long_bass: 0.0,
            pre_drop_bass_min: 0.0,
            envelope_initialized: false,
            flux_smooth: 0.0,
            fullness_smooth: 0.0,
            phase: ClimaxPhase::Warmup,
            phase_timer: 3.2,
            active_climax_secs: 0.0,
            onset_confirm_secs: 0.0,
            breakdown_confirm_secs: 0.0,
        }
    }

    /// Resets track-specific dynamic baselines and enters `Warmup` when a new track starts
    pub fn reset_for_new_track(&mut self) {
        self.short_rms = 0.0;
        self.long_rms = 0.0;
        self.short_bass = 0.0;
        self.long_bass = 0.0;
        self.pre_drop_bass_min = 0.0;
        self.envelope_initialized = false;
        self.flux_smooth = 0.0;
        self.fullness_smooth = 0.0;
        self.phase = ClimaxPhase::Warmup;
        self.phase_timer = 3.2;
        self.active_climax_secs = 0.0;
        self.onset_confirm_secs = 0.0;
        self.breakdown_confirm_secs = 0.0;
    }

    pub fn toggle_manual(&mut self) {
        self.manual_override = !self.manual_override;
    }

    pub fn is_visible(&self) -> bool {
        self.opacity > 0.01
    }

    pub fn phase(&self) -> ClimaxPhase {
        self.phase
    }

    /// Updates multi-criteria musical climax ("temps fort") detection and 144 FPS sub-frame cursor
    pub fn step(&mut self, ctx: &ClimaxContext<'_>) {
        let dt = ctx.dt.clamp(0.0005, 0.1);
        let tel = ctx.telemetry;

        let max_band = tel.bands.iter().copied().fold(0.0_f32, f32::max);
        let is_active_audio = ctx.is_playing && max_band >= 0.04 && tel.raw_rms >= 5.0e-6;

        let bass_norm: f32 = tel.bands[0..12].iter().sum::<f32>() / 12.0;

        if is_active_audio {
            if !self.envelope_initialized {
                self.short_rms = tel.raw_rms;
                self.long_rms = tel.raw_rms;
                self.short_bass = tel.raw_bass_energy.max(1e-6);
                self.long_bass = tel.raw_bass_energy.max(1e-6);
                self.pre_drop_bass_min = self.short_bass;
                self.flux_smooth = tel.spectral_flux;
                self.fullness_smooth = tel.spectral_fullness;
                self.envelope_initialized = true;
            } else {
                let rms_tau = if tel.raw_rms < self.short_rms {
                    0.35
                } else {
                    0.75
                };
                let bass_tau = if tel.raw_bass_energy < self.short_bass {
                    0.20
                } else {
                    0.38
                };
                let a_short_rms = 1.0 - (-dt / rms_tau).exp();
                let a_short_bass = 1.0 - (-dt / bass_tau).exp();

                // During Warmup, adapt long-term baseline faster (2.2s) to lock onto the song's level;
                // afterwards use a 15s musical baseline so verses vs choruses have clear contrast.
                let long_tau = if self.phase == ClimaxPhase::Warmup {
                    2.2
                } else {
                    15.0
                };
                let a_long = 1.0 - (-dt / long_tau).exp();

                self.short_rms += (tel.raw_rms - self.short_rms) * a_short_rms;
                self.short_bass += (tel.raw_bass_energy - self.short_bass) * a_short_bass;

                // Do not let an active climax inflate the baseline too quickly
                let baseline_scale = if self.phase == ClimaxPhase::ClimaxActive {
                    0.35
                } else {
                    1.0
                };
                self.long_rms += (tel.raw_rms - self.long_rms) * (a_long * baseline_scale);
                self.long_bass +=
                    (tel.raw_bass_energy - self.long_bass) * (a_long * baseline_scale);

                // Track pre-drop bass trough: falls quickly when bass cuts out during a build-up/break,
                // rises slowly (2.4s) so the subsequent drop impact produces a high `short_bass / pre_drop_bass_min`.
                if self.short_bass < self.pre_drop_bass_min {
                    let a_fall = 1.0 - (-dt / 0.25).exp();
                    self.pre_drop_bass_min += (self.short_bass - self.pre_drop_bass_min) * a_fall;
                } else {
                    let a_rise = 1.0 - (-dt / 2.4).exp();
                    self.pre_drop_bass_min += (self.short_bass - self.pre_drop_bass_min) * a_rise;
                }

                let a_flux = 1.0 - (-dt / 0.35).exp();
                let a_full = 1.0 - (-dt / 0.45).exp();
                self.flux_smooth += (tel.spectral_flux - self.flux_smooth) * a_flux;
                self.fullness_smooth += (tel.spectral_fullness - self.fullness_smooth) * a_full;
            }

            // --- 1. Volume-invariant dynamic ratios (system volume scalar cancels in numerator/denominator) ---
            let rms_ratio = (self.short_rms / self.long_rms.max(1e-7)).clamp(0.0, 3.5);
            let bass_ratio = (self.short_bass / self.long_bass.max(1e-7)).clamp(0.0, 4.0);
            let drop_contrast =
                (self.short_bass / self.pre_drop_bass_min.max(1e-7)).clamp(1.0, 4.0);

            // --- 2. Multi-criteria climax score ---
            let loudness_surge = ((rms_ratio - 1.06) / 0.32).clamp(0.0, 1.35);
            let bass_surge = ((bass_ratio - 1.10) / 0.38).clamp(0.0, 1.45);
            let drop_punch = ((drop_contrast - 1.30) / 0.60).clamp(0.0, 1.10)
                * (self.flux_smooth * 1.9).clamp(0.25, 1.0);
            let rhythm_drive = (self.flux_smooth * 1.45).clamp(0.0, 1.0)
                * (self.fullness_smooth * 1.45).clamp(0.0, 1.0);

            // --- Bonus 4: LRC Chorus & MPRIS structural position awareness ---
            let lrc_affinity = ctx
                .lyrics
                .map(|l| l.chorus_affinity_at(ctx.track_pos))
                .unwrap_or(0.30);

            let climax_score = loudness_surge * 0.32
                + bass_surge * 0.30
                + drop_punch * 0.20
                + rhythm_drive * 0.18
                + lrc_affinity * 0.24;

            // MPRIS intro/outro guards when track duration is known (> 30s)
            let known_long_track = ctx.track_duration >= Duration::from_secs(30);
            let in_early_intro = known_long_track && ctx.track_pos < Duration::from_millis(6500);
            let in_late_outro = known_long_track
                && ctx.track_duration.saturating_sub(ctx.track_pos) < Duration::from_secs(6);

            // If LRC has confirmed repeated choruses and we are currently in a non-chorus verse,
            // require a stronger purely-instrumental drop surge to trigger.
            let trigger_threshold =
                if ctx.lyrics.is_some_and(|l| l.has_detected_chorus) && lrc_affinity < 0.1 {
                    0.96
                } else {
                    0.76
                };

            // --- 3. 4-Phase Musical State Machine ---
            match self.phase {
                ClimaxPhase::Warmup => {
                    self.phase_timer = (self.phase_timer - dt).max(0.0);
                    if self.phase_timer <= 0.0 && !in_early_intro {
                        self.phase = ClimaxPhase::Monitoring;
                    }
                }
                ClimaxPhase::Monitoring => {
                    if !ctx.auto_enabled || in_early_intro || in_late_outro {
                        self.onset_confirm_secs = 0.0;
                    } else if climax_score >= trigger_threshold
                        && bass_ratio >= 1.12
                        && self.fullness_smooth >= 0.35
                    {
                        // Fast confirmation on heavy pre-drop impact, ~0.28s confirmation otherwise
                        let speed = if drop_punch > 0.45 { 2.2 } else { 1.0 };
                        self.onset_confirm_secs += dt * speed;
                        if self.onset_confirm_secs >= 0.26 {
                            self.phase = ClimaxPhase::ClimaxActive;
                            self.phase_timer = 8.5;
                            self.active_climax_secs = 0.0;
                            self.onset_confirm_secs = 0.0;
                            self.breakdown_confirm_secs = 0.0;
                        }
                    } else {
                        self.onset_confirm_secs = (self.onset_confirm_secs - dt * 1.5).max(0.0);
                    }
                }
                ClimaxPhase::ClimaxActive => {
                    if !ctx.auto_enabled || in_late_outro {
                        self.phase = ClimaxPhase::Cooldown;
                        self.phase_timer = 8.0;
                    } else {
                        self.active_climax_secs += dt;
                        self.phase_timer = (self.phase_timer - dt).max(0.0);

                        // Hysteresis: detect sudden breakdown/calm bridge to exit immediately on the break
                        let in_sudden_break =
                            rms_ratio < 0.74 || bass_ratio < 0.64 || self.fullness_smooth < 0.24;
                        if in_sudden_break {
                            self.breakdown_confirm_secs += dt;
                        } else {
                            self.breakdown_confirm_secs =
                                (self.breakdown_confirm_secs - dt * 2.0).max(0.0);
                        }

                        if self.breakdown_confirm_secs >= 0.35 {
                            self.phase = ClimaxPhase::Cooldown;
                            self.phase_timer = 9.0;
                        } else if self.phase_timer <= 0.5 {
                            // Extend phrase up to 15s max if the chorus/drop is still going strong
                            if self.active_climax_secs < 15.0
                                && climax_score >= 0.62
                                && rms_ratio >= 1.02
                            {
                                self.phase_timer = 2.5;
                            } else if self.phase_timer <= 0.0 {
                                self.phase = ClimaxPhase::Cooldown;
                                self.phase_timer = 10.0;
                            }
                        }
                    }
                }
                ClimaxPhase::Cooldown => {
                    self.phase_timer = (self.phase_timer - dt).max(0.0);
                    if self.phase_timer <= 0.0 {
                        self.phase = ClimaxPhase::Monitoring;
                        self.onset_confirm_secs = 0.0;
                    }
                }
            }
        } else {
            // Paused or silent: exit active climax cleanly
            self.onset_confirm_secs = 0.0;
            if self.phase == ClimaxPhase::ClimaxActive {
                self.phase = ClimaxPhase::Monitoring;
                self.phase_timer = 0.0;
            }
        }

        let auto_visible =
            ctx.auto_enabled && is_active_audio && self.phase == ClimaxPhase::ClimaxActive;
        let should_show = self.manual_override || auto_visible;

        let target_opacity = if should_show { 1.0 } else { 0.0 };
        let fade_speed = if should_show { 7.5 } else { 4.5 };
        let fade_factor = 1.0 - (-fade_speed * dt).exp();
        self.opacity =
            (self.opacity + (target_opacity - self.opacity) * fade_factor).clamp(0.0, 1.0);

        if self.opacity > 0.005 {
            let beat_boost = if is_active_audio {
                0.92 + 0.34 * bass_norm.clamp(0.0, 1.0)
            } else {
                1.0
            };
            self.frame_cursor =
                (self.frame_cursor + dt * BASE_FPS * beat_boost) % (FRAME_COUNT as f32);
        }
    }

    /// Renders the dancing rat in the center of `area` using 2x2 sub-pixel Unicode Quadrants
    /// and 144 FPS temporal frame interpolation, alpha-blended over the existing terminal buffer.
    pub fn render(&self, area: Rect, buf: &mut Buffer, bg: &AmbientBackground, cell_aspect: f32) {
        if self.opacity <= 0.01 || area.width < 16 || area.height < 12 {
            return;
        }

        let frames = decoded_rat_frames();
        let frame_pixels = SPRITE_W * SPRITE_H;
        if frames.len() < frame_pixels * FRAME_COUNT {
            return;
        }

        let frame_a = (self.frame_cursor.floor() as usize) % FRAME_COUNT;
        let frame_b = (frame_a + 1) % FRAME_COUNT;
        let frame_frac = self.frame_cursor.fract();

        let slice_a = &frames[frame_a * frame_pixels..(frame_a + 1) * frame_pixels];
        let slice_b = &frames[frame_b * frame_pixels..(frame_b + 1) * frame_pixels];

        let rat_h = ((area.height as u32 * 64) / 100)
            .clamp(14, 44)
            .min(area.height as u32) as u16;
        let aspect_sprite = (SPRITE_W as f32) / (SPRITE_H as f32);
        let rat_w = ((rat_h as f32) * aspect_sprite * cell_aspect.clamp(1.8, 2.6))
            .round()
            .clamp(12.0, area.width as f32) as u16;

        let start_x = area.x + area.width.saturating_sub(rat_w) / 2;
        let start_y = area.y + area.height.saturating_sub(rat_h) / 2;

        let sub_w = (rat_w as usize * 2).max(2) as f32;
        let sub_h = (rat_h as usize * 2).max(2) as f32;

        for cy in 0..rat_h {
            let screen_y = start_y + cy;
            if screen_y >= area.bottom() {
                break;
            }

            for cx in 0..rat_w {
                let screen_x = start_x + cx;
                if screen_x >= area.right() {
                    break;
                }

                let sub_coords = [
                    (cx * 2, cy * 2),
                    (cx * 2 + 1, cy * 2),
                    (cx * 2, cy * 2 + 1),
                    (cx * 2 + 1, cy * 2 + 1),
                ];

                let mut samples = [([0u8; 3], 0.0_f32); 4];
                let mut max_alpha = 0.0_f32;

                for (i, &(sx, sy)) in sub_coords.iter().enumerate() {
                    let u = ((sx as f32 + 0.5) / sub_w) * (SPRITE_W as f32 - 1.0);
                    let v = ((sy as f32 + 0.5) / sub_h) * (SPRITE_H as f32 - 1.0);
                    let (rgb, a) = sample_bilinear_temporal(slice_a, slice_b, frame_frac, u, v);
                    let eff_a = a * self.opacity;
                    max_alpha = max_alpha.max(eff_a);
                    samples[i] = (rgb, eff_a);
                }

                // Skip cells where all 4 sub-pixels are transparent so the background/UI stays untouched
                if max_alpha < 0.03 {
                    continue;
                }

                let backdrop_rgb = extract_cell_backdrop_rgb(buf, bg, screen_x, screen_y);
                let mut cell_sub_pixels: [Rgb; 4] = [backdrop_rgb; 4];

                for i in 0..4 {
                    let (rgb, a) = samples[i];
                    let inv_a = 1.0 - a;
                    cell_sub_pixels[i] = (
                        ((rgb[0] as f32) * a + (backdrop_rgb.0 as f32) * inv_a).round() as u8,
                        ((rgb[1] as f32) * a + (backdrop_rgb.1 as f32) * inv_a).round() as u8,
                        ((rgb[2] as f32) * a + (backdrop_rgb.2 as f32) * inv_a).round() as u8,
                    );
                }

                let (ch, fg, c_bg) = best_quadrant_cell(cell_sub_pixels);
                if let Some(cell) = buf.cell_mut((screen_x, screen_y)) {
                    cell.set_char(ch);
                    cell.set_fg(Color::Rgb(fg.0, fg.1, fg.2));
                    cell.set_bg(Color::Rgb(c_bg.0, c_bg.1, c_bg.2));
                }
            }
        }
    }
}

#[inline]
fn extract_cell_backdrop_rgb(buf: &Buffer, bg: &AmbientBackground, x: u16, y: u16) -> Rgb {
    if let Some(cell) = buf.cell((x, y)) {
        if let Color::Rgb(r, g, b) = cell.bg {
            return (r, g, b);
        }
    }
    bg.rgb_at(x, y)
}

#[inline]
fn sample_bilinear_temporal(
    slice_a: &[[u8; 4]],
    slice_b: &[[u8; 4]],
    frame_frac: f32,
    u: f32,
    v: f32,
) -> ([u8; 3], f32) {
    let x0 = (u.floor() as usize).min(SPRITE_W - 1);
    let y0 = (v.floor() as usize).min(SPRITE_H - 1);
    let x1 = (x0 + 1).min(SPRITE_W - 1);
    let y1 = (y0 + 1).min(SPRITE_H - 1);

    let tx = u - (x0 as f32);
    let ty = v - (y0 as f32);

    let w00 = (1.0 - tx) * (1.0 - ty);
    let w10 = tx * (1.0 - ty);
    let w01 = (1.0 - tx) * ty;
    let w11 = tx * ty;

    let idx00 = y0 * SPRITE_W + x0;
    let idx10 = y0 * SPRITE_W + x1;
    let idx01 = y1 * SPRITE_W + x0;
    let idx11 = y1 * SPRITE_W + x1;

    let sample_slice = |slice: &[[u8; 4]]| -> [f32; 4] {
        let p00 = slice[idx00];
        let p10 = slice[idx10];
        let p01 = slice[idx01];
        let p11 = slice[idx11];
        [
            (p00[0] as f32) * w00
                + (p10[0] as f32) * w10
                + (p01[0] as f32) * w01
                + (p11[0] as f32) * w11,
            (p00[1] as f32) * w00
                + (p10[1] as f32) * w10
                + (p01[1] as f32) * w01
                + (p11[1] as f32) * w11,
            (p00[2] as f32) * w00
                + (p10[2] as f32) * w10
                + (p01[2] as f32) * w01
                + (p11[2] as f32) * w11,
            (p00[3] as f32) * w00
                + (p10[3] as f32) * w10
                + (p01[3] as f32) * w01
                + (p11[3] as f32) * w11,
        ]
    };

    let ca = sample_slice(slice_a);
    let cb = sample_slice(slice_b);
    let inv_f = 1.0 - frame_frac;

    let r = (ca[0] * inv_f + cb[0] * frame_frac).clamp(0.0, 255.0) as u8;
    let g = (ca[1] * inv_f + cb[1] * frame_frac).clamp(0.0, 255.0) as u8;
    let b = (ca[2] * inv_f + cb[2] * frame_frac).clamp(0.0, 255.0) as u8;
    let a = ((ca[3] * inv_f + cb[3] * frame_frac) / 255.0).clamp(0.0, 1.0);

    ([r, g, b], a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fft::NUM_BANDS;
    use crate::media::lyrics::parse_lrc;

    #[test]
    fn test_rat_spritesheet_decoded_and_multi_criteria_state_machine() {
        let frames = decoded_rat_frames();
        assert_eq!(frames.len(), SPRITE_W * SPRITE_H * FRAME_COUNT);

        let lrc = parse_lrc(
            "[00:02.00] Quiet intro line\n\
             [00:10.00] Soft verse words here\n\
             [00:25.00] Feel the rhythm in the night\n\
             [00:29.00] Dancing under neon light\n\
             [00:45.00] Second verse calm again\n\
             [01:00.00] Feel the rhythm in the night\n\
             [01:04.00] Dancing under neon light",
            "Test",
        )
        .unwrap();
        assert!(lrc.has_detected_chorus);
        assert_eq!(lrc.chorus_affinity_at(Duration::from_secs(12)), 0.0);
        assert_eq!(lrc.chorus_affinity_at(Duration::from_secs(26)), 1.0);

        let mut rat = RatEasterEgg::new();

        // 1. Warmup + Verse (calm RMS = 0.05, bass = 0.03): should calibrate to Monitoring and NOT trigger
        let calm_tel = AudioTelemetry {
            bands: [0.55; NUM_BANDS],
            raw_rms: 0.05,
            raw_bass_energy: 0.03,
            spectral_flux: 0.12,
            spectral_fullness: 0.42,
        };
        for _ in 0..80 {
            rat.step(&ClimaxContext {
                telemetry: &calm_tel,
                dt: 0.1,
                is_playing: true,
                auto_enabled: true,
                track_pos: Duration::from_secs(12),
                track_duration: Duration::from_secs(180),
                lyrics: Some(&lrc),
            });
        }
        assert_eq!(rat.phase(), ClimaxPhase::Monitoring);
        assert!(
            !rat.is_visible(),
            "Rat must not trigger during steady calm verse"
        );

        // 2. Chorus Drop at 00:26 (RMS +6 dB = 0.10, Bass surge = 0.08, high flux & fullness + LRC chorus)
        let drop_tel = AudioTelemetry {
            bands: [0.85; NUM_BANDS],
            raw_rms: 0.10,
            raw_bass_energy: 0.08,
            spectral_flux: 0.65,
            spectral_fullness: 0.78,
        };
        for _ in 0..10 {
            rat.step(&ClimaxContext {
                telemetry: &drop_tel,
                dt: 0.1,
                is_playing: true,
                auto_enabled: true,
                track_pos: Duration::from_secs(26),
                track_duration: Duration::from_secs(180),
                lyrics: Some(&lrc),
            });
        }
        assert_eq!(rat.phase(), ClimaxPhase::ClimaxActive);
        assert!(rat.is_visible(), "Rat should appear on the chorus drop");

        // 3. Sudden breakdown cuts bass & RMS -> Hysteresis exits ClimaxActive into Cooldown
        let break_tel = AudioTelemetry {
            bands: [0.25; NUM_BANDS],
            raw_rms: 0.02,
            raw_bass_energy: 0.005,
            spectral_flux: 0.05,
            spectral_fullness: 0.18,
        };
        for _ in 0..10 {
            rat.step(&ClimaxContext {
                telemetry: &break_tel,
                dt: 0.1,
                is_playing: true,
                auto_enabled: true,
                track_pos: Duration::from_secs(36),
                track_duration: Duration::from_secs(180),
                lyrics: Some(&lrc),
            });
        }
        assert_eq!(rat.phase(), ClimaxPhase::Cooldown);
    }
}
