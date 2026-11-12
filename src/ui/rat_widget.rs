use crate::audio::fft::NUM_BANDS;
use crate::ui::background::AmbientBackground;
use crate::ui::cover_widget::{Rgb, best_quadrant_cell};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::sync::OnceLock;

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

/// High-definition 144 FPS dancing rat easter egg state and musical climax ("temps fort") detector
pub struct RatEasterEgg {
    /// Manual toggle triggered via the `r` key (acts like `Tab`, independent of climax detection)
    pub manual_override: bool,
    frame_cursor: f32,
    fast_energy: f32,
    slow_energy: f32,
    climax_hold_secs: f32,
    opacity: f32,
}

impl Default for RatEasterEgg {
    fn default() -> Self {
        Self::new()
    }
}

impl RatEasterEgg {
    pub fn new() -> Self {
        // Warm up static spritesheet decoding once
        let _ = decoded_rat_frames();
        Self {
            manual_override: false,
            frame_cursor: 0.0,
            fast_energy: 0.0,
            slow_energy: 0.25,
            climax_hold_secs: 0.0,
            opacity: 0.0,
        }
    }

    pub fn toggle_manual(&mut self) {
        self.manual_override = !self.manual_override;
    }

    pub fn is_visible(&self) -> bool {
        self.opacity > 0.01
    }

    /// Updates musical climax ("temps fort") detection and 144 FPS sub-frame animation cursor
    pub fn step(
        &mut self,
        raw_bands: &[f32; NUM_BANDS],
        dt: f32,
        is_playing: bool,
        auto_enabled: bool,
    ) {
        let dt = dt.clamp(0.0005, 0.1);

        let max_band = raw_bands.iter().copied().fold(0.0_f32, f32::max);
        let is_active_audio = is_playing && max_band >= 0.04;

        let bass_energy: f32 = raw_bands[0..12].iter().sum::<f32>() / 12.0;
        let mid_energy: f32 = raw_bands[12..34].iter().sum::<f32>() / 22.0;
        let high_energy: f32 = raw_bands[34..NUM_BANDS].iter().sum::<f32>() / 14.0;
        let composite = bass_energy * 0.45 + mid_energy * 0.35 + high_energy * 0.20;

        if is_active_audio {
            let fast_alpha = 1.0 - (-dt / 0.32).exp();
            let slow_alpha = 1.0 - (-dt / 6.5).exp();
            self.fast_energy += (composite - self.fast_energy) * fast_alpha;
            self.slow_energy += (composite - self.slow_energy) * slow_alpha;

            // Detect musical climax ("temps fort"): strong bass + full spectrum surge above baseline
            let is_climax_surge = (self.fast_energy > 0.46
                && bass_energy > 0.44
                && self.fast_energy >= self.slow_energy * 1.08)
                || (self.fast_energy > 0.56 && bass_energy > 0.50);

            if auto_enabled && is_climax_surge {
                self.climax_hold_secs = 3.2;
            } else {
                self.climax_hold_secs = (self.climax_hold_secs - dt).max(0.0);
            }
        } else {
            let decay = 1.0 - (-dt / 0.25).exp();
            self.fast_energy *= 1.0 - decay;
            self.climax_hold_secs = 0.0;
        }

        let should_show = self.manual_override || (auto_enabled && self.climax_hold_secs > 0.0);
        let target_opacity = if should_show { 1.0 } else { 0.0 };
        let fade_speed = if should_show { 6.5 } else { 4.2 };
        let fade_factor = 1.0 - (-fade_speed * dt).exp();
        self.opacity = (self.opacity + (target_opacity - self.opacity) * fade_factor).clamp(0.0, 1.0);

        if self.opacity > 0.005 {
            // Modulate dance speed slightly with bass energy so the rat grooves to the beat
            let beat_boost = if is_active_audio {
                0.92 + 0.32 * bass_energy.clamp(0.0, 1.0)
            } else {
                1.0
            };
            self.frame_cursor =
                (self.frame_cursor + dt * BASE_FPS * beat_boost) % (FRAME_COUNT as f32);
        }
    }

    /// Renders the dancing rat in the center of `area` using 2x2 sub-pixel Unicode Quadrants
    /// and 144 FPS temporal frame interpolation, alpha-blended over the existing terminal buffer.
    pub fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        bg: &AmbientBackground,
        cell_aspect: f32,
    ) {
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

        let rat_h = ((area.height as u32 * 64) / 100).clamp(14, 44).min(area.height as u32) as u16;
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
fn extract_cell_backdrop_rgb(
    buf: &Buffer,
    bg: &AmbientBackground,
    x: u16,
    y: u16,
) -> Rgb {
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
            (p00[0] as f32) * w00 + (p10[0] as f32) * w10 + (p01[0] as f32) * w01 + (p11[0] as f32) * w11,
            (p00[1] as f32) * w00 + (p10[1] as f32) * w10 + (p01[1] as f32) * w01 + (p11[1] as f32) * w11,
            (p00[2] as f32) * w00 + (p10[2] as f32) * w10 + (p01[2] as f32) * w01 + (p11[2] as f32) * w11,
            (p00[3] as f32) * w00 + (p10[3] as f32) * w10 + (p01[3] as f32) * w01 + (p11[3] as f32) * w11,
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

    #[test]
    fn test_rat_spritesheet_decoded_and_climax_detection() {
        let frames = decoded_rat_frames();
        assert_eq!(frames.len(), SPRITE_W * SPRITE_H * FRAME_COUNT);

        let mut rat = RatEasterEgg::new();
        let silent = [0.0_f32; NUM_BANDS];
        rat.step(&silent, 1.0 / 60.0, true, true);
        assert!(!rat.is_visible(), "Rat should not be visible on silence");

        // Trigger climax surge
        let climax = [0.85_f32; NUM_BANDS];
        for _ in 0..30 {
            rat.step(&climax, 1.0 / 60.0, true, true);
        }
        assert!(rat.is_visible(), "Rat should appear during musical climax");

        // Manual toggle works even when auto_enabled is false
        let mut rat_manual = RatEasterEgg::new();
        rat_manual.toggle_manual();
        for _ in 0..20 {
            rat_manual.step(&silent, 1.0 / 60.0, false, false);
        }
        assert!(
            rat_manual.is_visible(),
            "Rat should appear when manually toggled via 'r' key"
        );
    }
}
