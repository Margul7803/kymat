use std::time::{Duration, Instant};

/// Smooth transition animation between View 1 (Centered Cover + Waveform, `progress = 0.0`)
/// and View 2 (Split View Synchronized Lyrics, `progress = 1.0`).
pub struct LayoutTransition {
    from_val: f32,
    target_val: f32,
    current_val: f32,
    start_instant: Option<Instant>,
    duration: Duration,
}

impl LayoutTransition {
    pub fn new(duration_secs: f32) -> Self {
        Self {
            from_val: 0.0,
            target_val: 0.0,
            current_val: 0.0,
            start_instant: None,
            duration: Duration::from_secs_f32(duration_secs.max(0.05)),
        }
    }

    pub fn set_split_active(&mut self, active: bool, now: Instant) {
        let desired = if active { 1.0 } else { 0.0 };
        if (self.target_val - desired).abs() < f32::EPSILON {
            return;
        }
        self.from_val = self.current_val;
        self.target_val = desired;
        self.start_instant = Some(now);
    }

    pub fn update(&mut self, now: Instant) -> f32 {
        if let Some(start) = self.start_instant {
            let t = (now.saturating_duration_since(start).as_secs_f32()
                / self.duration.as_secs_f32())
            .clamp(0.0, 1.0);
            if t >= 1.0 {
                self.current_val = self.target_val;
                self.start_instant = None;
            } else {
                let eased = cubic_bezier_ease_in_out(t);
                self.current_val = self.from_val + (self.target_val - self.from_val) * eased;
            }
        }
        self.current_val
    }

    pub fn value(&self) -> f32 {
        self.current_val
    }
}

/// Cubic-Bezier ease-in-out curve
#[inline]
pub fn cubic_bezier_ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cubic_bezier_bounds_and_midpoint() {
        assert!((cubic_bezier_ease_in_out(0.0) - 0.0).abs() < 1e-6);
        assert!((cubic_bezier_ease_in_out(0.5) - 0.5).abs() < 1e-6);
        assert!((cubic_bezier_ease_in_out(1.0) - 1.0).abs() < 1e-6);
    }
}
