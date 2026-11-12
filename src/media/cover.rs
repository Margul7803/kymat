use crate::ui::background::ThemePalette;
use color_thief::{ColorFormat, get_palette};
use image::{DynamicImage, RgbaImage, imageops::FilterType};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct CoverArtData {
    pub art_url: String,
    /// Image RGBA source décodée (256x256 pour un échantillonnage quadrant 2x2 ultra-précis)
    pub image: Arc<RgbaImage>,
    /// Palette Oklab extraite de la pochette
    pub palette: ThemePalette,
}

/// Décode une URL `file://` ou télécharge une URL `http(s)://` et extrait la palette Oklab.
pub async fn load_cover_and_palette(art_url: &str) -> Option<CoverArtData> {
    if art_url.trim().is_empty() {
        return None;
    }

    if let Some(file_path) = art_url.strip_prefix("file://") {
        let decoded_path = urlencoding::decode(file_path)
            .map(|s| PathBuf::from(s.into_owned()))
            .unwrap_or_else(|_| PathBuf::from(file_path));

        for attempt in 0..10 {
            if let Ok(bytes) = fs::read(&decoded_path) {
                if !bytes.is_empty() {
                    if let Ok(img) = image::load_from_memory(&bytes) {
                        return Some(process_dynamic_image(art_url, &img));
                    }
                }
            }
            if attempt < 9 {
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            }
        }
        return None;
    }

    let bytes = if art_url.starts_with("http://") || art_url.starts_with("https://") {
        let resp = reqwest::get(art_url).await.ok()?;
        resp.bytes().await.ok()?.to_vec()
    } else {
        fs::read(art_url).ok()?
    };

    let img = image::load_from_memory(&bytes).ok()?;
    Some(process_dynamic_image(art_url, &img))
}

pub fn process_dynamic_image(art_url: &str, img: &DynamicImage) -> CoverArtData {
    let thumb = img.resize_exact(256, 256, FilterType::Lanczos3);
    let rgb_img = thumb.to_rgb8();

    // Filtrer les pixels quasi-blancs ou quasi-noirs pour que color-thief extraie les vraies
    // teintes chromatiques de la pochette (ex: le rose corail et le vert d'eau sur Alizée)
    let mut filtered_rgb: Vec<u8> = Vec::with_capacity(256 * 256 * 3);
    for p in rgb_img.pixels() {
        let [r, g, b] = p.0;
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let sat = max_c.saturating_sub(min_c);
        if max_c > 24 && !(max_c > 225 && sat < 25) {
            filtered_rgb.extend_from_slice(&[r, g, b]);
        }
    }

    let sample_buf = if filtered_rgb.len() >= 300 {
        &filtered_rgb[..]
    } else {
        rgb_img.as_raw()
    };

    let dominant_rgb: Vec<(u8, u8, u8)> = match get_palette(sample_buf, ColorFormat::Rgb, 5, 8) {
        Ok(colors) => colors.into_iter().map(|c| (c.r, c.g, c.b)).collect(),
        Err(_) => vec![(235, 105, 135), (15, 75, 85), (40, 20, 35)],
    };

    let palette = ThemePalette::from_dominant_rgb(&dominant_rgb);
    CoverArtData {
        art_url: art_url.to_string(),
        image: Arc::new(thumb.to_rgba8()),
        palette,
    }
}

/// Worker asynchrone qui surveille les changements d'`art_url` et met en cache les pochettes décodées
pub fn spawn_cover_worker(
    mut url_rx: watch::Receiver<String>,
) -> watch::Receiver<Option<CoverArtData>> {
    let (tx, rx) = watch::channel(None);

    tokio::spawn(async move {
        let mut cache: HashMap<String, CoverArtData> = HashMap::new();

        loop {
            let current_url = url_rx.borrow_and_update().clone();
            if current_url.is_empty() {
                let _ = tx.send(None);
            } else if let Some(cached) = cache.get(&current_url) {
                let _ = tx.send(Some(cached.clone()));
            } else {
                match load_cover_and_palette(&current_url).await {
                    Some(loaded) => {
                        if cache.len() > 24 {
                            cache.clear();
                        }
                        cache.insert(current_url, loaded.clone());
                        let _ = tx.send(Some(loaded));
                    }
                    None => {
                        let _ = tx.send(None);
                    }
                }
            }

            if url_rx.changed().await.is_err() {
                break;
            }
        }
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn test_process_dynamic_image_extracts_palette() {
        let mut img = RgbImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let color = if x < 32 {
                    Rgb([235, 105, 135])
                } else {
                    Rgb([20, 90, 100])
                };
                img.put_pixel(x, y, color);
            }
        }
        let dyn_img = DynamicImage::ImageRgb8(img);
        let data = process_dynamic_image("file:///tmp/test.jpg", &dyn_img);
        assert_eq!(data.image.dimensions(), (256, 256));
        assert!(data.palette.accent.l >= 0.65);
    }
}
