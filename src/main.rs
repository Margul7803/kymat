pub mod app;
pub mod audio;
pub mod config;
pub mod media;
pub mod ui;

use crate::app::App;
use crate::audio::capture::spawn_audio_capture_thread;
use crate::config::{AppConfig, CliArgs};
use anyhow::Result;
use clap::Parser;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use crossterm::terminal::{
    BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    disable_raw_mode, enable_raw_mode,
};
use crossterm::{execute, queue};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Write};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = CliArgs::parse();
    let config = AppConfig::load(&cli);

    // Under Hyprland/Wayland, terminals (foot/alacritty/ghostty) briefly spawn at 80x24
    // for a few milliseconds before the compositor applies fullscreen dimensions.
    if cli.screensaver {
        wait_for_fullscreen_pty_resize();
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let run_result = run_event_loop(&mut terminal, config, cli).await;

    // Unconditional atomic terminal state restoration
    let _ = execute!(
        terminal.backend_mut(),
        EndSynchronizedUpdate,
        DisableMouseCapture,
        Show,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();

    run_result
}

fn wait_for_fullscreen_pty_resize() {
    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        if let Ok((w, h)) = crossterm::terminal::size() {
            if (w, h) != (80, 24) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: AppConfig,
    cli: CliArgs,
) -> Result<()> {
    let target_dt = Duration::from_secs_f64(1.0 / (config.fps.max(15) as f64));
    let audio_rx = spawn_audio_capture_thread(config.audio_sensitivity);
    let mut app = App::new(config, cli.debug_fps, audio_rx);

    let startup_instant = Instant::now();
    let mut last_frame = Instant::now();
    let mut initial_mouse_pos: Option<(u16, u16)> = None;

    loop {
        let frame_start = Instant::now();
        let dt = frame_start
            .saturating_duration_since(last_frame)
            .as_secs_f32()
            .clamp(0.0005, 0.1);
        last_frame = frame_start;

        // Non-blocking poll of all pending terminal input events
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.code == KeyCode::Tab {
                        app.config.auto_lyrics_split = !app.config.auto_lyrics_split;
                        continue;
                    }
                    if matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R'))
                        && !key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.rat_easter_egg.toggle_manual();
                        continue;
                    }
                    if cli.screensaver {
                        return Ok(());
                    }
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) if cli.screensaver => match mouse.kind {
                    MouseEventKind::Down(_)
                    | MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown => {
                        return Ok(());
                    }
                    MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                        if frame_start.saturating_duration_since(startup_instant)
                            <= Duration::from_millis(1000)
                        {
                            initial_mouse_pos = Some((mouse.column, mouse.row));
                            continue;
                        }
                        let pos = (mouse.column, mouse.row);
                        match initial_mouse_pos {
                            None => {
                                initial_mouse_pos = Some(pos);
                            }
                            Some(init_pos) => {
                                if pos != init_pos {
                                    return Ok(());
                                }
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        let size = terminal.size()?;
        app.tick(dt, frame_start, size.width, size.height);

        // Atomic rendering with Synchronized Output (CSI ? 2026 h/l) to eliminate tearing at 144 FPS
        queue!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
        terminal.draw(|frame| {
            let area = frame.area();
            app.render(area, frame.buffer_mut(), dt, frame_start);
        })?;
        execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;
        terminal.backend_mut().flush()?;

        let elapsed = frame_start.elapsed();
        if elapsed < target_dt {
            tokio::time::sleep(target_dt - elapsed).await;
        } else {
            tokio::task::yield_now().await;
        }
    }
}
