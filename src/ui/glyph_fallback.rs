use fontdue::{Font, FontSettings};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug)]
pub struct RasterizedGlyph {
    pub width: usize,
    pub height: usize,
    pub top_y: isize,
    pub left_x: isize,
    pub advance: usize,
    pub alpha: Vec<f32>,
}

struct SharedFontPool {
    specs: Option<Vec<(PathBuf, u32)>>,
    next_spec_idx: usize,
    fonts: Vec<Font>,
    bytes_cache: HashMap<PathBuf, Vec<u8>>,
}

impl SharedFontPool {
    fn new() -> Self {
        Self {
            specs: None,
            next_spec_idx: 0,
            fonts: Vec::new(),
            bytes_cache: HashMap::new(),
        }
    }

    fn ensure_specs(&mut self) {
        if self.specs.is_none() {
            self.specs = Some(discover_fallback_font_specs());
        }
    }

    /// Loads only as many system fonts as needed until `ch` is found.
    fn rasterize_with_fallback(&mut self, ch: char, px_size: f32) -> Option<RasterizedGlyph> {
        self.ensure_specs();

        let existing_idx = self
            .fonts
            .iter()
            .position(|f| f.lookup_glyph_index(ch) != 0);

        let font_idx = match existing_idx {
            Some(i) => i,
            None => {
                let mut loaded_match = None;
                let total_specs = self.specs.as_ref().map(|s| s.len()).unwrap_or(0);
                while self.next_spec_idx < total_specs {
                    let (path, col_idx) = self.specs.as_ref().unwrap()[self.next_spec_idx].clone();
                    self.next_spec_idx += 1;

                    let bytes = match self.bytes_cache.get(&path) {
                        Some(b) => b,
                        None => match std::fs::read(&path) {
                            Ok(b) => {
                                self.bytes_cache.insert(path.clone(), b);
                                self.bytes_cache.get(&path).expect("just inserted")
                            }
                            Err(_) => continue,
                        },
                    };

                    let settings = FontSettings {
                        collection_index: col_idx,
                        scale: 40.0,
                        ..FontSettings::default()
                    };
                    if let Ok(font) = Font::from_bytes(bytes.as_slice(), settings) {
                        let has_ch = font.lookup_glyph_index(ch) != 0;
                        self.fonts.push(font);
                        if has_ch {
                            loaded_match = Some(self.fonts.len() - 1);
                            break;
                        }
                    }
                }
                loaded_match?
            }
        };

        let font = &self.fonts[font_idx];
        let sub_h_usize = px_size.round().max(1.0) as usize;
        // CJK & general glyphs occupy ~82% of the em-square height, so scale slightly above sub_h
        let raster_px = (sub_h_usize as f32) * 1.14;
        let (metrics, bitmap) = font.rasterize(ch, raster_px);
        if metrics.width == 0 || metrics.height == 0 {
            let adv = (metrics.advance_width.round() as usize).max(3);
            return Some(RasterizedGlyph {
                width: 0,
                height: 0,
                top_y: 0,
                left_x: 0,
                advance: adv,
                alpha: Vec::new(),
            });
        }

        // Fixed baseline at ~84% of sub_h keeps punctuation (。、ー) aligned while tall Kanji fill 0..sub_h
        let baseline_y = ((sub_h_usize as f32) * 0.84).round() as isize;
        let mut top_y = baseline_y - (metrics.ymin as isize) - (metrics.height as isize);
        if top_y < 0 {
            top_y = 0;
        } else if top_y + (metrics.height as isize) > sub_h_usize as isize {
            top_y = (sub_h_usize as isize - metrics.height as isize).max(0);
        }

        // Gentle normalization: preserve open counter-spaces in dense Kanji while keeping strokes crisp
        let peak = bitmap.iter().copied().max().unwrap_or(0) as f32 / 255.0;
        let boost = if peak > 0.15 && peak < 0.90 {
            0.90 / peak
        } else {
            1.0
        };
        let alpha: Vec<f32> = bitmap
            .into_iter()
            .map(|v| ((v as f32 / 255.0) * boost).clamp(0.0, 1.0))
            .collect();

        let left_x = metrics.xmin.max(0) as isize;
        let min_adv = metrics.width + left_x as usize;
        let advance = (metrics.advance_width.ceil() as usize).max(min_adv).max(4);

        Some(RasterizedGlyph {
            width: metrics.width,
            height: metrics.height,
            top_y,
            left_x,
            advance,
            alpha,
        })
    }
}

fn shared_font_pool() -> &'static Mutex<SharedFontPool> {
    static POOL: OnceLock<Mutex<SharedFontPool>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(SharedFontPool::new()))
}

/// Pre-loads the primary CJK/Unicode fallback font on a background thread so the first
/// non-Latin track never causes a frame hitch at 144 FPS.
pub fn warm_up_in_background() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_ok() {
        std::thread::spawn(|| {
            if let Ok(mut pool) = shared_font_pool().lock() {
                pool.rasterize_with_fallback('あ', 10.0);
            }
        });
    }
}

/// Per-layout fallback glyph cache backed by a process-wide lazy font pool.
/// Only invoked at track layout time (`ensure_layout`), never in the 144 FPS loop.
pub struct GlyphFallbackCache {
    cache: HashMap<char, Option<RasterizedGlyph>>,
    sub_h: usize,
}

impl Default for GlyphFallbackCache {
    fn default() -> Self {
        Self {
            cache: HashMap::new(),
            sub_h: 8,
        }
    }
}

impl GlyphFallbackCache {
    pub fn set_sub_h(&mut self, sub_h: usize) {
        if self.sub_h != sub_h {
            self.sub_h = sub_h;
            self.cache.clear();
        }
    }

    pub fn get_or_rasterize(&mut self, ch: char, sub_h: usize) -> Option<&RasterizedGlyph> {
        self.set_sub_h(sub_h);
        self.cache
            .entry(ch)
            .or_insert_with(|| {
                if ch.is_whitespace() {
                    Some(RasterizedGlyph {
                        width: 0,
                        height: 0,
                        top_y: 0,
                        left_x: 0,
                        advance: (sub_h / 2).max(3),
                        alpha: Vec::new(),
                    })
                } else {
                    let px_size = sub_h as f32;
                    let mut pool = shared_font_pool().lock().expect("font pool lock");
                    pool.rasterize_with_fallback(ch, px_size)
                }
            })
            .as_ref()
    }

    pub fn measure_advance(&mut self, ch: char, sub_h: usize) -> Option<usize> {
        self.get_or_rasterize(ch, sub_h).map(|g| g.advance)
    }
}

fn discover_fallback_font_specs() -> Vec<(PathBuf, u32)> {
    let mut specs: Vec<(PathBuf, u32)> = Vec::new();
    let mut seen: HashSet<(PathBuf, u32)> = HashSet::new();

    let mut push_spec = |path: PathBuf, idx: u32| {
        if path.is_file() && seen.insert((path.clone(), idx)) {
            specs.push((path, idx));
        }
    };

    // 1. Query fontconfig: prefer Regular Noto CJK JP (covers JP + most SC/TC/KR in one face)
    for query in [
        ":lang=ja:style=Regular",
        ":lang=ja",
        ":lang=ko:style=Regular",
        ":lang=zh:style=Regular",
        "sans-serif:style=Regular",
    ] {
        if let Some(found) = query_fc_list(query) {
            for (p, idx) in found.into_iter().take(1) {
                push_spec(p, idx);
            }
        }
    }

    // 2. Direct Linux / Arch / Omarchy fallback paths
    for (raw_path, idx) in [
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 1),
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 2),
        ("/usr/share/fonts/noto/NotoSans-Regular.ttf", 0),
        ("/usr/share/fonts/TTF/DejaVuSans.ttf", 0),
        ("/usr/share/fonts/liberation/LiberationSans-Regular.ttf", 0),
    ] {
        push_spec(PathBuf::from(raw_path), idx);
    }

    specs
}

fn query_fc_list(pattern: &str) -> Option<Vec<(PathBuf, u32)>> {
    let output = Command::new("fc-list")
        .args([pattern, "-f", "%{file}\t%{index}\n"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    for line in stdout.lines() {
        let Some((file_str, idx_str)) = line.split_once('\t') else {
            continue;
        };
        let path = PathBuf::from(file_str.trim());
        let index = idx_str.trim().parse::<u32>().unwrap_or(0);
        results.push((path, index));
    }
    Some(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cjk_fallback_rasterization() {
        let mut cache = GlyphFallbackCache::default();
        for ch in ['夜', 'に', '駆', '愛', '夢', '사', '你'] {
            let glyph = cache
                .get_or_rasterize(ch, 10)
                .unwrap_or_else(|| panic!("Should rasterize '{ch}' via system CJK font"));
            assert!(glyph.width > 0 && glyph.height > 0);
            assert!(glyph.alpha.iter().copied().fold(0.0_f32, f32::max) > 0.3);
        }
    }
}
