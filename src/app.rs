use crate::audio::fft::NUM_BANDS;
use crate::audio::smoother::SpectrumSmoother;
use crate::config::AppConfig;
use crate::media::cover::{CoverArtData, spawn_cover_worker};
use crate::media::lyrics::{SyncedLyrics, spawn_lyrics_worker};
use crate::media::mpris_watcher::{MprisState, PlaybackStatus, TrackMetadata, spawn_mpris_watcher};
use crate::ui::background::{AmbientBackground, ThemePalette, oklab_to_rgb};
use crate::ui::cover_widget::{
    CoverMetaView, ScaledCoverCache, draw_centered_text, render_cover_and_meta,
    render_passive_time_bar,
};
use crate::ui::lyrics_widget::{LyricsRenderParams, LyricsScrollState, render_synced_lyrics};
use crate::ui::transitions::LayoutTransition;
use crate::ui::wave_widget::render_braille_wave;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

pub struct App {
    pub config: AppConfig,
    pub debug_fps: bool,
    mpris_rx: watch::Receiver<MprisState>,
    audio_rx: watch::Receiver<[f32; NUM_BANDS]>,
    cover_rx: watch::Receiver<Option<CoverArtData>>,
    lyrics_rx: watch::Receiver<Option<SyncedLyrics>>,
    art_url_tx: watch::Sender<String>,
    meta_tx: watch::Sender<TrackMetadata>,

    current_mpris: MprisState,
    last_track_key: String,
    last_art_url: String,
    current_cover: Option<CoverArtData>,
    current_lyrics: Option<SyncedLyrics>,

    bg: AmbientBackground,
    cover_cache: ScaledCoverCache,
    smoother: SpectrumSmoother,
    wave_sub_cols: Vec<f32>,
    wave_sec_sub_cols: Vec<f32>,
    layout_transition: LayoutTransition,
    lyrics_scroll: LyricsScrollState,

    frame_times: [f32; 60],
    frame_idx: usize,
    measured_fps: f32,
}

impl App {
    pub fn new(
        config: AppConfig,
        debug_fps: bool,
        audio_rx: watch::Receiver<[f32; NUM_BANDS]>,
    ) -> Self {
        let mpris_rx = spawn_mpris_watcher();
        let (art_url_tx, art_url_rx) = watch::channel(String::new());
        let (meta_tx, meta_rx) = watch::channel(TrackMetadata::default());

        let cover_rx = spawn_cover_worker(art_url_rx);
        let lyrics_rx = spawn_lyrics_worker(meta_rx);

        let bg = AmbientBackground::new(config.background_transition_secs);
        let smoother = SpectrumSmoother::new(config.attack_lambda, config.decay_lambda);
        let layout_transition = LayoutTransition::new(config.layout_transition_secs);
        let current_mpris = mpris_rx.borrow().clone();

        Self {
            config,
            debug_fps,
            mpris_rx,
            audio_rx,
            cover_rx,
            lyrics_rx,
            art_url_tx,
            meta_tx,
            current_mpris,
            last_track_key: String::new(),
            last_art_url: String::new(),
            current_cover: None,
            current_lyrics: None,
            bg,
            cover_cache: ScaledCoverCache::default(),
            smoother,
            wave_sub_cols: Vec::with_capacity(512),
            wave_sec_sub_cols: Vec::with_capacity(512),
            layout_transition,
            lyrics_scroll: LyricsScrollState::default(),
            frame_times: [1.0 / 144.0; 60],
            frame_idx: 0,
            measured_fps: 144.0,
        }
    }

    pub fn tick(&mut self, dt: f32, now: Instant, term_width: u16, term_height: u16) {
        self.frame_times[self.frame_idx] = dt.max(0.0001);
        self.frame_idx = (self.frame_idx + 1) % self.frame_times.len();
        let avg_dt: f32 =
            self.frame_times.iter().sum::<f32>() / (self.frame_times.len() as f32);
        self.measured_fps = 1.0 / avg_dt;

        if self.mpris_rx.has_changed().unwrap_or(false) {
            self.current_mpris = self.mpris_rx.borrow_and_update().clone();
            let track_key = self.current_mpris.metadata.cache_key();

            if track_key != self.last_track_key {
                self.last_track_key = track_key;
                self.lyrics_scroll.reset();
                self.current_lyrics = None;
                let _ = self.meta_tx.send(self.current_mpris.metadata.clone());
            }

            if self.current_mpris.metadata.art_url != self.last_art_url {
                self.last_art_url
                    .clone_from(&self.current_mpris.metadata.art_url);
                let _ = self.art_url_tx.send(self.last_art_url.clone());
            }
        }

        if self.cover_rx.has_changed().unwrap_or(false) {
            self.current_cover = self.cover_rx.borrow_and_update().clone();
            if let Some(ref cov) = self.current_cover {
                self.bg.set_target_palette(cov.palette);
            } else {
                self.bg.set_target_palette(ThemePalette::default());
            }
        }

        if self.lyrics_rx.has_changed().unwrap_or(false) {
            self.current_lyrics = self.lyrics_rx.borrow_and_update().clone();
        }

        let should_split = self.config.auto_lyrics_split
            && self.current_lyrics.is_some()
            && !self.current_mpris.metadata.is_empty();
        self.layout_transition.set_split_active(should_split, now);
        let split_t = self.layout_transition.update(now);

        let raw_bands = *self.audio_rx.borrow();
        let is_playing = self.current_mpris.clock.status == PlaybackStatus::Playing;
        self.smoother.step(&raw_bands, dt, is_playing);

        self.bg
            .update_and_ensure_cache(term_width, term_height, split_t, now);
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, dt: f32, now: Instant) {
        if area.width < 16 || area.height < 10 {
            return;
        }

        // 1. Fond sombre à halos radiaux Oklab
        self.bg.render(area, buf);

        let is_idle = self.current_mpris.metadata.is_empty();
        let split_t = self.layout_transition.value();

        // L'onde sonore ne s'affiche QU'EN MODE SANS PAROLES (Vue 1 : split_t < 1.0)
        // comme sur l'Image 3 (et disparaît complètement en Vue 2 Paroles comme sur l'Image 1).
        let wave_opacity = if is_idle {
            0.75
        } else {
            (1.0 - split_t * 1.25).clamp(0.0, 1.0)
        };

        let wave_h = ((area.height as u32 * 30) / 100).clamp(6, 18) as u16;
        let wave_area = Rect {
            x: area.x,
            y: area.bottom().saturating_sub(wave_h),
            width: area.width,
            height: wave_h,
        };

        if is_idle {
            let main_area = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: area.height.saturating_sub(wave_h),
            };
            self.render_idle_omarchy(main_area, buf);
            let sub_cols = (wave_area.width as usize) * 2;
            self.smoother
                .sample_catmull_rom(sub_cols, &mut self.wave_sub_cols);
            self.smoother
                .sample_secondary_catmull_rom(sub_cols, &mut self.wave_sec_sub_cols);
            render_braille_wave(
                wave_area,
                buf,
                &self.bg,
                &self.wave_sub_cols,
                &self.wave_sec_sub_cols,
                wave_opacity,
            );
        } else {
            let pos = self.current_mpris.clock.position_at(now);
            let dur = self.current_mpris.metadata.duration;

            let left_ratio = 1.0 - 0.56 * split_t;
            let left_w = ((area.width as f32) * left_ratio)
                .round()
                .clamp(22.0, area.width as f32) as u16;

            let effective_wave_deduction =
                ((wave_h as f32) * (1.0 - split_t) * 0.72).round() as u16;
            let left_area = Rect {
                x: area.x,
                y: area.y,
                width: left_w,
                height: area.height.saturating_sub(effective_wave_deduction),
            };

            let show_time_bar_under_artist = split_t >= 0.45;
            let meta_view = CoverMetaView {
                title: &self.current_mpris.metadata.title,
                artist: &self.current_mpris.metadata.artist,
                position: pos,
                duration: dur,
                show_time_bar_under_artist,
            };

            render_cover_and_meta(
                left_area,
                buf,
                &self.bg,
                self.current_cover.as_ref(),
                &mut self.cover_cache,
                &meta_view,
            );

            // Dessin de l'onde sonore uniquement en Vue 1 (Mode Sans Paroles — Image 3)
            if wave_opacity > 0.02 {
                let sub_cols = (wave_area.width as usize) * 2;
                self.smoother
                    .sample_catmull_rom(sub_cols, &mut self.wave_sub_cols);
                self.smoother
                    .sample_secondary_catmull_rom(sub_cols, &mut self.wave_sec_sub_cols);
                render_braille_wave(
                    wave_area,
                    buf,
                    &self.bg,
                    &self.wave_sub_cols,
                    &self.wave_sec_sub_cols,
                    wave_opacity,
                );

                // En Vue 1 (Image 3), la barre de progression passive est tout en bas au centre
                // par-dessus le bas de l'onde sonore !
                if !show_time_bar_under_artist {
                    let bar_y = area.bottom().saturating_sub(3);
                    let desired_bar_w = ((area.width as u32 * 22) / 100).clamp(18, 36) as u16;
                    let bar_area = Rect::new(area.x, bar_y, area.width, 1);
                    render_passive_time_bar(buf, bar_area, desired_bar_w, pos, dur, &self.bg);
                }
            }

            // En Vue 2 (Split Lyrics — Image 1), les paroles occupent toute la hauteur à droite sans onde en bas
            if split_t > 0.12 {
                if let Some(ref lyrics) = self.current_lyrics {
                    let right_x = area.x + left_w;
                    let right_w = area.width.saturating_sub(left_w);
                    if right_w >= 18 {
                        let right_area = Rect {
                            x: right_x,
                            y: area.y,
                            width: right_w,
                            height: area.height,
                        };
                        let opacity = ((split_t - 0.12) / 0.88).clamp(0.0, 1.0);
                        let params = LyricsRenderParams {
                            bg: &self.bg,
                            lyrics,
                            position: pos,
                            dt,
                            opacity,
                        };
                        render_synced_lyrics(right_area, buf, &mut self.lyrics_scroll, &params);
                    }
                }
            }
        }

        if self.debug_fps {
            let label = format!(" {:5.1} FPS ({:4.2} ms) ", self.measured_fps, dt * 1000.0);
            let x_start = area.right().saturating_sub(label.len() as u16 + 1);
            for (i, ch) in label.chars().enumerate() {
                if let Some(cell) = buf.cell_mut((x_start + i as u16, area.y)) {
                    cell.set_char(ch);
                    cell.set_fg(Color::Rgb(180, 220, 255));
                }
            }
        }
    }

    fn render_idle_omarchy(&self, area: Rect, buf: &mut Buffer) {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let hours = (secs / 3600) % 24;
        let mins = (secs / 60) % 60;
        let s = secs % 60;
        let clock_str = format!("{:02}:{:02}:{:02}", hours, mins, s);

        let cy = area.y + area.height / 2;
        let (ar, ag, ab) = oklab_to_rgb(self.bg.current_palette().accent);

        draw_centered_text(
            buf,
            area.x,
            cy.saturating_sub(1),
            area.width,
            &clock_str,
            Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        );
        draw_centered_text(
            buf,
            area.x,
            cy + 1,
            area.width,
            "kymat · en attente d'un média",
            Style::default().fg(Color::Rgb(ar, ag, ab)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::lyrics::parse_lrc;

    #[tokio::test]
    async fn test_full_frame_render_budget_under_6ms_at_144fps() {
        let cfg = AppConfig::default();
        let (_tx, audio_rx) = watch::channel([0.35_f32; NUM_BANDS]);
        let mut app = App::new(cfg, true, audio_rx);

        app.current_lyrics = parse_lrc(
            "[00:01.00] First line\n[00:05.00] Active centered line\n[00:10.00] Future line",
            "Test",
        );

        let width = 160u16;
        let height = 45u16;
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);

        let t0 = Instant::now();
        app.tick(1.0 / 144.0, t0, width, height);
        app.render(area, &mut buf, 1.0 / 144.0, t0);

        let bench_start = Instant::now();
        for frame_i in 1..=144 {
            let now = t0 + std::time::Duration::from_micros(6944 * frame_i);
            app.tick(1.0 / 144.0, now, width, height);
            app.render(area, &mut buf, 1.0 / 144.0, now);
        }
        let total_144_frames = bench_start.elapsed();
        let avg_frame_micros = total_144_frames.as_micros() / 144;

        assert!(
            avg_frame_micros < 2500,
            "Frame render too slow: {} µs avg",
            avg_frame_micros
        );
    }
}
