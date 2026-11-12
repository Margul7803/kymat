use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::f32::consts::PI;
use std::sync::Arc;

pub const FFT_SIZE: usize = 2048;
pub const HOP_SIZE: usize = 512;
pub const SAMPLE_RATE: f32 = 44_100.0;
pub const NUM_BANDS: usize = 48;

pub struct FftAnalyzer {
    ring_buf: Vec<f32>,
    write_pos: usize,
    hann_window: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    band_edges: Vec<(usize, usize)>,
    band_weights: Vec<f32>,
    /// Volume-independent spectral peak follower
    rolling_peak: f32,
}

impl FftAnalyzer {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        let hann_window: Vec<f32> = (0..FFT_SIZE)
            .map(|n| 0.5 * (1.0 - (2.0 * PI * (n as f32) / ((FFT_SIZE - 1) as f32)).cos()))
            .collect();

        // Dense musical frequency range (45 Hz to 8,500 Hz) so the full screen width stays active
        let min_freq = 45.0_f32;
        let max_freq = 8_500.0_f32;
        let bin_hz = SAMPLE_RATE / (FFT_SIZE as f32);
        let max_bin = (FFT_SIZE / 2) - 1;

        let mut band_edges = Vec::with_capacity(NUM_BANDS);
        let mut band_weights = Vec::with_capacity(NUM_BANDS);

        for i in 0..NUM_BANDS {
            let t0 = (i as f32) / (NUM_BANDS as f32);
            let t1 = ((i + 1) as f32) / (NUM_BANDS as f32);
            let f0 = min_freq * (max_freq / min_freq).powf(t0);
            let f1 = min_freq * (max_freq / min_freq).powf(t1);

            let b0 = ((f0 / bin_hz).floor() as usize).clamp(1, max_bin);
            let b1 = ((f1 / bin_hz).ceil() as usize).clamp(b0 + 1, max_bin + 1);
            band_edges.push((b0, b1));

            // Psychoacoustic tilt boosting mid and high frequencies
            let center_f = (f0 * f1).sqrt();
            let tilt = (center_f / 180.0).powf(0.42).clamp(0.70, 3.8);
            band_weights.push(tilt);
        }

        Self {
            ring_buf: vec![0.0; FFT_SIZE],
            write_pos: 0,
            hann_window,
            fft,
            scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            band_edges,
            band_weights,
            rolling_peak: 0.25,
        }
    }

    pub fn push_samples(&mut self, samples: &[f32], sensitivity: f32) -> [f32; NUM_BANDS] {
        for &s in samples {
            let clean = if s.is_finite() { s } else { 0.0 };
            self.ring_buf[self.write_pos] = clean;
            self.write_pos = (self.write_pos + 1) % FFT_SIZE;
        }

        // Pre-FFT window peak normalization: cancels out system volume scalar G completely
        // so that 1% volume and 100% volume produce the exact same normalized waveform into the FFT.
        let mut window_peak = 0.0_f32;
        for &s in &self.ring_buf {
            window_peak = window_peak.max(s.abs());
        }

        // True digital silence (< -110 dB) returns zero
        if window_peak < 3.0e-6 {
            return [0.0; NUM_BANDS];
        }

        let inv_window_peak = 1.0 / window_peak;
        for i in 0..FFT_SIZE {
            let sample_idx = (self.write_pos + i) % FFT_SIZE;
            let normalized = self.ring_buf[sample_idx] * inv_window_peak;
            let windowed = normalized * self.hann_window[i];
            self.scratch[i] = Complex::new(windowed, 0.0);
        }

        self.fft.process(&mut self.scratch);

        let mut raw_bands = [0.0_f32; NUM_BANDS];
        let norm_factor = 2.0 / (FFT_SIZE as f32);
        let mut frame_max = 0.0_f32;

        for (band_idx, &(b0, b1)) in self.band_edges.iter().enumerate() {
            let mut max_norm_sqr = 0.0_f32;
            for bin in b0..b1 {
                let c = self.scratch[bin];
                let norm_sqr = c.re * c.re + c.im * c.im;
                max_norm_sqr = max_norm_sqr.max(norm_sqr);
            }
            let max_bin_mag = max_norm_sqr.sqrt() * norm_factor;
            let weighted = max_bin_mag * self.band_weights[band_idx] * sensitivity;
            raw_bands[band_idx] = weighted;
            frame_max = frame_max.max(weighted);
        }

        if frame_max > self.rolling_peak {
            self.rolling_peak = self.rolling_peak * 0.65 + frame_max * 0.35;
        } else {
            self.rolling_peak = (self.rolling_peak * 0.96 + frame_max * 0.04).max(1e-4);
        }

        let mut bands = [0.0_f32; NUM_BANDS];
        let ref_peak = self.rolling_peak.max(frame_max * 0.85).max(1e-4);
        for i in 0..NUM_BANDS {
            let rel = (raw_bands[i] / ref_peak).clamp(0.0, 1.0);
            bands[i] = rel.powf(0.65);
        }

        // 5-tap spatial smoothing (1-2-3-2-1) to form wide organic hills
        let mut smoothed = bands;
        for i in 2..(NUM_BANDS - 2) {
            smoothed[i] = bands[i - 2] * 0.12
                + bands[i - 1] * 0.22
                + bands[i] * 0.32
                + bands[i + 1] * 0.22
                + bands[i + 2] * 0.12;
        }
        smoothed
    }
}

impl Default for FftAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fft_analyzer_silence_and_sine_wave() {
        let mut analyzer = FftAnalyzer::new();
        let silence = vec![0.0_f32; FFT_SIZE];
        let out_silent = analyzer.push_samples(&silence, 1.0);
        assert!(out_silent.iter().all(|&v| v == 0.0));

        let sine: Vec<f32> = (0..FFT_SIZE)
            .map(|n| (2.0 * PI * 440.0 * (n as f32) / SAMPLE_RATE).sin() * 0.5)
            .collect();
        let out_sine = analyzer.push_samples(&sine, 1.2);
        let max_band = out_sine.iter().copied().fold(0.0_f32, f32::max);
        assert!(
            max_band > 0.3,
            "Expected clear peak on 440Hz sine, got {max_band}"
        );
    }

    #[test]
    fn test_fft_volume_invariance_low_vs_high_volume() {
        let mut analyzer_low = FftAnalyzer::new();
        let mut analyzer_high = FftAnalyzer::new();

        // Same musical signal at 0.5% volume (0.005) vs 95% volume (0.95)
        let low_vol: Vec<f32> = (0..FFT_SIZE)
            .map(|n| {
                let t = (n as f32) / SAMPLE_RATE;
                ((2.0 * PI * 120.0 * t).sin() + 0.6 * (2.0 * PI * 880.0 * t).sin()) * 0.005
            })
            .collect();
        let high_vol: Vec<f32> = (0..FFT_SIZE)
            .map(|n| {
                let t = (n as f32) / SAMPLE_RATE;
                ((2.0 * PI * 120.0 * t).sin() + 0.6 * (2.0 * PI * 880.0 * t).sin()) * 0.95
            })
            .collect();

        let bands_low = analyzer_low.push_samples(&low_vol, 1.15);
        let bands_high = analyzer_high.push_samples(&high_vol, 1.15);

        for i in 0..NUM_BANDS {
            assert!(
                (bands_low[i] - bands_high[i]).abs() < 1e-5,
                "Band {i} differed between low and high volume: {} vs {}",
                bands_low[i],
                bands_high[i]
            );
        }
    }
}
