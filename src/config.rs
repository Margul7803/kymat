use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    #[default]
    Lyrics,
    #[serde(alias = "wave")]
    #[value(alias = "wave")]
    Soundwave,
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "kymat",
    about = "High-frequency (144 FPS) TUI/ASCII musical screensaver for Wayland/Hyprland (Omarchy x Sonora)"
)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Enable screensaver mode (exits immediately on any key press or mouse movement/click, except Tab)
    #[arg(long)]
    pub screensaver: bool,

    /// Target refresh rate in FPS (overrides config.toml)
    #[arg(long)]
    pub fps: Option<u32>,

    /// Terminal font size in pt (overrides config.toml)
    #[arg(long)]
    pub font_size: Option<u16>,

    /// Default display mode: "lyrics" or "soundwave" (overrides config.toml)
    #[arg(long)]
    pub mode: Option<DisplayMode>,

    /// Show live FPS and frame time counter in the top-right corner
    #[arg(long)]
    pub debug_fps: bool,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Manage Kymat configuration (view, edit, locate config.toml)
    Config(ConfigArgs),
}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConfigAction {
    /// Open config.toml in your default editor ($VISUAL, $EDITOR, or nano/micro/vim)
    Edit,
    /// Print the absolute path to config.toml
    Path,
    /// Print the current configuration file contents
    Show,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub fps: u32,
    pub font_size: u16,
    pub default_mode: DisplayMode,
    pub rat_easter_egg: bool,
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
            font_size: 12,
            default_mode: DisplayMode::Lyrics,
            rat_easter_egg: false,
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
        let cfg_path = Self::config_path();
        if let Some(ref path) = cfg_path {
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(path, Self::default_toml_template());
            }
        }

        let mut raw_text = String::new();
        let mut cfg = cfg_path
            .and_then(|p| fs::read_to_string(p).ok())
            .inspect(|raw| raw_text.clone_from(raw))
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
        if let Some(mode) = cli.mode {
            cfg.default_mode = mode;
        }

        // Apply `default_mode` ("lyrics" vs "soundwave") unless only legacy `auto_lyrics_split = false` was set
        if cfg.default_mode == DisplayMode::Soundwave {
            cfg.auto_lyrics_split = false;
        } else if raw_text.contains("default_mode") || cli.mode.is_some() {
            cfg.auto_lyrics_split = true;
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
            "[main]\nfont=JetBrainsMono Nerd Font:size={},Noto Sans CJK JP:size={}\npad=0x0\n\n[colors-dark]\nbackground=050508\nforeground=ffffff\n",
            self.font_size, self.font_size
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

    pub fn default_toml_template() -> &'static str {
        r#"fps = 144
font_size = 12
# Default view mode: "lyrics" or "soundwave"
default_mode = "lyrics"
# Automatically show the dancing rat easter egg during musical climaxes (can always be toggled with 'r')
rat_easter_egg = false
audio_sensitivity = 1.15
attack_lambda = 35.0
decay_lambda = 12.0
background_transition_secs = 1.5
layout_transition_secs = 0.6
"#
    }
}

pub fn handle_config_command(args: &ConfigArgs) -> Result<()> {
    let action = args.action.as_ref().unwrap_or(&ConfigAction::Edit);
    let path = AppConfig::config_path()
        .context("Could not determine configuration directory ($XDG_CONFIG_HOME or ~/.config)")?;

    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, AppConfig::default_toml_template())?;
    }

    match action {
        ConfigAction::Path => {
            println!("{}", path.display());
            Ok(())
        }
        ConfigAction::Show => {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            print!("{}", content);
            Ok(())
        }
        ConfigAction::Edit => {
            let is_term = std::io::stdin().is_terminal();

            if is_term {
                let editor = get_terminal_editor();
                let parts: Vec<&str> = editor.split_whitespace().collect();
                if parts.is_empty() {
                    bail!("No editor command specified");
                }
                let cmd = parts[0];
                let cmd_args = &parts[1..];

                let status = std::process::Command::new(cmd)
                    .args(cmd_args)
                    .arg(&path)
                    .status();

                match status {
                    Ok(exit_status) => {
                        if !exit_status.success() {
                            eprintln!("Editor exited with status: {}", exit_status);
                        }
                        Ok(())
                    }
                    Err(e) => {
                        for fallback in ["nano", "micro", "nvim", "vim", "vi"] {
                            if cmd != fallback && binary_exists(fallback) {
                                eprintln!(
                                    "'{}' failed ({}), falling back to '{}'...",
                                    cmd, e, fallback
                                );
                                let _ = std::process::Command::new(fallback).arg(&path).status();
                                return Ok(());
                            }
                        }
                        bail!(
                            "Failed to launch editor '{}': {}. Please set your $EDITOR environment variable.",
                            editor,
                            e
                        );
                    }
                }
            } else {
                // Invoked outside an interactive TTY (e.g. desktop menu / application launcher)
                if binary_exists("omarchy-launch-editor") {
                    let _ = std::process::Command::new("omarchy-launch-editor")
                        .arg(&path)
                        .spawn();
                    Ok(())
                } else if binary_exists("xdg-open") {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&path)
                        .spawn();
                    Ok(())
                } else {
                    let editor = get_terminal_editor();
                    let parts: Vec<&str> = editor.split_whitespace().collect();
                    let cmd = parts[0];
                    let cmd_args = &parts[1..];
                    let _ = std::process::Command::new(cmd).args(cmd_args).arg(&path).status();
                    Ok(())
                }
            }
        }
    }
}

fn get_terminal_editor() -> String {
    std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            for candidate in ["micro", "nano", "nvim", "vim", "vi"] {
                if binary_exists(candidate) {
                    return candidate.to_string();
                }
            }
            "nano".to_string()
        })
}

fn binary_exists(bin: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let full_path = dir.join(bin);
            if full_path.is_file() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_and_cli_override() {
        let cli = CliArgs {
            command: None,
            screensaver: true,
            fps: Some(120),
            font_size: Some(12),
            mode: Some(DisplayMode::Soundwave),
            debug_fps: false,
        };
        let cfg = AppConfig::load(&cli);
        assert_eq!(cfg.fps, 120);
        assert_eq!(cfg.font_size, 12);
        assert_eq!(cfg.default_mode, DisplayMode::Soundwave);
        assert!(!cfg.auto_lyrics_split);
        assert!(!AppConfig::default().rat_easter_egg);
        assert!((cfg.attack_lambda - 35.0).abs() < f32::EPSILON);
        assert!((cfg.decay_lambda - 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_config_subcommands_parsing() {
        let cli_default_config = CliArgs::try_parse_from(["kymat", "config"]).unwrap();
        assert_eq!(
            cli_default_config.command,
            Some(Command::Config(ConfigArgs { action: None }))
        );

        let cli_edit = CliArgs::try_parse_from(["kymat", "config", "edit"]).unwrap();
        assert_eq!(
            cli_edit.command,
            Some(Command::Config(ConfigArgs {
                action: Some(ConfigAction::Edit)
            }))
        );

        let cli_path = CliArgs::try_parse_from(["kymat", "config", "path"]).unwrap();
        assert_eq!(
            cli_path.command,
            Some(Command::Config(ConfigArgs {
                action: Some(ConfigAction::Path)
            }))
        );

        let cli_show = CliArgs::try_parse_from(["kymat", "config", "show"]).unwrap();
        assert_eq!(
            cli_show.command,
            Some(Command::Config(ConfigArgs {
                action: Some(ConfigAction::Show)
            }))
        );
    }
}
