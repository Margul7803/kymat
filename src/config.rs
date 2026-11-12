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
    /// Enable screensaver mode (exits immediately on any key press or mouse movement/click, except Tab)
    #[arg(long)]
    pub screensaver: bool,

    /// Target refresh rate in FPS (overrides config.toml)
    #[arg(long)]
    pub fps: Option<u32>,

    /// Terminal font size in pt (overrides config.toml)
    #[arg(long)]
    pub font_size: Option<u16>,

    /// Show live FPS and frame time counter in the top-right corner
    #[arg(long)]
    pub debug_fps: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub fps: u32,
    pub font_size: u16,
    pub audio_sensitivity: f32,
    pub attack_lambda: f32,
    pub decay_lambda: f32,
    pub background_transition_secs: f32,
    pub layout_transition_secs: f32,
    pub auto_lyrics_split: bool,
    pub rat_easter_egg: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            fps: 144,
            font_size: 12,
            audio_sensitivity: 1.15,
            attack_lambda: 35.0,
            decay_lambda: 12.0,
            background_transition_secs: 1.5,
            layout_transition_secs: 0.6,
            auto_lyrics_split: true,
            rat_easter_egg: true,
        }
    }
}

impl AppConfig {
    pub fn load(cli: &CliArgs) -> Self {
        let cfg_path = Self::config_path();
        if let Some(ref path) = cfg_path {
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(path, Self::default_toml_template());
            }
        }

        let mut cfg = cfg_path
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|raw| toml::from_str::<AppConfig>(&raw).ok())
            .unwrap_or_default();

        if let Some(fps) = cli.fps {
            cfg.fps = fps.clamp(15, 360);
        }
        if let Some(font_size) = cli.font_size {
            cfg.font_size = font_size.clamp(6, 48);
        } else {
            cfg.font_size = cfg.font_size.clamp(6, 48);
        }

        cfg.sync_foot_config();
        cfg
    }

    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("kymat").join("config.toml"))
    }

    pub fn foot_config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("kymat").join("foot-kymat.ini"))
    }

    pub fn sync_foot_config(&self) {
        let content = format!(
            "[main]\nfont=JetBrainsMono Nerd Font:size={}\npad=0x0\n\n[colors-dark]\nbackground=050508\nforeground=ffffff\n",
            self.font_size
        );
        if let Some(path) = Self::foot_config_path() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(path, &content);
        }
        let local_foot = PathBuf::from("foot-kymat.ini");
        if local_foot.exists() {
            let _ = fs::write(local_foot, &content);
        }
    }

    fn default_toml_template() -> &'static str {
        r#"fps = 144
font_size = 12
audio_sensitivity = 1.15
attack_lambda = 35.0
decay_lambda = 12.0
background_transition_secs = 1.5
layout_transition_secs = 0.6
auto_lyrics_split = true
rat_easter_egg = true
"#
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
            font_size: Some(12),
            debug_fps: false,
        };
        let cfg = AppConfig::load(&cli);
        assert_eq!(cfg.fps, 120);
        assert_eq!(cfg.font_size, 12);
        assert!((cfg.attack_lambda - 35.0).abs() < f32::EPSILON);
        assert!((cfg.decay_lambda - 12.0).abs() < f32::EPSILON);
    }
}
