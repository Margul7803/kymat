use crate::media::cover::CoverArtData;
use crate::ui::background::{AmbientBackground, oklab_to_rgb};
use image::imageops::FilterType;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::time::Duration;

pub type Rgb = (u8, u8, u8);

/// 8 Unicode 2x2 quadrant partition patterns (bits: 0=TL, 1=TR, 2=BL, 3=BR)
/// Each pattern partitions the 4 sub-pixels between `fg` (bit=1) and `bg` (bit=0).
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

/// Default cell height-to-width pixel aspect ratio for JetBrainsMono Nerd Font in Wayland terminals
const DEFAULT_CELL_ASPECT: f32 = 2.25;

/// Pre-calculated cover art cell avoiding per-frame SDF and quadrant fitting at 144 FPS
#[derive(Clone, Copy)]
pub enum CachedCoverCell {
    /// Cell completely outside the rounded corners
    Transparent,
    /// Fully opaque interior cell: quadrant character and colors precomputed once per track
    Opaque { ch: char, fg: Rgb, bg: Rgb },
    /// Semi-transparent rounded corner cell: blended on the fly with the background glow
    Edge { pixels: [Rgb; 4], covs: [f32; 4] },
}

/// 2x2 quadrant cell grid cache for zero-allocation rendering at 144 FPS
#[derive(Default)]
pub struct ScaledCoverCache {
    art_url: String,
    cell_w: u16,
    cell_h: u16,
    cells: Vec<CachedCoverCell>,
    cached_term_dims: (u16, u16),
    cached_cell_aspect: f32,
}

impl ScaledCoverCache {
    /// Queries the terminal's physical pixel dimensions (via `TIOCGWINSZ`) only when the
    /// terminal window dimensions change, returning the exact cell height/width pixel ratio so
    /// the album cover renders as a true 1:1 physical square on screen.
    pub fn cell_aspect_for_window(&mut self, term_w: u16, term_h: u16) -> f32 {
        if self.cached_term_dims != (term_w, term_h) || self.cached_cell_aspect <= 0.0 {
            self.cached_term_dims = (term_w, term_h);
            self.cached_cell_aspect = detect_terminal_cell_aspect();
        }
        self.cached_cell_aspect
    }

    #[inline]
    pub fn cell_aspect_for_area(&mut self, term_w: u16, term_h: u16) -> f32 {
        self.cell_aspect_for_window(term_w, term_h)
    }

    pub fn ensure_cells(
        &mut self,
        cover: &CoverArtData,
        cell_w: u16,
        cell_h: u16,
        corner_radius: f32,
    ) -> &[CachedCoverCell] {
        if self.art_url != cover.art_url
            || self.cell_w != cell_w
            || self.cell_h != cell_h
            || self.cells.len() != (cell_w as usize) * (cell_h as usize)
        {
            self.art_url.clone_from(&cover.art_url);
            self.cell_w = cell_w;
            self.cell_h = cell_h;

            let pw = ((cell_w as u32) * 2).max(4);
            let ph = ((cell_h as u32) * 2).max(4);
            let resized = image::imageops::resize(&*cover.image, pw, ph, FilterType::Lanczos3);

            let pw_f = pw as f32;
            let ph_f = ph as f32;
            // Normalize X into `[0, ph_f]` so rounded corners are circular in physical screen pixels
            let x_scale = ph_f / pw_f;
            let total_cells = (cell_w as usize) * (cell_h as usize);
            self.cells.clear();
            self.cells.reserve(total_cells);

            for cy in 0..cell_h {
                let py0 = (cy * 2) as u32;
                let py1 = py0 + 1;
                for cx in 0..cell_w {
                    let px0 = (cx * 2) as u32;
                    let px1 = px0 + 1;

                    let coords = [(px0, py0), (px1, py0), (px0, py1), (px1, py1)];
                    let mut pixels = [(0u8, 0u8, 0u8); 4];
                    let mut covs = [0.0_f32; 4];
                    let mut max_cov = 0.0_f32;
                    let mut min_cov = 1.0_f32;

                    for (i, &(px, py)) in coords.iter().enumerate() {
                        let cov = rounded_rect_coverage(
                            (px as f32 + 0.5) * x_scale,
                            py as f32 + 0.5,
                            ph_f,
                            ph_f,
                            corner_radius,
                        );
                        max_cov = max_cov.max(cov);
                        min_cov = min_cov.min(cov);
                        let p = resized.get_pixel(px, py).0;
                        pixels[i] = (p[0], p[1], p[2]);
                        covs[i] = cov;
                    }

                    let cell_entry = if max_cov <= 0.02 {
                        CachedCoverCell::Transparent
                    } else if min_cov >= 0.995 {
                        let (ch, fg, bg) = best_quadrant_cell(pixels);
                        CachedCoverCell::Opaque { ch, fg, bg }
                    } else {
                        CachedCoverCell::Edge { pixels, covs }
                    };
                    self.cells.push(cell_entry);
                }
            }
        }

        &self.cells
    }
}

fn detect_terminal_cell_aspect() -> f32 {
    if let Ok(win) = crossterm::terminal::window_size() {
        if win.width > 0 && win.height > 0 && win.columns > 0 && win.rows > 0 {
            let cell_w = (win.width as f32) / (win.columns as f32);
            let cell_h = (win.height as f32) / (win.rows as f32);
            if cell_w > 0.5 && cell_h > 0.5 {
                return (cell_h / cell_w).clamp(1.85, 2.60);
            }
        }
    }
    DEFAULT_CELL_ASPECT
}

/// Metadata view parameters for the cover column renderer
pub struct CoverMetaView<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub position: Duration,
    pub duration: Duration,
    pub show_time_bar_under_artist: bool,
}

/// Signed Distance Field (SDF) coverage test for a rounded rectangle.
#[inline]
pub fn rounded_rect_coverage(px: f32, py: f32, w: f32, h: f32, radius: f32) -> f32 {
    let half_w = w * 0.5;
    let half_h = h * 0.5;
    let r = radius.min(half_w).min(half_h);

    let qx = (px - half_w).abs() - (half_w - r);
    let qy = (py - half_h).abs() - (half_h - r);

    let out_dist = (qx.max(0.0).hypot(qy.max(0.0))) + qx.max(qy).min(0.0) - r;
    (0.5 - out_dist).clamp(0.0, 1.0)
}

#[inline]
fn avg_rgb(sum_r: u32, sum_g: u32, sum_b: u32, count: u32, fallback: Rgb) -> Rgb {
    match count {
        0 => fallback,
        n => ((sum_r / n) as u8, (sum_g / n) as u8, (sum_b / n) as u8),
    }
}

/// Selects the 2x2 Unicode quadrant glyph (`▀`, `▌`, `▞`, `▛`, etc.) and `(fg, bg)` pair
/// minimizing squared RGB error across the 4 sub-pixels of a terminal cell.
pub(crate) fn best_quadrant_cell(pixels: [Rgb; 4]) -> (char, Rgb, Rgb) {
    let mut best_err = u32::MAX;
    let mut best_ch = '▀';
    let mut best_fg = pixels[0];
    let mut best_bg = pixels[2];

    for &(mask, ch) in &QUADRANT_PATTERNS {
        let mut fg_r = 0u32;
        let mut fg_g = 0u32;
        let mut fg_b = 0u32;
        let mut fg_n = 0u32;

        let mut bg_r = 0u32;
        let mut bg_g = 0u32;
        let mut bg_b = 0u32;
        let mut bg_n = 0u32;

        for (i, &(r, g, b)) in pixels.iter().enumerate() {
            if (mask >> i) & 1 == 1 {
                fg_r += r as u32;
                fg_g += g as u32;
                fg_b += b as u32;
                fg_n += 1;
            } else {
                bg_r += r as u32;
                bg_g += g as u32;
                bg_b += b as u32;
                bg_n += 1;
            }
        }

        let bg = avg_rgb(bg_r, bg_g, bg_b, bg_n, pixels[0]);
        let fg = avg_rgb(fg_r, fg_g, fg_b, fg_n, bg);

        let mut err = 0u32;
        for (i, &(r, g, b)) in pixels.iter().enumerate() {
            let ref_c = if (mask >> i) & 1 == 1 { fg } else { bg };
            let dr = (r as i32) - (ref_c.0 as i32);
            let dg = (g as i32) - (ref_c.1 as i32);
            let db = (b as i32) - (ref_c.2 as i32);
            err += (dr * dr + dg * dg + db * db) as u32;
        }

        // Slight tie-breaker preference for horizontal half-block `▀` on flat regions
        if mask == 0b0011 {
            err = err.saturating_sub(18);
        }

        if err < best_err {
            best_err = err;
            best_ch = ch;
            best_fg = fg;
            best_bg = bg;
        }
    }

    (best_ch, best_fg, best_bg)
}

/// Renders the 2x2 Unicode quadrant album cover (true 1:1 physical pixel square),
/// bold title with solid heart icon, artist name, and (when in split view) the
/// passive progress bar beneath the artist.
pub fn render_cover_and_meta(
    area: Rect,
    buf: &mut Buffer,
    bg: &AmbientBackground,
    cover: Option<&CoverArtData>,
    cache: &mut ScaledCoverCache,
    meta: &CoverMetaView<'_>,
    cell_aspect: f32,
) {
    if area.width < 14 || area.height < 10 {
        return;
    }

    let meta_rows = if meta.show_time_bar_under_artist {
        5u16
    } else {
        3u16
    };

    let desired_h = ((area.height as u32 * 52) / 100) as u16;
    let max_cover_h = area.height.saturating_sub(meta_rows + 3).min(38);
    let max_cover_w = area.width.saturating_sub(6);
    let max_h_from_w = ((max_cover_w as f32) / cell_aspect).floor() as u16;
    let cover_h = desired_h.min(max_cover_h).min(max_h_from_w).max(6);
    let cover_w = ((cover_h as f32) * cell_aspect)
        .round()
        .clamp(12.0, max_cover_w.max(12) as f32) as u16;

    let total_block_h = cover_h + meta_rows;
    let vertical_bias = if meta.show_time_bar_under_artist {
        50
    } else {
        36
    };
    let start_y = area.y
        + ((area.height.saturating_sub(total_block_h) as u32 * vertical_bias) / 100) as u16;
    let cover_x = area.x + area.width.saturating_sub(cover_w) / 2;

    let pw = (cover_w * 2) as f32;
    let ph = (cover_h * 2) as f32;
    let x_scale = ph / pw.max(1.0);
    let corner_radius = (ph * 0.045).max(1.8);

    if let Some(cover_data) = cover {
        let cells = cache.ensure_cells(cover_data, cover_w, cover_h, corner_radius);
        let w_usize = cover_w as usize;

        for cy in 0..cover_h {
            let screen_y = start_y + cy;
            if screen_y >= area.bottom() {
                break;
            }
            let row_offset = (cy as usize) * w_usize;

            for cx in 0..cover_w {
                let screen_x = cover_x + cx;
                if screen_x >= area.right() {
                    break;
                }

                match cells[row_offset + (cx as usize)] {
                    CachedCoverCell::Transparent => {}
                    CachedCoverCell::Opaque { ch, fg, bg: c_bg } => {
                        if let Some(cell) = buf.cell_mut((screen_x, screen_y)) {
                            cell.set_char(ch);
                            cell.set_fg(Color::Rgb(fg.0, fg.1, fg.2));
                            cell.set_bg(Color::Rgb(c_bg.0, c_bg.1, c_bg.2));
                        }
                    }
                    CachedCoverCell::Edge { pixels, covs } => {
                        let bg_rgb = bg.rgb_at(screen_x, screen_y);
                        let sub_pixels = [
                            blend_rgb(bg_rgb, pixels[0], covs[0]),
                            blend_rgb(bg_rgb, pixels[1], covs[1]),
                            blend_rgb(bg_rgb, pixels[2], covs[2]),
                            blend_rgb(bg_rgb, pixels[3], covs[3]),
                        ];
                        let (ch, fg, c_bg) = best_quadrant_cell(sub_pixels);
                        if let Some(cell) = buf.cell_mut((screen_x, screen_y)) {
                            cell.set_char(ch);
                            cell.set_fg(Color::Rgb(fg.0, fg.1, fg.2));
                            cell.set_bg(Color::Rgb(c_bg.0, c_bg.1, c_bg.2));
                        }
                    }
                }
            }
        }
    } else {
        let accent = oklab_to_rgb(bg.current_palette().accent);
        for cy in 0..cover_h {
            let screen_y = start_y + cy;
            let py_top = cy * 2;
            for cx in 0..cover_w {
                let screen_x = cover_x + cx;
                let cov = rounded_rect_coverage(
                    ((cx * 2) as f32 + 0.5) * x_scale,
                    py_top as f32 + 0.5,
                    ph,
                    ph,
                    corner_radius,
                );
                if cov <= 0.05 {
                    continue;
                }
                let bg_rgb = bg.rgb_at(screen_x, screen_y);
                let card_rgb = blend_rgb(bg_rgb, accent, 0.14 * cov);
                if let Some(cell) = buf.cell_mut((screen_x, screen_y)) {
                    cell.set_char('▀');
                    cell.set_fg(Color::Rgb(card_rgb.0, card_rgb.1, card_rgb.2));
                    cell.set_bg(Color::Rgb(card_rgb.0, card_rgb.1, card_rgb.2));
                }
            }
        }
        draw_centered_text(
            buf,
            area.x,
            start_y + cover_h / 2,
            area.width,
            "♪",
            Style::default()
                .fg(Color::Rgb(accent.0, accent.1, accent.2))
                .add_modifier(Modifier::BOLD),
        );
    }

    // 1. Bold white title (#FFFFFF) followed by solid Nerd Font heart `\u{f004}`
    let title_y = start_y + cover_h + 1;
    render_title_with_heart(buf, area.x, title_y, area.width, meta.title, bg);

    // 2. Artist name (#969EAA)
    let artist_y = title_y + 1;
    if !meta.artist.is_empty() {
        draw_centered_text(
            buf,
            area.x,
            artist_y,
            area.width,
            meta.artist,
            Style::default().fg(Color::Rgb(150, 158, 170)),
        );
    }

    // 3. In View 2 (Split Lyrics), the passive time bar sits directly below the artist
    if meta.show_time_bar_under_artist {
        let bar_y = artist_y + 2;
        let bar_w = cover_w.saturating_sub(4).clamp(18, 36);
        if bar_y < area.bottom() && (!meta.duration.is_zero() || !meta.position.is_zero()) {
            let bar_area = Rect::new(area.x, bar_y, area.width, 1);
            render_passive_time_bar(buf, bar_area, bar_w, meta.position, meta.duration, bg);
        }
    }
}

fn render_title_with_heart(
    buf: &mut Buffer,
    area_x: u16,
    y: u16,
    area_w: u16,
    title: &str,
    bg: &AmbientBackground,
) {
    if y >= buf.area().bottom() || area_w < 6 {
        return;
    }
    let display_title = if title.is_empty() {
        "Waiting for playback…"
    } else {
        title
    };

    let has_heart = !title.is_empty();
    let max_title_chars = (area_w.saturating_sub(6)) as usize;
    let total_chars = display_title.chars().count();
    let is_truncated = total_chars > max_title_chars;
    let shown_chars = if is_truncated {
        max_title_chars
    } else {
        total_chars
    };

    let title_len = shown_chars as u16;
    let total_len = if has_heart { title_len + 2 } else { title_len };
    let start_x = area_x + area_w.saturating_sub(total_len) / 2;

    let title_style = Style::default()
        .fg(Color::Rgb(255, 255, 255))
        .add_modifier(Modifier::BOLD);

    let limit = if is_truncated {
        shown_chars.saturating_sub(1)
    } else {
        shown_chars
    };

    for (i, ch) in display_title.chars().take(limit).enumerate() {
        let x = start_x + i as u16;
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(ch);
            cell.set_style(title_style);
        }
    }
    if is_truncated && shown_chars > 0 {
        let x = start_x + (shown_chars - 1) as u16;
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char('…');
            cell.set_style(title_style);
        }
    }

    if has_heart {
        let heart_x = start_x + title_len + 1;
        let (ar, ag, ab) = oklab_to_rgb(bg.current_palette().accent);
        if let Some(cell) = buf.cell_mut((heart_x, y)) {
            // '\u{f004}' is the solid heart (nf-fa-heart) in JetBrainsMono Nerd Font
            cell.set_char('\u{f004}');
            cell.set_style(Style::default().fg(Color::Rgb(ar, ag, ab)));
        }
    }
}

pub fn render_passive_time_bar(
    buf: &mut Buffer,
    row_area: Rect,
    desired_bar_cells: u16,
    position: Duration,
    duration: Duration,
    bg: &AmbientBackground,
) {
    let y = row_area.y;
    let area_x = row_area.x;
    let area_w = row_area.width;
    if y >= buf.area().bottom() {
        return;
    }
    let left_ts = format_mm_ss(position);
    let right_ts = format_mm_ss(duration);

    let bar_cells = desired_bar_cells
        .min(area_w.saturating_sub(16))
        .clamp(10, 42) as usize;
    let ratio = if duration.as_secs_f32() > 0.0 {
        (position.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((ratio * (bar_cells.saturating_sub(1) as f32)).round() as usize)
        .min(bar_cells.saturating_sub(1));

    let total_str_w = left_ts.len() + 1 + bar_cells + 1 + right_ts.len();
    if total_str_w > area_w as usize {
        return;
    }

    let start_x = area_x + (area_w - total_str_w as u16) / 2;
    let dim_style = Style::default().fg(Color::Rgb(145, 152, 165));
    let (ar, ag, ab) = oklab_to_rgb(bg.current_palette().accent);
    let active_style = Style::default().fg(Color::Rgb(ar, ag, ab));
    let track_style = Style::default().fg(Color::Rgb(65, 70, 82));

    let mut cur_x = start_x;
    for ch in left_ts.chars() {
        if let Some(c) = buf.cell_mut((cur_x, y)) {
            c.set_char(ch);
            c.set_style(dim_style);
        }
        cur_x += 1;
    }
    cur_x += 1;

    for i in 0..bar_cells {
        if let Some(c) = buf.cell_mut((cur_x, y)) {
            if i < filled {
                c.set_char('━');
                c.set_style(active_style);
            } else if i == filled {
                c.set_char('●');
                c.set_style(Style::default().fg(Color::Rgb(255, 255, 255)));
            } else {
                c.set_char('─');
                c.set_style(track_style);
            }
        }
        cur_x += 1;
    }

    cur_x += 1;
    for ch in right_ts.chars() {
        if let Some(c) = buf.cell_mut((cur_x, y)) {
            c.set_char(ch);
            c.set_style(dim_style);
        }
        cur_x += 1;
    }
}

pub fn format_mm_ss(d: Duration) -> String {
    let total_secs = d.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{}:{:02}", mins, secs)
}

pub fn draw_centered_text(
    buf: &mut Buffer,
    area_x: u16,
    y: u16,
    area_w: u16,
    text: &str,
    style: Style,
) {
    if y >= buf.area().bottom() || area_w < 4 {
        return;
    }
    let max_chars = (area_w.saturating_sub(2)) as usize;
    let total_chars = text.chars().count();
    let is_truncated = total_chars > max_chars;
    let shown_chars = if is_truncated { max_chars } else { total_chars };

    let char_count = shown_chars as u16;
    let start_x = area_x + area_w.saturating_sub(char_count) / 2;

    let limit = if is_truncated {
        shown_chars.saturating_sub(1)
    } else {
        shown_chars
    };

    for (i, ch) in text.chars().take(limit).enumerate() {
        let x = start_x + i as u16;
        if x < area_x + area_w {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(ch);
                cell.set_style(style);
            }
        }
    }
    if is_truncated && shown_chars > 0 {
        let x = start_x + (shown_chars - 1) as u16;
        if x < area_x + area_w {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char('…');
                cell.set_style(style);
            }
        }
    }
}

#[inline]
fn blend_rgb(bg: Rgb, fg: Rgb, alpha: f32) -> Rgb {
    let a = alpha.clamp(0.0, 1.0);
    let r = (bg.0 as f32 * (1.0 - a) + fg.0 as f32 * a).round() as u8;
    let g = (bg.1 as f32 * (1.0 - a) + fg.1 as f32 * a).round() as u8;
    let b = (bg.2 as f32 * (1.0 - a) + fg.2 as f32 * a).round() as u8;
    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rounded_rect_coverage_corners_and_center() {
        assert_eq!(rounded_rect_coverage(16.0, 16.0, 32.0, 32.0, 4.0), 1.0);
        assert_eq!(rounded_rect_coverage(0.1, 0.1, 32.0, 32.0, 4.0), 0.0);
    }

    #[test]
    fn test_format_mm_ss() {
        assert_eq!(format_mm_ss(Duration::from_secs(15)), "0:15");
        assert_eq!(format_mm_ss(Duration::from_secs(176)), "2:56");
    }

    #[test]
    fn test_best_quadrant_cell_vertical_split() {
        let left_red_right_blue = [
            (240, 20, 20),
            (20, 20, 240),
            (240, 20, 20),
            (20, 20, 240),
        ];
        let (ch, fg, bg) = best_quadrant_cell(left_red_right_blue);
        assert_eq!(ch, '▌');
        assert_eq!(fg, (240, 20, 20));
        assert_eq!(bg, (20, 20, 240));
    }

    #[test]
    fn test_square_cover_aspect_ratio() {
        let area = Rect::new(0, 0, 72, 46);
        let mut buf = Buffer::empty(area);
        let bg = AmbientBackground::new(1.2);
        let mut cache = ScaledCoverCache::default();
        let meta = CoverMetaView {
            title: "Lover Is a Day",
            artist: "Cuco",
            position: Duration::from_secs(102),
            duration: Duration::from_secs(456),
            show_time_bar_under_artist: true,
        };

        let cell_aspect = cache.cell_aspect_for_window(area.width, area.height);
        render_cover_and_meta(area, &mut buf, &bg, None, &mut cache, &meta, cell_aspect);

        // Verify that the cached cell aspect produces a true square (~2.25 cols per row)
        assert!((cache.cached_cell_aspect - 2.25).abs() < 0.35);
    }
}
