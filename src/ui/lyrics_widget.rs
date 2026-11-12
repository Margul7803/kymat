use crate::media::lyrics::SyncedLyrics;
use crate::ui::background::{AmbientBackground, lerp_oklab, oklab_to_rgb, rgb_to_oklab};
use crate::ui::cover_widget::Rgb;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::time::Duration;

/// 2x2 Unicode quadrant partition table (bits: 0=TL, 1=TR, 2=BL, 3=BR)
const QUADRANT_PATTERNS: [(u8, char); 8] = [
    (0b0000, ' '),
    (0b0001, '▘'),
    (0b0010, '▝'),
    (0b0011, '▀'),
    (0b0100, '▖'),
    (0b0101, '▌'),
    (0b0110, '▞'),
    (0b0111, '▛'),
];

const SUB_H: usize = 8;
const SUB_LINE_ROWS: i32 = 4;
const ENTRY_GAP_ROWS: i32 = 1;

struct CachedSubLine {
    /// Sharp unblurred sub-pixel grid of size `max_sub_w * 8`
    sharp_grid: Vec<f32>,
    /// Pre-blurred sub-pixel grid for inactive lines, computed once at layout time
    blurred_grid: Vec<f32>,
    used_sub_w: usize,
}

struct CachedLyricEntry {
    virtual_y: i32,
    sub_lines: Vec<CachedSubLine>,
}

/// Vertical lyrics scroll state + zero-allocation 144 FPS sub-pixel rasterization cache
#[derive(Default)]
pub struct LyricsScrollState {
    smooth_row: f32,
    initialized: bool,
    cached_line_count: usize,
    cached_first_text: String,
    cached_wrap_chars: usize,
    cached_max_sub_w: usize,
    entries: Vec<CachedLyricEntry>,
}

impl LyricsScrollState {
    pub fn reset(&mut self) {
        self.smooth_row = 0.0;
        self.initialized = false;
        self.cached_line_count = 0;
        self.cached_first_text.clear();
        self.entries.clear();
    }

    pub fn update(&mut self, target_row: f32, dt: f32) -> f32 {
        if !self.initialized {
            self.smooth_row = target_row;
            self.initialized = true;
        } else {
            let diff = target_row - self.smooth_row;
            if diff.abs() < 0.08 {
                self.smooth_row = target_row;
            } else {
                let factor = 1.0 - (-14.0 * dt.clamp(0.001, 0.1)).exp();
                self.smooth_row += diff * factor;
            }
        }
        self.smooth_row
    }

    fn ensure_layout(&mut self, lyrics: &SyncedLyrics, wrap_chars: usize, max_sub_w: usize) {
        let first_text = lyrics.lines.first().map(|l| l.text.as_str()).unwrap_or("");

        if self.cached_line_count == lyrics.lines.len()
            && self.cached_wrap_chars == wrap_chars
            && self.cached_max_sub_w == max_sub_w
            && self.cached_first_text == first_text
            && !self.entries.is_empty()
        {
            return;
        }

        self.cached_line_count = lyrics.lines.len();
        self.cached_first_text = first_text.to_string();
        self.cached_wrap_chars = wrap_chars;
        self.cached_max_sub_w = max_sub_w;
        self.entries.clear();
        self.entries.reserve(lyrics.lines.len());

        let mut blur_temp = vec![0.0_f32; max_sub_w * SUB_H];
        let mut cursor_y = 0i32;
        for line in &lyrics.lines {
            let wrapped = wrap_lyric_text(&line.text, wrap_chars);
            let mut sub_lines = Vec::with_capacity(wrapped.len());
            for sub_text in &wrapped {
                let (sharp_grid, used_sub_w) = rasterize_line_subpixels(sub_text, max_sub_w);
                let mut blurred_grid = sharp_grid.clone();
                apply_gaussian_blur_in_place(
                    &mut blurred_grid,
                    &mut blur_temp,
                    max_sub_w,
                    SUB_H,
                    0.65,
                );
                sub_lines.push(CachedSubLine {
                    sharp_grid,
                    blurred_grid,
                    used_sub_w,
                });
            }
            let h = (sub_lines.len() as i32) * SUB_LINE_ROWS;
            self.entries.push(CachedLyricEntry {
                virtual_y: cursor_y,
                sub_lines,
            });
            cursor_y += h + ENTRY_GAP_ROWS;
        }
    }
}

/// Word-wraps a lyric line into sub-lines of at most `max_width` characters
pub fn wrap_lyric_text(text: &str, max_width: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return vec!["• • •".to_string()];
    }
    let limit = max_width.clamp(14, 42);
    let mut wrapped = Vec::new();
    let mut current = String::new();

    for word in trimmed.split_whitespace() {
        let word_len = word.chars().count();
        let cur_len = current.chars().count();
        if cur_len == 0 {
            current.push_str(word);
        } else if cur_len + 1 + word_len <= limit {
            current.push(' ');
            current.push_str(word);
        } else {
            wrapped.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        wrapped.push(current);
    }
    if wrapped.is_empty() {
        wrapped.push("• • •".to_string());
    }
    wrapped
}

/// Returns the 5x6 sub-pixel bitmap matrix (width 5, height 6 = 3 terminal rows in 2x2 quadrants)
/// and horizontal sub-pixel advance for a character.
fn glyph_bitmap_5x6(ch: char) -> ([u8; 6], usize) {
    let c = match ch {
        '’' | '‘' | '`' => '\'',
        '–' | '—' => '-',
        'œ' => 'o',
        'Œ' => 'O',
        other => other,
    };

    match c {
        ' ' => ([0, 0, 0, 0, 0, 0], 3),
        '.' => ([0, 0, 0, 0, 0b01100, 0b01100], 3),
        ',' => ([0, 0, 0, 0, 0b00110, 0b01100], 3),
        '\'' => ([0b01100, 0b01100, 0, 0, 0, 0], 3),
        '-' => ([0, 0, 0b11110, 0, 0, 0], 4),
        '!' => ([0b01100, 0b01100, 0b01100, 0, 0b01100, 0], 3),
        '?' => ([0b01110, 0b10011, 0b00110, 0b00100, 0, 0b00100], 5),
        '•' => ([0, 0, 0b01100, 0b01100, 0, 0], 4),
        '(' => ([0b00110, 0b01100, 0b01100, 0b01100, 0b00110, 0], 4),
        ')' => ([0b01100, 0b00110, 0b00110, 0b00110, 0b01100, 0], 4),

        // Uppercase
        'A' | 'À' | 'Â' => ([0b01110, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'B' => ([0b11110, 0b11011, 0b11110, 0b11011, 0b11110, 0], 5),
        'C' | 'Ç' => ([0b01111, 0b11000, 0b11000, 0b11000, 0b01111, 0], 5),
        'D' => ([0b11110, 0b11011, 0b11011, 0b11011, 0b11110, 0], 5),
        'E' | 'É' | 'È' | 'Ê' | 'Ë' => ([0b11111, 0b11000, 0b11110, 0b11000, 0b11111, 0], 5),
        'F' => ([0b11111, 0b11000, 0b11110, 0b11000, 0b11000, 0], 5),
        'G' => ([0b01111, 0b11000, 0b11011, 0b11011, 0b01111, 0], 5),
        'H' => ([0b11011, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'I' | 'Î' | 'Ï' => ([0b01110, 0b00100, 0b00100, 0b00100, 0b01110, 0], 4),
        'J' => ([0b00111, 0b00011, 0b00011, 0b11011, 0b01110, 0], 5),
        'K' => ([0b11011, 0b11110, 0b11100, 0b11110, 0b11011, 0], 5),
        'L' => ([0b11000, 0b11000, 0b11000, 0b11000, 0b11111, 0], 5),
        'M' => ([0b10001, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'N' => ([0b11011, 0b11111, 0b11111, 0b11011, 0b11011, 0], 5),
        'O' | 'Ô' => ([0b01110, 0b11011, 0b11011, 0b11011, 0b01110, 0], 5),
        'P' => ([0b11110, 0b11011, 0b11110, 0b11000, 0b11000, 0], 5),
        'Q' => ([0b01110, 0b11011, 0b11011, 0b11111, 0b01111, 0b00011], 5),
        'R' => ([0b11110, 0b11011, 0b11110, 0b11110, 0b11011, 0], 5),
        'S' => ([0b01111, 0b11000, 0b01110, 0b00011, 0b11110, 0], 5),
        'T' => ([0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0], 5),
        'U' | 'Ù' | 'Û' | 'Ü' => ([0b11011, 0b11011, 0b11011, 0b11011, 0b01110, 0], 5),
        'V' => ([0b11011, 0b11011, 0b11011, 0b01110, 0b00100, 0], 5),
        'W' => ([0b11011, 0b11011, 0b11111, 0b11111, 0b10001, 0], 5),
        'X' => ([0b11011, 0b01110, 0b00100, 0b01110, 0b11011, 0], 5),
        'Y' => ([0b11011, 0b01110, 0b00100, 0b00100, 0b00100, 0], 5),
        'Z' => ([0b11111, 0b00110, 0b01100, 0b11000, 0b11111, 0], 5),

        // Lowercase & accented vowels
        'a' => ([0, 0b01110, 0b00011, 0b01111, 0b11011, 0b01111], 5),
        'à' | 'â' => ([0b01100, 0b01110, 0b00011, 0b01111, 0b11011, 0b01111], 5),
        'b' => ([0b11000, 0b11000, 0b11110, 0b11011, 0b11110, 0], 5),
        'c' => ([0, 0b01111, 0b11000, 0b11000, 0b01111, 0], 5),
        'ç' => ([0, 0b01111, 0b11000, 0b11000, 0b01111, 0b00100], 5),
        'd' => ([0b00011, 0b00011, 0b01111, 0b11011, 0b01111, 0], 5),
        'e' => ([0, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'é' => ([0b00110, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'è' => ([0b01100, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'ê' | 'ë' => ([0b01110, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'f' => ([0b00111, 0b01100, 0b11110, 0b01100, 0b01100, 0], 4),
        'g' => ([0, 0b01111, 0b11011, 0b01111, 0b00011, 0b01110], 5),
        'h' => ([0b11000, 0b11000, 0b11110, 0b11011, 0b11011, 0], 5),
        'i' => ([0b01100, 0, 0b01100, 0b01100, 0b01100, 0], 3),
        'î' | 'ï' => ([0b01110, 0, 0b01100, 0b01100, 0b01100, 0], 3),
        'j' => ([0b00110, 0, 0b00110, 0b00110, 0b10110, 0b01100], 4),
        'k' => ([0b11000, 0b11011, 0b11110, 0b11110, 0b11011, 0], 5),
        'l' => ([0b01100, 0b01100, 0b01100, 0b01100, 0b00110, 0], 3),
        'm' => ([0, 0b11010, 0b11111, 0b11111, 0b11011, 0], 5),
        'n' => ([0, 0b11110, 0b11011, 0b11011, 0b11011, 0], 5),
        'o' => ([0, 0b01110, 0b11011, 0b11011, 0b01110, 0], 5),
        'ô' => ([0b01110, 0b01110, 0b11011, 0b11011, 0b01110, 0], 5),
        'p' => ([0, 0b11110, 0b11011, 0b11110, 0b11000, 0b11000], 5),
        'q' => ([0, 0b01111, 0b11011, 0b01111, 0b00011, 0b00011], 5),
        'r' => ([0, 0b11110, 0b11000, 0b11000, 0b11000, 0], 4),
        's' => ([0, 0b01111, 0b11100, 0b00111, 0b11110, 0], 5),
        't' => ([0b01100, 0b11110, 0b01100, 0b01100, 0b00111, 0], 4),
        'u' => ([0, 0b11011, 0b11011, 0b11011, 0b01111, 0], 5),
        'ù' | 'û' | 'ü' => ([0b01100, 0b11011, 0b11011, 0b11011, 0b01111, 0], 5),
        'v' => ([0, 0b11011, 0b11011, 0b01110, 0b00100, 0], 5),
        'w' => ([0, 0b11011, 0b11111, 0b11111, 0b01010, 0], 5),
        'x' => ([0, 0b11011, 0b01110, 0b01110, 0b11011, 0], 5),
        'y' => ([0, 0b11011, 0b11011, 0b01111, 0b00011, 0b01110], 5),
        'z' => ([0, 0b11111, 0b00110, 0b01100, 0b11111, 0], 5),

        // Digits
        '0' => ([0b01110, 0b11011, 0b11011, 0b11011, 0b01110, 0], 5),
        '1' => ([0b00110, 0b01110, 0b00110, 0b00110, 0b01111, 0], 4),
        '2' => ([0b01110, 0b11011, 0b00110, 0b01100, 0b11111, 0], 5),
        '3' => ([0b11110, 0b00011, 0b01110, 0b00011, 0b11110, 0], 5),
        '4' => ([0b11011, 0b11011, 0b11111, 0b00011, 0b00011, 0], 5),
        '5' => ([0b11111, 0b11000, 0b11110, 0b00011, 0b11110, 0], 5),
        '6' => ([0b01111, 0b11000, 0b11110, 0b11011, 0b01110, 0], 5),
        '7' => ([0b11111, 0b00011, 0b00110, 0b01100, 0b01100, 0], 5),
        '8' => ([0b01110, 0b11011, 0b01110, 0b11011, 0b01110, 0], 5),
        '9' => ([0b01110, 0b11011, 0b01111, 0b00011, 0b11110, 0], 5),

        _ => ([0, 0b01110, 0b01110, 0b01110, 0, 0], 4),
    }
}

/// Rasterizes a text line into a `(width_sub, 8)` sub-pixel opacity grid
fn rasterize_line_subpixels(text: &str, max_sub_w: usize) -> (Vec<f32>, usize) {
    let mut cursor_x = 2usize;
    let mut grid = vec![0.0_f32; max_sub_w * SUB_H];

    for ch in text.chars() {
        let (rows, advance) = glyph_bitmap_5x6(ch);
        if cursor_x + advance + 2 >= max_sub_w {
            break;
        }
        for (ry, &bits) in rows.iter().enumerate() {
            let gy = ry + 1;
            for rx in 0..5usize {
                if (bits >> (4 - rx)) & 1 == 1 {
                    let gx = cursor_x + rx;
                    if gx < max_sub_w && gy < SUB_H {
                        grid[gy * max_sub_w + gx] = 1.0;
                    }
                }
            }
        }
        cursor_x += advance + 1;
    }

    let used_w = (cursor_x + 2).min(max_sub_w);
    (grid, used_w)
}

/// Applies an in-place 2D separable Gaussian blur filter over the sub-pixel grid.
fn apply_gaussian_blur_in_place(
    grid: &mut [f32],
    temp: &mut [f32],
    w: usize,
    h: usize,
    blur_strength: f32,
) {
    if blur_strength <= 0.05 || w < 3 || h < 3 {
        return;
    }

    let passes = if blur_strength > 0.65 { 2 } else { 1 };
    let alpha = (blur_strength * 0.85).clamp(0.15, 0.85);
    let center_w = 1.0 - alpha * 0.55;
    let side_w = alpha * 0.275;

    for _ in 0..passes {
        for y in 0..h {
            let row = y * w;
            for x in 0..w {
                let left = if x > 0 { grid[row + x - 1] } else { 0.0 };
                let mid = grid[row + x];
                let right = if x + 1 < w { grid[row + x + 1] } else { 0.0 };
                temp[row + x] = left * side_w + mid * center_w + right * side_w;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let up = if y > 0 { temp[(y - 1) * w + x] } else { 0.0 };
                let mid = temp[y * w + x];
                let down = if y + 1 < h { temp[(y + 1) * w + x] } else { 0.0 };
                grid[y * w + x] = up * side_w + mid * center_w + down * side_w;
            }
        }
    }
}

/// Converts 4 sub-pixel alpha values `[TL, TR, BL, BR]` into a 2x2 Unicode quadrant character
/// and Oklab-interpolated `(fg, bg)` colors.
fn best_quadrant_alpha(
    alphas: [f32; 4],
    bg_ok: palette::Oklab,
    text_ok: palette::Oklab,
) -> Option<(char, Rgb, Rgb)> {
    let max_a = alphas.iter().copied().fold(0.0_f32, f32::max);
    if max_a < 0.03 {
        return None;
    }

    let mut best_err = f32::MAX;
    let mut best_ch = '▀';
    let mut best_fg_a = max_a;
    let mut best_bg_a = 0.0_f32;

    for &(mask, ch) in &QUADRANT_PATTERNS {
        let mut fg_sum = 0.0_f32;
        let mut fg_n = 0.0_f32;
        let mut bg_sum = 0.0_f32;
        let mut bg_n = 0.0_f32;

        for (i, &a) in alphas.iter().enumerate() {
            if (mask >> i) & 1 == 1 {
                fg_sum += a;
                fg_n += 1.0;
            } else {
                bg_sum += a;
                bg_n += 1.0;
            }
        }

        let bg_a = if bg_n > 0.0 { bg_sum / bg_n } else { 0.0 };
        let fg_a = if fg_n > 0.0 { fg_sum / fg_n } else { bg_a };

        let mut err = 0.0_f32;
        for (i, &a) in alphas.iter().enumerate() {
            let ref_a = if (mask >> i) & 1 == 1 { fg_a } else { bg_a };
            let diff = a - ref_a;
            err += diff * diff;
        }

        if err < best_err {
            best_err = err;
            best_ch = ch;
            best_fg_a = fg_a;
            best_bg_a = bg_a;
        }
    }

    let fg_ok = lerp_oklab(bg_ok, text_ok, best_fg_a.clamp(0.0, 1.0));
    let cell_bg_ok = lerp_oklab(bg_ok, text_ok, best_bg_a.clamp(0.0, 1.0));
    Some((best_ch, oklab_to_rgb(fg_ok), oklab_to_rgb(cell_bg_ok)))
}

/// Parameters for rendering the synchronized lyrics column
pub struct LyricsRenderParams<'a> {
    pub bg: &'a AmbientBackground,
    pub lyrics: &'a SyncedLyrics,
    pub position: Duration,
    pub dt: f32,
    pub opacity: f32,
}

/// Renders synchronized lyrics using large anti-aliased 2x2 Unicode quadrant pixel-art typography
/// with real 2D Gaussian blur on past and upcoming lines (0 allocations per frame).
pub fn render_synced_lyrics(
    area: Rect,
    buf: &mut Buffer,
    scroll: &mut LyricsScrollState,
    params: &LyricsRenderParams<'_>,
) {
    let lyrics = params.lyrics;
    let opacity = params.opacity;
    if area.width < 20 || area.height < 12 || lyrics.lines.is_empty() || opacity <= 0.05 {
        return;
    }

    if area.height < 28 {
        render_compact_fallback(area, buf, scroll, params);
        return;
    }

    let avail_cols = area.width.saturating_sub(4) as usize;
    let wrap_chars = (avail_cols / 3).clamp(14, 34);
    let max_sub_w = avail_cols * 2;

    scroll.ensure_layout(lyrics, wrap_chars, max_sub_w);

    let active_idx = lyrics.active_line_index(params.position).unwrap_or(0);
    let target_virtual_y = scroll
        .entries
        .get(active_idx)
        .map(|e| e.virtual_y)
        .unwrap_or(0) as f32;
    let smooth_virtual_y = scroll.update(target_virtual_y, params.dt);

    let anchor_screen_y = area.y + ((area.height as u32 * 35) / 100).max(4) as u16;
    let max_above_rows = (anchor_screen_y - area.y).saturating_sub(2) as f32;
    let max_below_rows = (area.bottom() - anchor_screen_y).saturating_sub(2) as f32;

    let white_ok = rgb_to_oklab(255, 255, 255);
    let inactive_base_ok = rgb_to_oklab(175, 172, 180);

    for (idx, entry) in scroll.entries.iter().enumerate() {
        let entry_virtual_y = entry.virtual_y as f32;
        let is_active = idx == active_idx;
        let entry_dist = (idx as f32 - active_idx as f32).abs();

        for (sub_i, sub_line) in entry.sub_lines.iter().enumerate() {
            let rel_rows =
                (entry_virtual_y + (sub_i as i32 * SUB_LINE_ROWS) as f32) - smooth_virtual_y;
            let top_screen_y_i32 = (anchor_screen_y as i32) + (rel_rows.round() as i32);

            if top_screen_y_i32 + SUB_LINE_ROWS < (area.y as i32 + 1)
                || top_screen_y_i32 >= (area.bottom() as i32 - 1)
            {
                continue;
            }

            let norm_dist = if rel_rows < 0.0 {
                (-rel_rows / max_above_rows.max(6.0)).clamp(0.0, 1.0)
            } else {
                (rel_rows / max_below_rows.max(10.0)).clamp(0.0, 1.0)
            };

            let depth_fade = if is_active {
                0.0
            } else {
                (0.42 + norm_dist.powf(1.1) * 0.46 + (entry_dist * 0.025).min(0.10))
                    .clamp(0.42, 0.90)
            };

            let effective_fade = 1.0 - (1.0 - depth_fade) * opacity.clamp(0.0, 1.0);
            let base_ok = if is_active { white_ok } else { inactive_base_ok };

            let grid_slice: &[f32] = if is_active {
                &sub_line.sharp_grid
            } else {
                &sub_line.blurred_grid
            };

            let used_cells_w = sub_line.used_sub_w.div_ceil(2);
            let start_x = area.x + 2;

            for cell_row in 0..4i32 {
                let sy_i32 = top_screen_y_i32 + cell_row;
                if sy_i32 < (area.y as i32 + 1) || sy_i32 >= (area.bottom() as i32 - 1) {
                    continue;
                }
                let sy = sy_i32 as u16;
                let py0 = (cell_row as usize) * 2;
                let py1 = py0 + 1;

                for cx in 0..used_cells_w {
                    let sx = start_x + cx as u16;
                    if sx >= area.right().saturating_sub(1) {
                        break;
                    }
                    let px0 = cx * 2;
                    let px1 = (px0 + 1).min(max_sub_w - 1);

                    let vis_scale = 1.0 - effective_fade;
                    let a_tl = grid_slice[py0 * max_sub_w + px0] * vis_scale;
                    let a_tr = grid_slice[py0 * max_sub_w + px1] * vis_scale;
                    let a_bl = grid_slice[py1 * max_sub_w + px0] * vis_scale;
                    let a_br = grid_slice[py1 * max_sub_w + px1] * vis_scale;

                    let bg_ok = params.bg.oklab_at(sx, sy);
                    if let Some((ch, fg_rgb, cell_bg_rgb)) =
                        best_quadrant_alpha([a_tl, a_tr, a_bl, a_br], bg_ok, base_ok)
                    {
                        if let Some(cell) = buf.cell_mut((sx, sy)) {
                            cell.set_char(ch);
                            cell.set_fg(Color::Rgb(fg_rgb.0, fg_rgb.1, fg_rgb.2));
                            cell.set_bg(Color::Rgb(cell_bg_rgb.0, cell_bg_rgb.1, cell_bg_rgb.2));
                        }
                    }
                }
            }
        }
    }
}

fn render_compact_fallback(
    area: Rect,
    buf: &mut Buffer,
    scroll: &mut LyricsScrollState,
    params: &LyricsRenderParams<'_>,
) {
    let active_idx = params.lyrics.active_line_index(params.position).unwrap_or(0);
    let smooth_center = scroll.update(active_idx as f32, params.dt);
    let center_y = area.y + area.height / 2;
    let white_ok = rgb_to_oklab(255, 255, 255);
    let inactive_ok = rgb_to_oklab(175, 172, 180);

    for (idx, line) in params.lyrics.lines.iter().enumerate() {
        let rel = (idx as f32) - smooth_center;
        let sy_i32 = (center_y as i32) + (rel * 2.0).round() as i32;
        if sy_i32 < area.y as i32 || sy_i32 >= area.bottom() as i32 {
            continue;
        }
        let sy = sy_i32 as u16;
        let is_active = idx == active_idx;
        let fade = if is_active {
            0.0
        } else {
            (0.45 + rel.abs() * 0.12).clamp(0.45, 0.88)
        };
        let eff_fade = 1.0 - (1.0 - fade) * params.opacity;
        let base_ok = if is_active { white_ok } else { inactive_ok };

        for (i, ch) in line.text.chars().enumerate() {
            let sx = area.x + 2 + i as u16;
            if sx >= area.right() {
                break;
            }
            let bg_ok = params.bg.oklab_at(sx, sy);
            let (r, g, b) = oklab_to_rgb(lerp_oklab(base_ok, bg_ok, eff_fade));
            if let Some(cell) = buf.cell_mut((sx, sy)) {
                cell.set_char(ch);
                let mut style = Style::default().fg(Color::Rgb(r, g, b));
                if is_active {
                    style = style.add_modifier(Modifier::BOLD);
                }
                cell.set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_lyric_text_splits_long_lines_cleanly() {
        let line = "When the night falls down around the neon city lights";
        let wrapped = wrap_lyric_text(line, 34);
        assert_eq!(
            wrapped,
            vec![
                "When the night falls down around".to_string(),
                "the neon city lights".to_string()
            ]
        );
    }

    #[test]
    fn test_subpixel_rasterizer_and_gaussian_blur() {
        let (mut grid, used_w) = rasterize_line_subpixels("Midnight", 120);
        let mut temp = vec![0.0_f32; 120 * SUB_H];
        assert!(used_w > 20);
        let peak_before = grid.iter().copied().fold(0.0_f32, f32::max);
        assert_eq!(peak_before, 1.0);

        apply_gaussian_blur_in_place(&mut grid, &mut temp, 120, SUB_H, 0.8);
        let peak_after = grid.iter().copied().fold(0.0_f32, f32::max);
        assert!(peak_after > 0.2 && peak_after < 0.95);
    }
}
