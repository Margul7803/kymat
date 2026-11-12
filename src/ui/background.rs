use palette::{FromColor, Oklab, Srgb};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::time::{Duration, Instant};

/// Matrice de Bayer 4x4 très subtile pour éviter le color banding sans jamais créer de trame visible
const BAYER_4X4: [[f32; 4]; 4] = [
    [-0.46875, 0.03125, -0.34375, 0.15625],
    [0.28125, -0.21875, 0.40625, -0.09375],
    [-0.28125, 0.21875, -0.40625, 0.09375],
    [0.46875, -0.03125, 0.34375, -0.15625],
];

/// Palette extraite d'une pochette d'album dans l'espace perceptuel Oklab,
/// calibrée pour reproduire fidèlement le fond très sombre à halos radiaux de Sonora.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemePalette {
    /// Couleur de fond de base quasi-noire (#050507) sur tous les bords
    pub base_dark: Oklab,
    /// Halo radial principal (derrière la cover / haut-centre)
    pub primary_glow: Oklab,
    /// Halo radial secondaire (centre-bas / droite)
    pub secondary_glow: Oklab,
    /// Couleur d'accent vive (pour le cœur ♥, la crête de l'onde et la barre de temps)
    pub accent: Oklab,
    /// Seconde couleur d'accent pour le dégradé sous l'onde sonore
    pub secondary_accent: Oklab,
}

impl Default for ThemePalette {
    fn default() -> Self {
        let base_dark = rgb_to_oklab(5, 5, 8);
        let primary_glow = rgb_to_oklab(36, 12, 20);
        let secondary_glow = rgb_to_oklab(10, 28, 32);
        let accent = rgb_to_oklab(235, 105, 135);
        let secondary_accent = rgb_to_oklab(95, 45, 70);
        Self {
            base_dark,
            primary_glow,
            secondary_glow,
            accent,
            secondary_accent,
        }
    }
}

impl ThemePalette {
    /// Construit une palette sombre style Sonora à partir des couleurs dominantes de la cover
    pub fn from_dominant_rgb(colors: &[(u8, u8, u8)]) -> Self {
        if colors.is_empty() {
            return Self::default();
        }

        // Trier les couleurs par saturation (chroma) en ignorant le blanc/gris pur
        let mut oklab_candidates: Vec<(Oklab, f32)> = colors
            .iter()
            .copied()
            .map(|(r, g, b)| {
                let ok = rgb_to_oklab(r, g, b);
                let chroma = (ok.a * ok.a + ok.b * ok.b).sqrt();
                (ok, chroma)
            })
            .collect();

        oklab_candidates.sort_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });

        let c_primary = oklab_candidates[0].0;
        // Choisir une seconde couleur suffisamment distincte de la première si possible
        let c_secondary = oklab_candidates
            .iter()
            .skip(1)
            .find(|(ok, _)| {
                let da = ok.a - c_primary.a;
                let db = ok.b - c_primary.b;
                (da * da + db * db).sqrt() > 0.04
            })
            .map(|(ok, _)| *ok)
            .unwrap_or_else(|| {
                if oklab_candidates.len() > 1 {
                    oklab_candidates[1].0
                } else {
                    c_primary
                }
            });

        // Base très sombre (#050507) légèrement teintée
        let base_dark = Oklab::new(
            0.095,
            (c_primary.a * 0.12).clamp(-0.015, 0.015),
            (c_primary.b * 0.12).clamp(-0.015, 0.015),
        );

        // Halos radiaux sombres (L autour de 0.21-0.24 au cœur du halo, jamais criards)
        let primary_glow = Oklab::new(
            0.235,
            (c_primary.a * 0.85).clamp(-0.09, 0.09),
            (c_primary.b * 0.85).clamp(-0.09, 0.09),
        );

        let secondary_glow = Oklab::new(
            0.205,
            (c_secondary.a * 0.75).clamp(-0.08, 0.08),
            (c_secondary.b * 0.75).clamp(-0.08, 0.08),
        );

        // Couleur d'accent douce et lumineuse (comme le rose corail de Moi... Lolita)
        let mut accent = c_primary;
        accent.l = accent.l.clamp(0.68, 0.78);
        accent.a = (accent.a * 1.15).clamp(-0.18, 0.18);
        accent.b = (accent.b * 1.15).clamp(-0.18, 0.18);

        let mut secondary_accent = c_primary;
        secondary_accent.l = 0.36;
        secondary_accent.a = (c_primary.a * 0.90).clamp(-0.12, 0.12);
        secondary_accent.b = (c_primary.b * 0.90).clamp(-0.12, 0.12);

        Self {
            base_dark,
            primary_glow,
            secondary_glow,
            accent,
            secondary_accent,
        }
    }

    pub fn lerp(&self, target: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        Self {
            base_dark: lerp_oklab(self.base_dark, target.base_dark, s),
            primary_glow: lerp_oklab(self.primary_glow, target.primary_glow, s),
            secondary_glow: lerp_oklab(self.secondary_glow, target.secondary_glow, s),
            accent: lerp_oklab(self.accent, target.accent, s),
            secondary_accent: lerp_oklab(self.secondary_accent, target.secondary_accent, s),
        }
    }
}

pub fn rgb_to_oklab(r: u8, g: u8, b: u8) -> Oklab {
    let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    Oklab::from_color(srgb.into_linear())
}

pub fn oklab_to_rgb(c: Oklab) -> (u8, u8, u8) {
    let srgb: Srgb<f32> = Srgb::from_linear(palette::LinSrgb::<f32>::from_color(c));
    let r = (srgb.red.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (srgb.green.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (srgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8;
    (r, g, b)
}

#[inline]
pub fn lerp_oklab(a: Oklab, b: Oklab, t: f32) -> Oklab {
    let t = t.clamp(0.0, 1.0);
    Oklab::new(
        a.l + (b.l - a.l) * t,
        a.a + (b.a - a.a) * t,
        a.b + (b.b - a.b) * t,
    )
}

/// Gestionnaire du fond "Ambient Glow" radial style Sonora avec cache statique 144 FPS
pub struct AmbientBackground {
    from_palette: ThemePalette,
    target_palette: ThemePalette,
    current_palette: ThemePalette,
    transition_start: Option<Instant>,
    transition_duration: Duration,
    last_cache_regen: Instant,
    cached_width: u16,
    cached_height: u16,
    cached_split_bucket: u8,
    cached_rgb: Vec<(u8, u8, u8)>,
    cached_oklab: Vec<Oklab>,
}

impl AmbientBackground {
    pub fn new(transition_secs: f32) -> Self {
        let default_palette = ThemePalette::default();
        Self {
            from_palette: default_palette,
            target_palette: default_palette,
            current_palette: default_palette,
            transition_start: None,
            transition_duration: Duration::from_secs_f32(transition_secs.max(0.1)),
            last_cache_regen: Instant::now(),
            cached_width: 0,
            cached_height: 0,
            cached_split_bucket: 255,
            cached_rgb: Vec::new(),
            cached_oklab: Vec::new(),
        }
    }

    pub fn set_target_palette(&mut self, new_palette: ThemePalette) {
        if self.target_palette == new_palette {
            return;
        }
        self.from_palette = self.current_palette;
        self.target_palette = new_palette;
        self.transition_start = Some(Instant::now());
    }

    pub fn current_palette(&self) -> &ThemePalette {
        &self.current_palette
    }

    pub fn update_and_ensure_cache(
        &mut self,
        width: u16,
        height: u16,
        split_t: f32,
        now: Instant,
    ) {
        let split_bucket = (split_t.clamp(0.0, 1.0) * 10.0).round() as u8;
        let size_changed = width != self.cached_width
            || height != self.cached_height
            || split_bucket != self.cached_split_bucket;
        let mut need_regen = size_changed;

        if let Some(start) = self.transition_start {
            let elapsed = now.saturating_duration_since(start);
            let t = (elapsed.as_secs_f32() / self.transition_duration.as_secs_f32()).clamp(0.0, 1.0);
            if t >= 1.0 {
                self.current_palette = self.target_palette;
                self.transition_start = None;
                need_regen = true;
            } else if now.saturating_duration_since(self.last_cache_regen)
                >= Duration::from_millis(33)
                || size_changed
            {
                self.current_palette = self.from_palette.lerp(&self.target_palette, t);
                self.last_cache_regen = now;
                need_regen = true;
            }
        }

        if need_regen {
            self.cached_split_bucket = split_bucket;
            self.rebuild_grid(width, height, split_t);
        }
    }

    /// Génère un fond très sombre (#050507 sur tous les bords) avec 2 halos radiaux diffus
    /// comme dans Sonora (Images 1 & 3), au lieu d'un dégradé 4-coins trop lumineux.
    fn rebuild_grid(&mut self, width: u16, height: u16, split_t: f32) {
        self.cached_width = width;
        self.cached_height = height;
        let total = (width as usize) * (height as usize);
        self.cached_rgb.clear();
        self.cached_rgb.reserve(total);
        self.cached_oklab.clear();
        self.cached_oklab.reserve(total);

        let w_denom = (width.saturating_sub(1)).max(1) as f32;
        let h_denom = (height.saturating_sub(1)).max(1) as f32;

        // En Vue 1 (centrée, split_t = 0), le halo principal est au centre-haut (0.48, 0.34)
        // et le halo secondaire en bas-gauche/centre (0.36, 0.65).
        // En Vue 2 (paroles, split_t = 1), un halo doux éclaire la droite derrière la ligne active (0.68, 0.35)
        // et le second halo doux est en bas au centre (0.42, 0.72).
        let p1_x = 0.46 + 0.22 * split_t;
        let p1_y = 0.33;
        let p2_x = 0.38;
        let p2_y = 0.68;

        for y in 0..height {
            let v = (y as f32) / h_denom;
            for x in 0..width {
                let u = (x as f32) / w_denom;

                // Distance radiale normalisée au halo 1 (gaussienne large)
                let dx1 = (u - p1_x) * 1.35;
                let dy1 = (v - p1_y) * 1.55;
                let w1 = (-3.2 * (dx1 * dx1 + dy1 * dy1)).exp() * 0.78;

                // Distance radiale au halo 2
                let dx2 = (u - p2_x) * 1.45;
                let dy2 = (v - p2_y) * 1.65;
                let w2 = (-3.6 * (dx2 * dx2 + dy2 * dy2)).exp() * 0.68;

                // Assombrissement supplémentaire vers les bords extrêmes de l'écran (vignette noire)
                let edge_dx = (u - 0.5) * 2.0;
                let edge_dy = (v - 0.5) * 2.0;
                let edge_dist = (edge_dx * edge_dx + edge_dy * edge_dy).min(1.4);
                let edge_vignette = (1.0 - edge_dist * 0.42).clamp(0.35, 1.0);

                let mut cell_ok = self.current_palette.base_dark;
                cell_ok = lerp_oklab(
                    cell_ok,
                    self.current_palette.primary_glow,
                    (w1 * edge_vignette).clamp(0.0, 0.85),
                );
                cell_ok = lerp_oklab(
                    cell_ok,
                    self.current_palette.secondary_glow,
                    (w2 * edge_vignette).clamp(0.0, 0.75),
                );

                self.cached_oklab.push(cell_ok);

                let (r, g, b) = oklab_to_rgb(cell_ok);
                // Tramage ultra-léger (+/- 0.7) invisible à l'œil nu mais lissant les transitions
                let dither = BAYER_4X4[(y as usize) & 3][(x as usize) & 3] * 1.2;
                let dr = (r as f32 + dither).clamp(0.0, 255.0) as u8;
                let dg = (g as f32 + dither).clamp(0.0, 255.0) as u8;
                let db = (b as f32 + dither).clamp(0.0, 255.0) as u8;
                self.cached_rgb.push((dr, dg, db));
            }
        }
    }

    #[inline]
    pub fn rgb_at(&self, x: u16, y: u16) -> (u8, u8, u8) {
        if x < self.cached_width && y < self.cached_height {
            let idx = (y as usize) * (self.cached_width as usize) + (x as usize);
            self.cached_rgb[idx]
        } else {
            (5, 5, 8)
        }
    }

    #[inline]
    pub fn oklab_at(&self, x: u16, y: u16) -> Oklab {
        if x < self.cached_width && y < self.cached_height {
            let idx = (y as usize) * (self.cached_width as usize) + (x as usize);
            self.cached_oklab[idx]
        } else {
            self.current_palette.base_dark
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let max_x = area.right().min(self.cached_width);
        let max_y = area.bottom().min(self.cached_height);

        for y in area.y..max_y {
            let row_offset = (y as usize) * (self.cached_width as usize);
            for x in area.x..max_x {
                let (r, g, b) = self.cached_rgb[row_offset + (x as usize)];
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(Color::Rgb(r, g, b));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oklab_roundtrip_and_lerp() {
        let ok1 = rgb_to_oklab(220, 50, 120);
        let ok2 = rgb_to_oklab(30, 180, 240);
        let mid = lerp_oklab(ok1, ok2, 0.5);
        assert!(!mid.l.is_nan());
        let (r, g, b) = oklab_to_rgb(mid);
        assert!(r > 0 && g > 0 && b > 0);
    }

    #[test]
    fn test_static_cache_invariance_outside_transition() {
        let mut bg = AmbientBackground::new(1.5);
        let t0 = Instant::now();
        bg.update_and_ensure_cache(100, 40, 0.0, t0);
        let snapshot1 = bg.cached_rgb.clone();

        bg.update_and_ensure_cache(100, 40, 0.0, t0 + Duration::from_micros(6944));
        assert_eq!(snapshot1, bg.cached_rgb);
    }

    #[test]
    fn test_palette_darkening_ensures_deep_dark_background() {
        let p = ThemePalette::from_dominant_rgb(&[(255, 255, 255), (240, 100, 140)]);
        assert!(p.base_dark.l <= 0.12);
        assert!(p.primary_glow.l <= 0.25);
        assert!(p.secondary_glow.l <= 0.25);
    }
}
