use clap::Parser;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "kymat",
    about = "High-frequency (144 FPS) TUI/ASCII musical screensaver for Wayland/Hyprland (Omarchy x Sonora)"
)]
pub struct CliArgs {
    /// Active le mode économiseur d'écran (quitte immédiatement à la moindre touche ou mouvement/clic souris)
    #[arg(long)]
    pub screensaver: bool,

    /// Fréquence de rafraîchissement cible en FPS (surcharge config.toml)
    #[arg(long)]
    pub fps: Option<u32>,

    /// Affiche le compteur de FPS et le temps de frame en haut à droite
    #[arg(long)]
    pub debug_fps: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub fps: u32,
    pub audio_sensitivity: f32,
    pub attack_lambda: f32,
    pub decay_lambda: f32,
    pub background_transition_secs: f32,
    pub layout_transition_secs: f32,
    pub auto_lyrics_split: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            fps: 144,
            audio_sensitivity: 1.15,
            attack_lambda: 35.0,
            decay_lambda: 12.0,
            background_transition_secs: 1.5,
            layout_transition_secs: 0.6,
            auto_lyrics_split: true,
        }
    }
}

impl AppConfig {
    pub fn load(cli: &CliArgs) -> Self {
        let mut cfg = Self:: config_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|raw| toml::from_str::<AppConfig>(&raw).ok())
            .unwrap_or_default();

        if let Some(fps) = cli.fps {
            cfg.fps = fps.clamp(15, 360);
        }
        cfg
    }

    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("kymat").join("config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_and_cli_override() {
        let cli = CliArgs {
            screensaver: true,
            fps: Some(120),
            debug_fps: false,
        };
        let cfg = AppConfig::load(&cli);
        assert_eq!(cfg.fps, 120);
        assert!((cfg.attack_lambda - 35.0).abs() < f32::EPSILON);
        assert!((cfg.decay_lambda - 12.0).abs() < f32::EPSILON);
    }
}
