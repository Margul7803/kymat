use crate::ui::background::{lerp_oklab, oklab_to_rgb, rgb_to_oklab, AmbientBackground};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// Bitmasks for the 8 dots of a Unicode Braille cell (U+2800..U+28FF)
const BRAILLE_DOTS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

#[inline]
pub fn braille_char_from_mask(mask: u8) -> char {
    char::from_u32(0x2800 + (mask as u32)).unwrap_or('⠀')
}

/// Encodes a continuous wave crest across a 2x4 sub-pixel Unicode Braille cell.
pub fn compute_cell_braille_mask(
    cell_row_from_top: u16,
    total_cell_rows: u16,
    sub_y_left_from_bottom: f32,
    sub_y_right_from_bottom: f32,
    prev_sub_y_left: f32,
    next_sub_y_right: f32,
) -> (u8, bool) {
    let total_sub_rows = (total_cell_rows * 4) as f32;
    let mut mask = 0u8;
    let mut is_crest = false;

    let cols = [
        (0usize, sub_y_left_from_bottom, prev_sub_y_left),
        (1usize, sub_y_right_from_bottom, next_sub_y_right),
    ];

    for (dx, sub_h, neighbor_h) in cols {
        let y_top_sub = (total_sub_rows - sub_h).clamp(0.0, total_sub_rows - 0.01);
        let y_neighbor_sub = (total_sub_rows - neighbor_h).clamp(0.0, total_sub_rows - 0.01);
        let crest_min = y_top_sub.min(y_neighbor_sub) - 0.65;
        let crest_max = y_top_sub.max(y_neighbor_sub) + 1.10;

        for dy in 0..4u16 {
            let global_sub_y = (cell_row_from_top * 4 + dy) as f32 + 0.5;
            if global_sub_y >= crest_min && global_sub_y <= crest_max {
                mask |= BRAILLE_DOTS[dx][dy as usize];
                is_crest = true;
            }
        }
    }

    (mask, is_crest)
}

/// Renders the organic double Braille soundwave for View 1 (No-Lyrics mode):
/// - Two superimposed Braille curves (bright primary crest + softer secondary echo crest)
/// - Vertical gradient fill beneath the crest fading into the dark background at the bottom
pub fn render_braille_wave(
    area: Rect,
    buf: &mut Buffer,
    bg: &AmbientBackground,
    primary_sub_heights: &[f32],
    secondary_sub_heights: &[f32],
    opacity: f32,
) {
    let sub_len = (area.width as usize) * 2;
    if area.width < 4
        || area.height < 3
        || primary_sub_heights.len() < sub_len
        || secondary_sub_heights.len() < sub_len
        || opacity <= 0.02
    {
        return;
    }

    let palette = bg.current_palette();
    let total_sub_rows = (area.height * 4) as f32;
    let max_sub_h = total_sub_rows * 0.86;
    let white_ok = rgb_to_oklab(255, 235, 240);

    for col in 0..area.width {
        let screen_x = area.x + col;
        let idx_l = (col as usize) * 2;
        let idx_r = idx_l + 1;

        // Primary curve
        let h1_l = (primary_sub_heights[idx_l] * max_sub_h).clamp(2.5, max_sub_h);
        let h1_r = (primary_sub_heights[idx_r] * max_sub_h).clamp(2.5, max_sub_h);
        let prev1_l = if idx_l > 0 {
            (primary_sub_heights[idx_l - 1] * max_sub_h).clamp(2.5, max_sub_h)
        } else {
            h1_l
        };
        let next1_r = if idx_r + 1 < sub_len {
            (primary_sub_heights[idx_r + 1] * max_sub_h).clamp(2.5, max_sub_h)
        } else {
            h1_r
        };

        // Secondary echo curve
        let h2_l = (secondary_sub_heights[idx_l] * max_sub_h * 0.82).clamp(2.0, max_sub_h);
        let h2_r = (secondary_sub_heights[idx_r] * max_sub_h * 0.82).clamp(2.0, max_sub_h);
        let prev2_l = if idx_l > 0 {
            (secondary_sub_heights[idx_l - 1] * max_sub_h * 0.82).clamp(2.0, max_sub_h)
        } else {
            h2_l
        };
        let next2_r = if idx_r + 1 < sub_len {
            (secondary_sub_heights[idx_r + 1] * max_sub_h * 0.82).clamp(2.0, max_sub_h)
        } else {
            h2_r
        };

        let avg_h1 = 0.5 * (h1_l + h1_r);
        let crest1_sub_y = total_sub_rows - avg_h1;

        for row in 0..area.height {
            let screen_y = area.y + row;
            let (mask1, is_crest1) =
                compute_cell_braille_mask(row, area.height, h1_l, h1_r, prev1_l, next1_r);
            let (mask2, is_crest2) =
                compute_cell_braille_mask(row, area.height, h2_l, h2_r, prev2_l, next2_r);

            let cell_center_sub_y = (row * 4 + 2) as f32;
            let combined_mask = mask1 | mask2;
            let in_gradient = cell_center_sub_y >= crest1_sub_y - 1.5;

            // Empty cells above the crest already have their background rendered by AmbientBackground
            if combined_mask == 0 && !in_gradient {
                continue;
            }

            let base_oklab = bg.oklab_at(screen_x, screen_y);

            // Gradient fill below the crest fading smoothly to #050508 at the bottom
            let bg_rgb = if in_gradient {
                let dist_from_crest = (cell_center_sub_y - crest1_sub_y).max(0.0);
                let norm_depth = (dist_from_crest / avg_h1.max(4.0)).clamp(0.0, 1.0);
                let vertical_fade = (1.0 - norm_depth).powf(1.55);
                let fill_strength = vertical_fade * 0.38 * opacity;
                let wave_tint =
                    lerp_oklab(palette.accent, palette.secondary_accent, norm_depth * 0.65);
                let filled_ok = lerp_oklab(base_oklab, wave_tint, fill_strength);
                oklab_to_rgb(filled_ok)
            } else {
                bg.rgb_at(screen_x, screen_y)
            };

            if let Some(cell) = buf.cell_mut((screen_x, screen_y)) {
                cell.set_bg(Color::Rgb(bg_rgb.0, bg_rgb.1, bg_rgb.2));
                if combined_mask != 0 {
                    let fg_ok = if is_crest1 {
                        let bright = lerp_oklab(palette.accent, white_ok, 0.22);
                        lerp_oklab(base_oklab, bright, 0.88 * opacity)
                    } else if is_crest2 {
                        lerp_oklab(base_oklab, palette.accent, 0.52 * opacity)
                    } else {
                        lerp_oklab(base_oklab, palette.accent, 0.35 * opacity)
                    };
                    let (fr, fg, fb) = oklab_to_rgb(fg_ok);
                    cell.set_char(braille_char_from_mask(combined_mask));
                    cell.set_fg(Color::Rgb(fr, fg, fb));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_braille_encoding_bits() {
        assert_eq!(braille_char_from_mask(0x00), '⠀');
        assert_eq!(braille_char_from_mask(0xFF), '⣿');
        assert_eq!(braille_char_from_mask(0x01), '⠁');
        assert_eq!(braille_char_from_mask(0x80), '⢀');
    }

    #[test]
    fn test_compute_cell_braille_mask_crest_detection() {
        let (mask_crest, is_crest) = compute_cell_braille_mask(2, 4, 8.0, 8.0, 8.0, 8.0);
        assert!(is_crest);
        assert_ne!(mask_crest, 0);

        let (mask_top, is_crest_top) = compute_cell_braille_mask(0, 4, 8.0, 8.0, 8.0, 8.0);
        assert!(!is_crest_top);
        assert_eq!(mask_top, 0);
    }
}
