use crate::media::lyrics::SyncedLyrics;
use crate::ui::background::{lerp_oklab, oklab_to_rgb, rgb_to_oklab, AmbientBackground};
use crate::ui::cover_widget::{fit_text_to_cols, Rgb};
use crate::ui::glyph_fallback::GlyphFallbackCache;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::time::Duration;
use unicode_normalization::UnicodeNormalization;
use unicode_width::UnicodeWidthChar;

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

const DEFAULT_SUB_H: usize = 8;
const CJK_SUB_H: usize = 10;
const ENTRY_GAP_ROWS: i32 = 1;

struct CachedSubLine {
    /// Sharp unblurred sub-pixel grid of size `max_sub_w * sub_h`
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
    cached_sub_h: usize,
    sub_line_rows: i32,
    entries: Vec<CachedLyricEntry>,
    glyph_cache: GlyphFallbackCache,
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

    fn ensure_layout(&mut self, lyrics: &SyncedLyrics, max_sub_w: usize) {
        let first_text = lyrics.lines.first().map(|l| l.text.as_str()).unwrap_or("");
        let has_cjk = lyrics
            .lines
            .iter()
            .any(|l| l.text.chars().any(is_cjk_or_hangul));
        let sub_h = if has_cjk { CJK_SUB_H } else { DEFAULT_SUB_H };
        let sub_line_rows = (sub_h / 2) as i32;

        if self.cached_line_count == lyrics.lines.len()
            && self.cached_max_sub_w == max_sub_w
            && self.cached_sub_h == sub_h
            && self.cached_first_text == first_text
            && !self.entries.is_empty()
        {
            return;
        }

        self.cached_line_count = lyrics.lines.len();
        self.cached_first_text = first_text.to_string();
        self.cached_wrap_chars = max_sub_w;
        self.cached_max_sub_w = max_sub_w;
        self.cached_sub_h = sub_h;
        self.sub_line_rows = sub_line_rows;
        self.entries.clear();
        self.entries.reserve(lyrics.lines.len());

        let mut blur_temp = vec![0.0_f32; max_sub_w * sub_h];
        let mut cursor_y = 0i32;

        for line in &lyrics.lines {
            let wrapped =
                wrap_lyric_text_subpixel(&line.text, max_sub_w, &mut self.glyph_cache, sub_h);
            let mut sub_lines = Vec::with_capacity(wrapped.len());
            for sub_text in &wrapped {
                let (sharp_grid, used_sub_w) = rasterize_line_subpixels_with_cache(
                    sub_text,
                    max_sub_w,
                    sub_h,
                    &mut self.glyph_cache,
                );
                let mut blurred_grid = sharp_grid.clone();
                apply_gaussian_blur_in_place(
                    &mut blurred_grid,
                    &mut blur_temp,
                    max_sub_w,
                    sub_h,
                    0.65,
                );
                sub_lines.push(CachedSubLine {
                    sharp_grid,
                    blurred_grid,
                    used_sub_w,
                });
            }
            let h = (sub_lines.len() as i32) * sub_line_rows;
            self.entries.push(CachedLyricEntry {
                virtual_y: cursor_y,
                sub_lines,
            });
            cursor_y += h + ENTRY_GAP_ROWS;
        }
    }
}

#[inline]
fn is_cjk_or_hangul(ch: char) -> bool {
    matches!(
        ch,
        '\u{3000}'..='\u{303F}' // CJK Symbols and Punctuation
            | '\u{3040}'..='\u{309F}' // Hiragana
            | '\u{30A0}'..='\u{30FF}' // Katakana
            | '\u{3130}'..='\u{318F}' // Hangul Compatibility Jamo
            | '\u{31F0}'..='\u{31FF}' // Katakana Phonetic Extensions
            | '\u{3400}'..='\u{4DBF}' // CJK Unified Ideographs Extension A
            | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
            | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
            | '\u{FF00}'..='\u{FFEF}' // Halfwidth and Fullwidth Forms
    )
}

/// Characters where line breaks can occur directly before/after without whitespace (Japanese/Chinese).
/// Korean Hangul uses spaces between words, so only non-Hangul CJK breaks between chars by default.
#[inline]
fn is_cjk_breakable_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3040}'..='\u{309F}' // Hiragana
            | '\u{30A0}'..='\u{30FF}' // Katakana
            | '\u{31F0}'..='\u{31FF}' // Katakana Phonetic Extensions
            | '\u{3400}'..='\u{4DBF}' // CJK Unified Ideographs Ext A
            | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
            | '\u{3000}'..='\u{303F}' // CJK Punctuation
            | '\u{FF01}'..='\u{FF60}' // Fullwidth forms
    )
}

#[inline]
fn is_no_line_start(ch: char) -> bool {
    matches!(
        ch,
        '、' | '。'
            | '，'
            | '．'
            | '！'
            | '？'
            | '・'
            | 'ー'
            | '〜'
            | '」'
            | '』'
            | '】'
            | '）'
            | '］'
            | '…'
            | '.'
            | ','
            | '!'
            | '?'
            | ')'
            | ']'
            | '»'
    )
}

#[inline]
fn is_no_line_end(ch: char) -> bool {
    matches!(ch, '「' | '『' | '【' | '（' | '［' | '(' | '[' | '«')
}

/// Tokenizes a lyric line into wrap units: whitespace-delimited words for alphabetic/Korean text,
/// and individual kinsoku-respecting clusters for Japanese/Chinese without spaces.
fn tokenize_wrap_units(text: &str) -> Vec<(String, bool)> {
    let mut units: Vec<(String, bool)> = Vec::new();
    let mut buf = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            if !buf.is_empty() {
                units.push((std::mem::take(&mut buf), false));
            }
            // Mark that whitespace preceded the next token
            if let Some(last) = units.last_mut() {
                last.1 = true;
            }
            continue;
        }

        if is_cjk_breakable_char(ch) {
            if !buf.is_empty() {
                if is_no_line_start(ch) {
                    buf.push(ch);
                    continue;
                }
                units.push((std::mem::take(&mut buf), false));
            }
            buf.push(ch);
            // Attach any trailing no-line-start punctuation (e.g. `。`, `、`, `」`)
            while let Some(&next_ch) = chars.peek() {
                if is_no_line_start(next_ch) && !next_ch.is_whitespace() {
                    buf.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            // If `ch` cannot end a line (e.g. `「`), merge the following character into this unit
            if is_no_line_end(ch) {
                if let Some(&next_ch) = chars.peek() {
                    if !next_ch.is_whitespace() {
                        buf.push(chars.next().unwrap());
                    }
                }
            }
            units.push((std::mem::take(&mut buf), false));
        } else {
            buf.push(ch);
        }
    }

    if !buf.is_empty() {
        units.push((buf, false));
    }

    units
}

/// Word-wraps a lyric line into sub-lines of at most `max_width` display columns
pub fn wrap_lyric_text(text: &str, max_width: usize) -> Vec<String> {
    let normalized: String = text.trim().nfc().collect();
    if normalized.is_empty() {
        return vec!["• • •".to_string()];
    }
    let limit = max_width.clamp(14, 42);
    let units = tokenize_wrap_units(&normalized);
    let mut wrapped = Vec::new();
    let mut current = String::new();
    let mut cur_w = 0usize;
    let mut pending_space = false;

    for (unit, trailing_space) in units {
        let unit_w: usize = unit.chars().map(|c| c.width().unwrap_or(1).max(1)).sum();

        if cur_w == 0 {
            current.push_str(&unit);
            cur_w = unit_w;
        } else {
            let space_w = usize::from(pending_space);
            if cur_w + space_w + unit_w <= limit {
                if pending_space {
                    current.push(' ');
                }
                current.push_str(&unit);
                cur_w += space_w + unit_w;
            } else {
                wrapped.push(current);
                current = unit;
                cur_w = unit_w;
            }
        }
        pending_space = trailing_space;
    }

    if !current.is_empty() {
        wrapped.push(current);
    }
    if wrapped.is_empty() {
        wrapped.push("• • •".to_string());
    }
    wrapped
}

fn measure_char_subpixel_advance(
    ch: char,
    sub_h: usize,
    glyph_cache: &mut GlyphFallbackCache,
) -> usize {
    if let Some((_, adv)) = glyph_bitmap_5x6(ch) {
        adv
    } else if let Some(g) = glyph_cache.get_or_rasterize(ch, sub_h) {
        g.advance
    } else {
        4
    }
}

fn wrap_lyric_text_subpixel(
    text: &str,
    max_sub_w: usize,
    glyph_cache: &mut GlyphFallbackCache,
    sub_h: usize,
) -> Vec<String> {
    let normalized: String = text.trim().nfc().collect();
    if normalized.is_empty() {
        return vec!["• • •".to_string()];
    }

    let usable_w = max_sub_w.saturating_sub(4).max(12);
    let space_adv = measure_char_subpixel_advance(' ', sub_h, glyph_cache) + 1;
    let units = tokenize_wrap_units(&normalized);

    let mut wrapped = Vec::new();
    let mut current = String::new();
    let mut cur_sub_w = 0usize;
    let mut pending_space = false;

    for (unit, trailing_space) in units {
        let unit_sub_w: usize = unit
            .chars()
            .map(|c| measure_char_subpixel_advance(c, sub_h, glyph_cache) + 1)
            .sum();

        if cur_sub_w == 0 {
            if unit_sub_w <= usable_w {
                current.push_str(&unit);
                cur_sub_w = unit_sub_w;
            } else {
                // Emergency split if a single long word exceeds the whole column width
                for ch in unit.chars() {
                    let ch_w = measure_char_subpixel_advance(ch, sub_h, glyph_cache) + 1;
                    if cur_sub_w > 0 && cur_sub_w + ch_w > usable_w {
                        wrapped.push(std::mem::take(&mut current));
                        cur_sub_w = 0;
                    }
                    current.push(ch);
                    cur_sub_w += ch_w;
                }
            }
        } else {
            let gap = if pending_space { space_adv } else { 0 };
            if cur_sub_w + gap + unit_sub_w <= usable_w {
                if pending_space {
                    current.push(' ');
                }
                current.push_str(&unit);
                cur_sub_w += gap + unit_sub_w;
            } else {
                wrapped.push(std::mem::take(&mut current));
                if unit_sub_w <= usable_w {
                    current.push_str(&unit);
                    cur_sub_w = unit_sub_w;
                } else {
                    cur_sub_w = 0;
                    for ch in unit.chars() {
                        let ch_w = measure_char_subpixel_advance(ch, sub_h, glyph_cache) + 1;
                        if cur_sub_w > 0 && cur_sub_w + ch_w > usable_w {
                            wrapped.push(std::mem::take(&mut current));
                            cur_sub_w = 0;
                        }
                        current.push(ch);
                        cur_sub_w += ch_w;
                    }
                }
            }
        }

        pending_space = trailing_space;
    }

    if !current.is_empty() {
        wrapped.push(current);
    }
    if wrapped.is_empty() {
        wrapped.push("• • •".to_string());
    }
    wrapped
}

/// Returns the 5x6 sub-pixel bitmap matrix (width 5, height 6) and horizontal sub-pixel
/// advance for supported alphabetic / punctuation characters, or `None` for scripts handled
/// by the dynamic fallback font rasterizer (Japanese, Korean, Chinese, etc.).
fn glyph_bitmap_5x6(ch: char) -> Option<([u8; 6], usize)> {
    let c = match ch {
        '’' | '‘' | '`' => '\'',
        '–' | '—' => '-',
        '“' | '”' | '„' => '"',
        'œ' => 'o',
        'Œ' => 'O',
        // Greek accented vowels -> base glyphs
        'Ά' | 'Ἀ' | 'Ἁ' => 'Α',
        'Έ' | 'Ἐ' | 'Ἑ' => 'Ε',
        'Ή' | 'Ἠ' | 'Ἡ' => 'Η',
        'Ί' | 'Ϊ' | 'Ἰ' | 'Ἱ' => 'Ι',
        'Ό' | 'Ὀ' | 'Ὁ' => 'Ο',
        'Ύ' | 'Ϋ' | 'Ὑ' => 'Υ',
        'Ώ' | 'Ω' | 'Ὠ' | 'Ὡ' => 'Ω',
        'ά' | 'ἀ' | 'ἁ' => 'α',
        'έ' | 'ἐ' | 'ἑ' => 'ε',
        'ή' | 'ἠ' | 'ἡ' => 'η',
        'ί' | 'ϊ' | 'ΐ' | 'ἰ' | 'ἱ' => 'ι',
        'ό' | 'ὀ' | 'ὁ' => 'ο',
        'ύ' | 'ϋ' | 'ΰ' | 'ὐ' | 'ὑ' => 'υ',
        'ώ' | 'ὠ' | 'ὡ' => 'ω',
        other => other,
    };

    let res = match c {
        ' ' => ([0, 0, 0, 0, 0, 0], 3),
        '.' => ([0, 0, 0, 0, 0b01100, 0b01100], 3),
        ',' => ([0, 0, 0, 0, 0b00110, 0b01100], 3),
        '\'' => ([0b01100, 0b01100, 0, 0, 0, 0], 3),
        '"' => ([0b11011, 0b11011, 0, 0, 0, 0], 5),
        '-' => ([0, 0, 0b11110, 0, 0, 0], 4),
        '!' => ([0b01100, 0b01100, 0b01100, 0, 0b01100, 0], 3),
        '¡' => ([0b01100, 0, 0b01100, 0b01100, 0b01100, 0], 3),
        '?' => ([0b01110, 0b10011, 0b00110, 0b00100, 0, 0b00100], 5),
        '¿' => ([0b00100, 0, 0b00100, 0b01100, 0b10011, 0b01110], 5),
        ':' => ([0, 0b01100, 0b01100, 0, 0b01100, 0b01100], 3),
        ';' => ([0, 0b01100, 0, 0b01100, 0b00110, 0b01100], 3),
        '/' => ([0b00011, 0b00110, 0b01100, 0b11000, 0b10000, 0], 5),
        '&' => ([0b01100, 0b10010, 0b01100, 0b10101, 0b01111, 0], 5),
        '+' => ([0, 0b00100, 0b01110, 0b00100, 0, 0], 4),
        '*' => ([0b10101, 0b01110, 0b10101, 0, 0, 0], 5),
        '~' => ([0, 0b01001, 0b10110, 0, 0, 0], 5),
        '=' => ([0, 0b11110, 0, 0b11110, 0, 0], 4),
        '•' => ([0, 0, 0b01100, 0b01100, 0, 0], 4),
        '…' => ([0, 0, 0, 0, 0b10101, 0], 5),
        '(' => ([0b00110, 0b01100, 0b01100, 0b01100, 0b00110, 0], 4),
        ')' => ([0b01100, 0b00110, 0b00110, 0b00110, 0b01100, 0], 4),
        '[' => ([0b01110, 0b01100, 0b01100, 0b01100, 0b01110, 0], 4),
        ']' => ([0b01110, 0b00110, 0b00110, 0b00110, 0b01110, 0], 4),
        '«' => ([0, 0b01010, 0b10100, 0b01010, 0, 0], 5),
        '»' => ([0, 0b10100, 0b01010, 0b10100, 0, 0], 5),

        // Latin Uppercase & Extended Latin
        'A' | 'À' | 'Â' | 'Á' | 'Ã' | 'Ä' | 'Å' | 'Ă' | 'Ą' => {
            ([0b01110, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5)
        }
        'Æ' => ([0b01111, 0b10100, 0b11110, 0b10100, 0b10111, 0], 5),
        'B' => ([0b11110, 0b11011, 0b11110, 0b11011, 0b11110, 0], 5),
        'C' | 'Ç' | 'Ć' | 'Č' => ([0b01111, 0b11000, 0b11000, 0b11000, 0b01111, 0], 5),
        'D' | 'Ď' => ([0b11110, 0b11011, 0b11011, 0b11011, 0b11110, 0], 5),
        'Đ' | 'Ð' => ([0b11110, 0b01011, 0b11111, 0b01011, 0b11110, 0], 5),
        'E' | 'É' | 'È' | 'Ê' | 'Ë' | 'Ę' | 'Ě' => {
            ([0b11111, 0b11000, 0b11110, 0b11000, 0b11111, 0], 5)
        }
        'F' => ([0b11111, 0b11000, 0b11110, 0b11000, 0b11000, 0], 5),
        'G' | 'Ğ' => ([0b01111, 0b11000, 0b11011, 0b11011, 0b01111, 0], 5),
        'H' => ([0b11011, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'I' | 'Î' | 'Ï' | 'Í' | 'Ì' | 'İ' => {
            ([0b01110, 0b00100, 0b00100, 0b00100, 0b01110, 0], 4)
        }
        'J' => ([0b00111, 0b00011, 0b00011, 0b11011, 0b01110, 0], 5),
        'K' => ([0b11011, 0b11110, 0b11100, 0b11110, 0b11011, 0], 5),
        'L' => ([0b11000, 0b11000, 0b11000, 0b11000, 0b11111, 0], 5),
        'Ł' => ([0b01000, 0b01000, 0b11100, 0b01000, 0b01111, 0], 5),
        'M' => ([0b10001, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'N' | 'Ń' | 'Ň' => ([0b11011, 0b11111, 0b11111, 0b11011, 0b11011, 0], 5),
        'Ñ' => ([0b01110, 0b11011, 0b11111, 0b11111, 0b11011, 0b11011], 5),
        'O' | 'Ô' | 'Ó' | 'Ò' | 'Õ' | 'Ö' => {
            ([0b01110, 0b11011, 0b11011, 0b11011, 0b01110, 0], 5)
        }
        'Ø' => ([0b01111, 0b11011, 0b11111, 0b11011, 0b11110, 0], 5),
        'P' => ([0b11110, 0b11011, 0b11110, 0b11000, 0b11000, 0], 5),
        'Q' => ([0b01110, 0b11011, 0b11011, 0b11111, 0b01111, 0b00011], 5),
        'R' | 'Ř' => ([0b11110, 0b11011, 0b11110, 0b11110, 0b11011, 0], 5),
        'S' | 'Ś' | 'Š' | 'Ş' | 'Ș' => ([0b01111, 0b11000, 0b01110, 0b00011, 0b11110, 0], 5),
        'T' | 'Ť' | 'Ţ' | 'Ț' => ([0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0], 5),
        'U' | 'Ù' | 'Û' | 'Ü' | 'Ú' | 'Ů' => {
            ([0b11011, 0b11011, 0b11011, 0b11011, 0b01110, 0], 5)
        }
        'V' => ([0b11011, 0b11011, 0b11011, 0b01110, 0b00100, 0], 5),
        'W' => ([0b11011, 0b11011, 0b11111, 0b11111, 0b10001, 0], 5),
        'X' => ([0b11011, 0b01110, 0b00100, 0b01110, 0b11011, 0], 5),
        'Y' | 'Ý' => ([0b11011, 0b01110, 0b00100, 0b00100, 0b00100, 0], 5),
        'Z' | 'Ź' | 'Ż' | 'Ž' => ([0b11111, 0b00110, 0b01100, 0b11000, 0b11111, 0], 5),

        // Latin Lowercase & Extended Latin Lowercase
        'a' => ([0, 0b01110, 0b00011, 0b01111, 0b11011, 0b01111], 5),
        'à' | 'â' | 'ã' | 'ă' => ([0b01100, 0b01110, 0b00011, 0b01111, 0b11011, 0b01111], 5),
        'á' => ([0b00110, 0b01110, 0b00011, 0b01111, 0b11011, 0b01111], 5),
        'ä' | 'å' => ([0b01010, 0b01110, 0b00011, 0b01111, 0b11011, 0b01111], 5),
        'ą' => ([0, 0b01110, 0b00011, 0b01111, 0b11011, 0b01111], 5),
        'æ' => ([0, 0b01110, 0b10111, 0b11100, 0b10111, 0], 5),
        'b' => ([0b11000, 0b11000, 0b11110, 0b11011, 0b11110, 0], 5),
        'c' => ([0, 0b01111, 0b11000, 0b11000, 0b01111, 0], 5),
        'ç' => ([0, 0b01111, 0b11000, 0b11000, 0b01111, 0b00100], 5),
        'ć' | 'č' => ([0b00110, 0b01111, 0b11000, 0b11000, 0b01111, 0], 5),
        'd' | 'ď' => ([0b00011, 0b00011, 0b01111, 0b11011, 0b01111, 0], 5),
        'đ' => ([0b00111, 0b00011, 0b01111, 0b11011, 0b01111, 0], 5),
        'e' => ([0, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'é' => ([0b00110, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'è' => ([0b01100, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'ê' | 'ë' | 'ě' | 'ę' => ([0b01110, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'f' => ([0b00111, 0b01100, 0b11110, 0b01100, 0b01100, 0], 4),
        'g' => ([0, 0b01111, 0b11011, 0b01111, 0b00011, 0b01110], 5),
        'ğ' => ([0b01110, 0b01111, 0b11011, 0b01111, 0b00011, 0b01110], 5),
        'h' => ([0b11000, 0b11000, 0b11110, 0b11011, 0b11011, 0], 5),
        'i' | 'í' | 'ì' => ([0b01100, 0, 0b01100, 0b01100, 0b01100, 0], 3),
        'ı' => ([0, 0, 0b01100, 0b01100, 0b01100, 0], 3),
        'î' | 'ï' => ([0b01110, 0, 0b01100, 0b01100, 0b01100, 0], 3),
        'j' => ([0b00110, 0, 0b00110, 0b00110, 0b10110, 0b01100], 4),
        'k' => ([0b11000, 0b11011, 0b11110, 0b11110, 0b11011, 0], 5),
        'l' => ([0b01100, 0b01100, 0b01100, 0b01100, 0b00110, 0], 3),
        'ł' => ([0b01100, 0b11100, 0b01100, 0b01100, 0b00110, 0], 4),
        'm' => ([0, 0b11010, 0b11111, 0b11111, 0b11011, 0], 5),
        'n' => ([0, 0b11110, 0b11011, 0b11011, 0b11011, 0], 5),
        'ñ' | 'ń' | 'ň' => ([0b01110, 0, 0b11110, 0b11011, 0b11011, 0b11011], 5),
        'o' => ([0, 0b01110, 0b11011, 0b11011, 0b01110, 0], 5),
        'ô' | 'ó' | 'ò' | 'õ' | 'ö' => ([0b01010, 0b01110, 0b11011, 0b11011, 0b01110, 0], 5),
        'ø' => ([0, 0b01111, 0b11011, 0b11111, 0b11110, 0], 5),
        'p' => ([0, 0b11110, 0b11011, 0b11110, 0b11000, 0b11000], 5),
        'q' => ([0, 0b01111, 0b11011, 0b01111, 0b00011, 0b00011], 5),
        'r' => ([0, 0b11110, 0b11000, 0b11000, 0b11000, 0], 4),
        'ř' => ([0b01010, 0b11110, 0b11000, 0b11000, 0b11000, 0], 4),
        's' => ([0, 0b01111, 0b11100, 0b00111, 0b11110, 0], 5),
        'ś' | 'š' => ([0b01010, 0b01111, 0b11100, 0b00111, 0b11110, 0], 5),
        'ş' | 'ș' => ([0, 0b01111, 0b11100, 0b00111, 0b11110, 0b00100], 5),
        'ß' => ([0b01110, 0b11011, 0b11110, 0b11011, 0b11110, 0b11000], 5),
        't' | 'ť' => ([0b01100, 0b11110, 0b01100, 0b01100, 0b00111, 0], 4),
        'ţ' | 'ț' => ([0b01100, 0b11110, 0b01100, 0b01100, 0b00111, 0b00100], 4),
        'u' => ([0, 0b11011, 0b11011, 0b11011, 0b01111, 0], 5),
        'ù' | 'û' | 'ü' | 'ú' | 'ů' => ([0b01010, 0b11011, 0b11011, 0b11011, 0b01111, 0], 5),
        'v' => ([0, 0b11011, 0b11011, 0b01110, 0b00100, 0], 5),
        'w' => ([0, 0b11011, 0b11111, 0b11111, 0b01010, 0], 5),
        'x' => ([0, 0b11011, 0b01110, 0b01110, 0b11011, 0], 5),
        'y' | 'ý' | 'ÿ' => ([0, 0b11011, 0b11011, 0b01111, 0b00011, 0b01110], 5),
        'z' => ([0, 0b11111, 0b00110, 0b01100, 0b11111, 0], 5),
        'ź' | 'ż' | 'ž' => ([0b01010, 0b11111, 0b00110, 0b01100, 0b11111, 0], 5),

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

        // Cyrillic Uppercase (Russian, Ukrainian, Belarusian, Serbian, Bulgarian)
        'А' => ([0b01110, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'Б' => ([0b11111, 0b11000, 0b11110, 0b11011, 0b11110, 0], 5),
        'В' => ([0b11110, 0b11011, 0b11110, 0b11011, 0b11110, 0], 5),
        'Г' => ([0b11111, 0b11000, 0b11000, 0b11000, 0b11000, 0], 5),
        'Ґ' => ([0b11111, 0b11001, 0b11000, 0b11000, 0b11000, 0], 5),
        'Д' => ([0b01110, 0b01010, 0b01010, 0b11111, 0b10001, 0], 5),
        'Е' => ([0b11111, 0b11000, 0b11110, 0b11000, 0b11111, 0], 5),
        'Ё' => ([0b01010, 0b11111, 0b11000, 0b11110, 0b11000, 0b11111], 5),
        'Є' => ([0b01111, 0b11000, 0b11110, 0b11000, 0b01111, 0], 5),
        'Ж' => ([0b10101, 0b10101, 0b01110, 0b10101, 0b10101, 0], 5),
        'З' => ([0b11110, 0b00011, 0b01110, 0b00011, 0b11110, 0], 5),
        'И' => ([0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0], 5),
        'Й' => ([0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001], 5),
        'І' => ([0b01110, 0b00100, 0b00100, 0b00100, 0b01110, 0], 4),
        'Ї' => ([0b01010, 0b01110, 0b00100, 0b00100, 0b00100, 0b01110], 4),
        'К' => ([0b11011, 0b11110, 0b11100, 0b11110, 0b11011, 0], 5),
        'Л' => ([0b01111, 0b01001, 0b01001, 0b01001, 0b11001, 0], 5),
        'М' => ([0b10001, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'Н' => ([0b11011, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'О' => ([0b01110, 0b11011, 0b11011, 0b11011, 0b01110, 0], 5),
        'П' => ([0b11111, 0b11011, 0b11011, 0b11011, 0b11011, 0], 5),
        'Р' => ([0b11110, 0b11011, 0b11110, 0b11000, 0b11000, 0], 5),
        'С' => ([0b01111, 0b11000, 0b11000, 0b11000, 0b01111, 0], 5),
        'Т' => ([0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0], 5),
        'У' => ([0b11011, 0b11011, 0b01111, 0b00011, 0b11110, 0], 5),
        'Ў' => ([0b01110, 0b11011, 0b11011, 0b01111, 0b00011, 0b11110], 5),
        'Ф' => ([0b00100, 0b01110, 0b10101, 0b10101, 0b01110, 0b00100], 5),
        'Х' => ([0b11011, 0b01110, 0b00100, 0b01110, 0b11011, 0], 5),
        'Ц' => ([0b10010, 0b10010, 0b10010, 0b10010, 0b11111, 0b00001], 5),
        'Ч' => ([0b11011, 0b11011, 0b01111, 0b00011, 0b00011, 0], 5),
        'Ш' => ([0b10101, 0b10101, 0b10101, 0b10101, 0b11111, 0], 5),
        'Щ' => ([0b10101, 0b10101, 0b10101, 0b10101, 0b11111, 0b00001], 5),
        'Ъ' => ([0b11000, 0b01000, 0b01110, 0b01011, 0b01110, 0], 5),
        'Ы' => ([0b10001, 0b10001, 0b11101, 0b10101, 0b11101, 0], 5),
        'Ь' => ([0b11000, 0b11000, 0b11110, 0b11011, 0b11110, 0], 5),
        'Э' => ([0b01110, 0b00011, 0b01111, 0b00011, 0b01110, 0], 5),
        'Ю' => ([0b10010, 0b10101, 0b11101, 0b10101, 0b10010, 0], 5),
        'Я' => ([0b01111, 0b11011, 0b01111, 0b01011, 0b11011, 0], 5),
        'Ј' => ([0b00111, 0b00011, 0b00011, 0b11011, 0b01110, 0], 5),
        'Ђ' | 'Ћ' => ([0b11110, 0b01000, 0b01110, 0b01011, 0b01011, 0], 5),
        'Љ' => ([0b01110, 0b01010, 0b01011, 0b11010, 0b11011, 0], 5),
        'Њ' => ([0b10100, 0b11110, 0b10101, 0b10110, 0b10100, 0], 5),
        'Џ' => ([0b10001, 0b10001, 0b10001, 0b11111, 0b00100, 0], 5),

        // Cyrillic Lowercase
        'а' => ([0, 0b01110, 0b00011, 0b01111, 0b11011, 0b01111], 5),
        'б' => ([0b00111, 0b01000, 0b11110, 0b11011, 0b01110, 0], 5),
        'в' => ([0, 0b11110, 0b11100, 0b11011, 0b11110, 0], 5),
        'г' => ([0, 0b11111, 0b11000, 0b11000, 0b11000, 0], 5),
        'ґ' => ([0b00001, 0b11111, 0b11000, 0b11000, 0b11000, 0], 5),
        'д' => ([0, 0b01110, 0b01010, 0b01010, 0b11111, 0b10001], 5),
        'е' => ([0, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'ё' => ([0b01010, 0b01110, 0b11011, 0b11111, 0b11000, 0b01111], 5),
        'є' => ([0, 0b01111, 0b11000, 0b11110, 0b01111, 0], 5),
        'ж' => ([0, 0b10101, 0b01110, 0b01110, 0b10101, 0], 5),
        'з' => ([0, 0b11110, 0b00110, 0b00011, 0b11110, 0], 5),
        'и' => ([0, 0b10011, 0b10101, 0b11001, 0b10001, 0], 5),
        'й' => ([0b01110, 0b10011, 0b10101, 0b11001, 0b10001, 0], 5),
        'і' => ([0b01100, 0, 0b01100, 0b01100, 0b01100, 0], 3),
        'ї' => ([0b10100, 0, 0b01100, 0b01100, 0b01100, 0], 4),
        'к' => ([0, 0b11011, 0b11110, 0b11100, 0b11011, 0], 5),
        'л' => ([0, 0b01111, 0b01001, 0b01001, 0b11001, 0], 5),
        'м' => ([0, 0b10001, 0b11011, 0b10101, 0b10001, 0], 5),
        'н' => ([0, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'о' => ([0, 0b01110, 0b11011, 0b11011, 0b01110, 0], 5),
        'п' => ([0, 0b11111, 0b11011, 0b11011, 0b11011, 0], 5),
        'р' => ([0, 0b11110, 0b11011, 0b11110, 0b11000, 0b11000], 5),
        'с' => ([0, 0b01111, 0b11000, 0b11000, 0b01111, 0], 5),
        'т' => ([0, 0b11111, 0b00100, 0b00100, 0b00100, 0], 5),
        'у' => ([0, 0b11011, 0b11011, 0b01111, 0b00011, 0b11110], 5),
        'ў' => ([0b01110, 0b11011, 0b11011, 0b01111, 0b00011, 0b11110], 5),
        'ф' => ([0, 0b01110, 0b10101, 0b01110, 0b00100, 0b00100], 5),
        'х' => ([0, 0b11011, 0b01110, 0b01110, 0b11011, 0], 5),
        'ц' => ([0, 0b10010, 0b10010, 0b10010, 0b11111, 0b00001], 5),
        'ч' => ([0, 0b11011, 0b01111, 0b00011, 0b00011, 0], 5),
        'ш' => ([0, 0b10101, 0b10101, 0b10101, 0b11111, 0], 5),
        'щ' => ([0, 0b10101, 0b10101, 0b10101, 0b11111, 0b00001], 5),
        'ъ' => ([0, 0b11000, 0b01110, 0b01011, 0b01110, 0], 5),
        'ы' => ([0, 0b10001, 0b11101, 0b10101, 0b11101, 0], 5),
        'ь' => ([0, 0b11000, 0b11110, 0b11011, 0b11110, 0], 5),
        'э' => ([0, 0b01110, 0b00111, 0b00011, 0b01110, 0], 5),
        'ю' => ([0, 0b10010, 0b11101, 0b10101, 0b10010, 0], 5),
        'я' => ([0, 0b01111, 0b11011, 0b01111, 0b11011, 0], 5),
        'ј' => ([0b00110, 0, 0b00110, 0b00110, 0b10110, 0b01100], 4),
        'ђ' | 'ћ' => ([0b11100, 0b01000, 0b01110, 0b01011, 0b01011, 0], 5),
        'љ' => ([0, 0b01110, 0b01010, 0b11011, 0b11010, 0], 5),
        'њ' => ([0, 0b10100, 0b11110, 0b10101, 0b10110, 0], 5),
        'џ' => ([0, 0b10001, 0b10001, 0b11111, 0b00100, 0], 5),

        // Greek Uppercase
        'Α' => ([0b01110, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'Β' => ([0b11110, 0b11011, 0b11110, 0b11011, 0b11110, 0], 5),
        'Γ' => ([0b11111, 0b11000, 0b11000, 0b11000, 0b11000, 0], 5),
        'Δ' => ([0b00100, 0b01010, 0b10001, 0b10001, 0b11111, 0], 5),
        'Ε' => ([0b11111, 0b11000, 0b11110, 0b11000, 0b11111, 0], 5),
        'Ζ' => ([0b11111, 0b00110, 0b01100, 0b11000, 0b11111, 0], 5),
        'Η' => ([0b11011, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'Θ' => ([0b01110, 0b10001, 0b11111, 0b10001, 0b01110, 0], 5),
        'Ι' => ([0b01110, 0b00100, 0b00100, 0b00100, 0b01110, 0], 4),
        'Κ' => ([0b11011, 0b11110, 0b11100, 0b11110, 0b11011, 0], 5),
        'Λ' => ([0b00100, 0b01010, 0b11011, 0b11011, 0b11011, 0], 5),
        'Μ' => ([0b10001, 0b11011, 0b11111, 0b11011, 0b11011, 0], 5),
        'Ν' => ([0b11011, 0b11111, 0b11111, 0b11011, 0b11011, 0], 5),
        'Ξ' => ([0b11111, 0, 0b01110, 0, 0b11111, 0], 5),
        'Ο' => ([0b01110, 0b11011, 0b11011, 0b11011, 0b01110, 0], 5),
        'Π' => ([0b11111, 0b11011, 0b11011, 0b11011, 0b11011, 0], 5),
        'Ρ' => ([0b11110, 0b11011, 0b11110, 0b11000, 0b11000, 0], 5),
        'Σ' => ([0b11111, 0b01100, 0b00110, 0b01100, 0b11111, 0], 5),
        'Τ' => ([0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0], 5),
        'Υ' => ([0b11011, 0b01110, 0b00100, 0b00100, 0b00100, 0], 5),
        'Φ' => ([0b00100, 0b01110, 0b10101, 0b10101, 0b01110, 0b00100], 5),
        'Χ' => ([0b11011, 0b01110, 0b00100, 0b01110, 0b11011, 0], 5),
        'Ψ' => ([0b10101, 0b10101, 0b01110, 0b00100, 0b00100, 0], 5),
        'Ω' => ([0b01110, 0b11011, 0b10001, 0b01010, 0b11011, 0], 5),

        // Greek Lowercase
        'α' => ([0, 0b01101, 0b10010, 0b10010, 0b01101, 0], 5),
        'β' => ([0b01110, 0b10011, 0b11110, 0b10011, 0b11110, 0b10000], 5),
        'γ' => ([0, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100], 5),
        'δ' => ([0b01110, 0b01000, 0b01110, 0b10011, 0b01110, 0], 5),
        'ε' => ([0, 0b01111, 0b11000, 0b01110, 0b01111, 0], 5),
        'ζ' => ([0b01111, 0b00100, 0b01100, 0b01000, 0b00110, 0b00010], 5),
        'η' => ([0, 0b11110, 0b11011, 0b11011, 0b11011, 0b00011], 5),
        'θ' => ([0b01110, 0b10001, 0b11111, 0b10001, 0b01110, 0], 5),
        'ι' => ([0, 0b01100, 0b01100, 0b01100, 0b00110, 0], 3),
        'κ' => ([0, 0b11011, 0b11110, 0b11110, 0b11011, 0], 5),
        'λ' => ([0b01000, 0b00100, 0b01110, 0b10010, 0b10001, 0], 5),
        'μ' => ([0, 0b11011, 0b11011, 0b11011, 0b11110, 0b10000], 5),
        'ν' => ([0, 0b11011, 0b11011, 0b01110, 0b00100, 0], 5),
        'ξ' => ([0b01111, 0b01000, 0b01110, 0b01000, 0b00110, 0b00010], 5),
        'ο' => ([0, 0b01110, 0b11011, 0b11011, 0b01110, 0], 5),
        'π' => ([0, 0b11111, 0b01010, 0b01010, 0b01011, 0], 5),
        'ρ' => ([0, 0b01110, 0b11011, 0b11110, 0b11000, 0b11000], 5),
        'σ' => ([0, 0b01111, 0b11010, 0b11010, 0b01100, 0], 5),
        'ς' => ([0, 0b01111, 0b11000, 0b01110, 0b00011, 0b00110], 5),
        'τ' => ([0, 0b11111, 0b00100, 0b00100, 0b00110, 0], 5),
        'υ' => ([0, 0b11011, 0b11011, 0b11011, 0b01110, 0], 5),
        'φ' => ([0, 0b01110, 0b10101, 0b01110, 0b00100, 0b00100], 5),
        'χ' => ([0, 0b11011, 0b01110, 0b01110, 0b11011, 0b10001], 5),
        'ψ' => ([0, 0b10101, 0b10101, 0b01110, 0b00100, 0b00100], 5),
        'ω' => ([0, 0b10001, 0b10101, 0b10101, 0b01010, 0], 5),

        _ => return None,
    };

    Some(res)
}

/// Rasterizes a text line into a `(max_sub_w, sub_h)` sub-pixel opacity grid
fn rasterize_line_subpixels_with_cache(
    text: &str,
    max_sub_w: usize,
    sub_h: usize,
    glyph_cache: &mut GlyphFallbackCache,
) -> (Vec<f32>, usize) {
    let mut cursor_x = 2usize;
    let mut grid = vec![0.0_f32; max_sub_w * sub_h];
    let bitmap_y_offset = (sub_h.saturating_sub(6)) / 2;

    for ch in text.chars() {
        if let Some((rows, advance)) = glyph_bitmap_5x6(ch) {
            if cursor_x + advance + 2 >= max_sub_w {
                break;
            }
            for (ry, &bits) in rows.iter().enumerate() {
                let gy = ry + bitmap_y_offset;
                if gy >= sub_h {
                    continue;
                }
                for rx in 0..5usize {
                    if (bits >> (4 - rx)) & 1 == 1 {
                        let gx = cursor_x + rx;
                        if gx < max_sub_w {
                            grid[gy * max_sub_w + gx] = 1.0;
                        }
                    }
                }
            }
            cursor_x += advance + 1;
        } else if let Some(g) = glyph_cache.get_or_rasterize(ch, sub_h) {
            if cursor_x + g.advance + 2 >= max_sub_w {
                break;
            }
            for ry in 0..g.height {
                let gy_i = g.top_y + (ry as isize);
                if gy_i < 0 || gy_i >= sub_h as isize {
                    continue;
                }
                let gy = gy_i as usize;
                for rx in 0..g.width {
                    let gx_i = (cursor_x as isize) + g.left_x + (rx as isize);
                    if gx_i < 0 || gx_i >= max_sub_w as isize {
                        continue;
                    }
                    let a = g.alpha[ry * g.width + rx];
                    if a > 0.01 {
                        let idx = gy * max_sub_w + (gx_i as usize);
                        grid[idx] = grid[idx].max(a);
                    }
                }
            }
            cursor_x += g.advance + 1;
        } else {
            // Ultimate fallback glyph if no system font supplies `ch`
            let fallback_rows = [0u8, 0b01110, 0b01110, 0b01110, 0, 0];
            let advance = 4;
            if cursor_x + advance + 2 >= max_sub_w {
                break;
            }
            for (ry, &bits) in fallback_rows.iter().enumerate() {
                let gy = ry + bitmap_y_offset;
                if gy >= sub_h {
                    continue;
                }
                for rx in 0..5usize {
                    if (bits >> (4 - rx)) & 1 == 1 {
                        let gx = cursor_x + rx;
                        if gx < max_sub_w {
                            grid[gy * max_sub_w + gx] = 1.0;
                        }
                    }
                }
            }
            cursor_x += advance + 1;
        }
    }

    let used_w = (cursor_x + 2).min(max_sub_w);
    (grid, used_w)
}

#[cfg(test)]
fn rasterize_line_subpixels(text: &str, max_sub_w: usize) -> (Vec<f32>, usize) {
    let mut cache = GlyphFallbackCache::default();
    rasterize_line_subpixels_with_cache(text, max_sub_w, DEFAULT_SUB_H, &mut cache)
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
                let down = if y + 1 < h {
                    temp[(y + 1) * w + x]
                } else {
                    0.0
                };
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
    let max_sub_w = avail_cols * 2;

    scroll.ensure_layout(lyrics, max_sub_w);
    let sub_line_rows = scroll.sub_line_rows.max(4);

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
                (entry_virtual_y + (sub_i as i32 * sub_line_rows) as f32) - smooth_virtual_y;
            let top_screen_y_i32 = (anchor_screen_y as i32) + (rel_rows.round() as i32);

            if top_screen_y_i32 + sub_line_rows < (area.y as i32 + 1)
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
            let base_ok = if is_active {
                white_ok
            } else {
                inactive_base_ok
            };

            let grid_slice: &[f32] = if is_active {
                &sub_line.sharp_grid
            } else {
                &sub_line.blurred_grid
            };

            let used_cells_w = sub_line.used_sub_w.div_ceil(2);
            let start_x = area.x + 2;

            for cell_row in 0..sub_line_rows {
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
    let active_idx = params
        .lyrics
        .active_line_index(params.position)
        .unwrap_or(0);
    let smooth_center = scroll.update(active_idx as f32, params.dt);
    let center_y = area.y + area.height / 2;
    let white_ok = rgb_to_oklab(255, 255, 255);
    let inactive_ok = rgb_to_oklab(175, 172, 180);
    let max_cols = area.width.saturating_sub(3) as usize;

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

        let (shown, _) = fit_text_to_cols(&line.text, max_cols);
        let start_x = area.x + 2;
        let bg_ok = params.bg.oklab_at(start_x, sy);
        let (r, g, b) = oklab_to_rgb(lerp_oklab(base_ok, bg_ok, eff_fade));
        let mut style = Style::default().fg(Color::Rgb(r, g, b));
        if is_active {
            style = style.add_modifier(Modifier::BOLD);
        }
        buf.set_stringn(start_x, sy, &shown, max_cols, style);
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
    fn test_wrap_lyric_text_handles_cjk_without_spaces_and_kinsoku() {
        let line = "夢ならばどれほどよかったでしょう。まだあなたのことを夢にみる";
        let wrapped = wrap_lyric_text(line, 20);
        assert!(
            wrapped.len() >= 2,
            "Japanese line should wrap into multiple lines"
        );
        for sub in &wrapped {
            assert!(
                !sub.starts_with('。') && !sub.starts_with('、'),
                "Line should not begin with forbidden punctuation: {sub}"
            );
        }
    }

    #[test]
    fn test_subpixel_rasterizer_and_gaussian_blur() {
        let (mut grid, used_w) = rasterize_line_subpixels("Midnight", 120);
        let mut temp = vec![0.0_f32; 120 * DEFAULT_SUB_H];
        assert!(used_w > 20);
        let peak_before = grid.iter().copied().fold(0.0_f32, f32::max);
        assert_eq!(peak_before, 1.0);

        apply_gaussian_blur_in_place(&mut grid, &mut temp, 120, DEFAULT_SUB_H, 0.8);
        let peak_after = grid.iter().copied().fold(0.0_f32, f32::max);
        assert!(peak_after > 0.2 && peak_after < 0.95);
    }

    #[test]
    fn test_cyrillic_extended_latin_and_greek_use_native_bitmaps() {
        for sample in [
            "Привет, мир! Ёлка и жизнь",
            "Año ñoño Straße Łódź Ğüşİı Øresund",
            "Μουσική και ψυχή Ωμέγα",
        ] {
            for ch in sample.chars() {
                assert!(
                    glyph_bitmap_5x6(ch).is_some(),
                    "Missing native 5x6 bitmap for '{ch}'"
                );
            }
            let (grid, used_w) = rasterize_line_subpixels(sample, 240);
            assert!(used_w > 20);
            assert!(grid.iter().copied().any(|v| v > 0.5));
        }
    }

    #[test]
    fn test_cjk_and_hangul_rasterization_and_adaptive_height() {
        let lyrics = SyncedLyrics::new(
            "Test".to_string(),
            vec![crate::media::lyrics::LyricLine {
                start: Duration::ZERO,
                end: Duration::from_secs(4),
                text: "夜に駆ける 사랑해 你好".to_string(),
                words: Vec::new(),
                is_chorus: false,
            }],
        );
        let mut scroll = LyricsScrollState::default();
        scroll.ensure_layout(&lyrics, 120);
        assert_eq!(scroll.cached_sub_h, CJK_SUB_H);
        assert_eq!(scroll.sub_line_rows, 5);
        assert!(!scroll.entries[0].sub_lines.is_empty());
        if crate::ui::glyph_fallback::ensure_cjk_test_font_or_skip() {
            let peak = scroll.entries[0].sub_lines[0]
                .sharp_grid
                .iter()
                .copied()
                .fold(0.0_f32, f32::max);
            assert!(
                peak > 0.5,
                "CJK/Hangul line should rasterize visible pixels"
            );
        }
    }
}
