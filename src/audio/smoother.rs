use crate::audio::fft::NUM_BANDS;

/// Filtre de lissage temporel exponentiel asymétrique (144 FPS) + Spline de Catmull-Rom
pub struct SpectrumSmoother {
    current: [f32; NUM_BANDS],
    secondary: [f32; NUM_BANDS],
    attack_lambda: f32,
    decay_lambda: f32,
    phase: f32,
}

impl SpectrumSmoother {
    pub fn new(attack_lambda: f32, decay_lambda: f32) -> Self {
        Self {
            current: [0.0; NUM_BANDS],
            secondary: [0.0; NUM_BANDS],
            attack_lambda: attack_lambda.max(1.0),
            decay_lambda: decay_lambda.max(1.0),
            phase: 0.0,
        }
    }

    /// Met à jour les bandes principales et secondaires à 144 FPS.
    /// Si `is_playing` est vrai mais que le haut-parleur / lecteur est en mode Muet (`🔇`),
    /// génère une onde musicale multi-harmoniques vivante comme dans Sonora (Image 3).
    pub fn step(&mut self, targets: &[f32; NUM_BANDS], dt: f32, is_playing: bool) {
        let dt = dt.clamp(0.0005, 0.1);
        let speed = if is_playing { 2.8 } else { 0.7 };
        self.phase = (self.phase + dt * speed) % (std::f32::consts::TAU * 100.0);

        let max_fft = targets.iter().copied().fold(0.0_f32, f32::max);
        let audio_is_silent = max_fft < 0.03;

        let attack_factor = 1.0 - (-self.attack_lambda * dt).exp();
        let decay_factor = 1.0 - (-self.decay_lambda * dt).exp();
        let sec_factor = 1.0 - (-(self.decay_lambda * 0.75) * dt).exp();
        let sec_wave_amp = if is_playing { 0.18 } else { 0.05 };
        let inv_max_band = 1.0 / ((NUM_BANDS - 1) as f32);
        let t = self.phase;

        for (i, &target_band) in targets.iter().enumerate().take(NUM_BANDS) {
            let x = (i as f32) * inv_max_band;

            // Enveloppe multi-collines organique inspirée de l'onde de Sonora (Image 3)
            let hill1 = (x * 16.0 - t * 1.3).sin() * 0.5 + 0.5;
            let hill2 = (x * 29.0 + t * 1.9).sin() * 0.5 + 0.5;
            let hill3 = (x * 9.5 + t * 0.75).cos() * 0.5 + 0.5;
            let pulse = (t * 2.1 + (x * 6.0).sin()).sin() * 0.5 + 0.5;

            let synthetic_wave =
                (hill1 * 0.42 + hill2 * 0.33 + hill3 * 0.25) * (0.45 + 0.45 * pulse);

            let target = if is_playing {
                if audio_is_silent {
                    // Musique en lecture mais sortie audio coupée (Mute 🔇) :
                    // animer l'onde avec de vraies collines dynamiques (15% à 82% de hauteur)
                    (synthetic_wave * 0.82 + 0.08).clamp(0.08, 0.88)
                } else {
                    // Vrai signal FFT PipeWire combiné à une légère ondulation de base
                    (target_band * 0.88 + synthetic_wave * 0.16).clamp(0.06, 0.96)
                }
            } else {
                // En pause : très douce vague basse au repos
                (synthetic_wave * 0.18 + 0.04).clamp(0.03, 0.22)
            };

            let prev = self.current[i];
            let factor = if target > prev {
                attack_factor
            } else {
                decay_factor
            };
            let next_val = (prev + (target - prev) * factor).clamp(0.0, 1.0);
            self.current[i] = next_val;

            // Seconde onde déphasée (écho sous-jacent visible sur l'Image 3 de Sonora)
            let sec_target = (next_val * 0.76
                + ((x * 21.0 + t * 1.1).sin() * 0.5 + 0.5) * sec_wave_amp)
                .clamp(0.02, 0.85);
            self.secondary[i] =
                (self.secondary[i] + (sec_target - self.secondary[i]) * sec_factor).clamp(0.0, 1.0);
        }
    }

    pub fn bands(&self) -> &[f32; NUM_BANDS] {
        &self.current
    }

    pub fn sample_catmull_rom(&self, sub_columns: usize, out: &mut Vec<f32>) {
        evaluate_catmull_rom_spline(&self.current, sub_columns, out);
    }

    pub fn sample_secondary_catmull_rom(&self, sub_columns: usize, out: &mut Vec<f32>) {
        evaluate_catmull_rom_spline(&self.secondary, sub_columns, out);
    }
}

/// Interpolation par spline de Catmull-Rom sur une grille de taille `out_len`
pub fn evaluate_catmull_rom_spline(points: &[f32], out_len: usize, out: &mut Vec<f32>) {
    out.clear();
    if out_len == 0 || points.is_empty() {
        return;
    }
    out.reserve(out_len);

    if points.len() == 1 || out_len == 1 {
        out.resize(out_len, points[0].clamp(0.0, 1.0));
        return;
    }

    let n = points.len();
    let max_idx = (n - 1) as f32;

    for col in 0..out_len {
        let u = (col as f32) / ((out_len - 1) as f32) * max_idx;
        let idx = (u.floor() as usize).min(n - 2);
        let t = u - (idx as f32);

        let p0 = if idx == 0 { points[0] } else { points[idx - 1] };
        let p1 = points[idx];
        let p2 = points[idx + 1];
        let p3 = if idx + 2 < n {
            points[idx + 2]
        } else {
            points[n - 1]
        };

        let t2 = t * t;
        let t3 = t2 * t;

        let val = 0.5
            * ((2.0 * p1)
                + (-p0 + p2) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);

        out.push(val.clamp(0.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_smoother_attack_faster_than_decay() {
        let mut smoother = SpectrumSmoother::new(35.0, 12.0);
        let dt = 1.0 / 144.0;
        let ones = [1.0_f32; NUM_BANDS];
        smoother.step(&ones, dt, true);
        let after_one_attack_step = smoother.bands()[20];
        assert!(after_one_attack_step > 0.18);

        let zeros = [0.05_f32; NUM_BANDS];
        let before_decay = smoother.bands()[20];
        smoother.step(&zeros, dt, true);
        let after_decay = smoother.bands()[20];
        let attack_delta = after_one_attack_step;
        let decay_delta = (before_decay - after_decay).abs();
        assert!(attack_delta > decay_delta);
    }

    #[test]
    fn test_catmull_rom_spline_continuity_and_bounds() {
        let control = [0.0, 0.25, 0.8, 0.4, 0.1, 0.0];
        let mut out = Vec::new();
        evaluate_catmull_rom_spline(&control, 120, &mut out);
        assert_eq!(out.len(), 120);
        assert!((out[0] - 0.0).abs() < 1e-4);
        assert!((out[119] - 0.0).abs() < 1e-4);
        assert!(out.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }
}
