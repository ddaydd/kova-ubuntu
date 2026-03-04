mod app;
mod config;
mod input;
mod keybindings;
mod pane;
mod renderer;
mod session;
mod terminal;
mod window;

use log::LevelFilter;
use simplelog::{CombinedLogger, Config as LogConfig, TermLogger, TerminalMode, WriteLogger};
use std::fs;
use std::path::PathBuf;
use winit::event_loop::EventLoop;

fn log_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "kova")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/share/kova")
        })
        .join("logs")
}

fn setup_logging() {
    let dir = log_dir();
    fs::create_dir_all(&dir).expect("cannot create log dir");
    let log_file = fs::File::create(dir.join("kova.log")).expect("cannot create log file");

    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(LevelFilter::Debug);

    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> =
        vec![WriteLogger::new(level, LogConfig::default(), log_file)];

    if std::env::var("RUST_LOG").is_ok() {
        loggers.push(TermLogger::new(
            level,
            LogConfig::default(),
            TerminalMode::Stderr,
            simplelog::ColorChoice::Auto,
        ));
    }

    CombinedLogger::init(loggers).expect("cannot init logger");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--list-sessions") {
        session::list_session_backups();
        return;
    }

    let session_backup = args
        .windows(2)
        .find(|w| w[0] == "--session")
        .and_then(|w| w[1].parse::<usize>().ok());

    setup_logging();

    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {}", info);
    }));

    let config = config::Config::load();

    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = app::App::new(config, session_backup);
    event_loop.run_app(&mut app).expect("event loop error");
}
