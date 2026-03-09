mod app;
mod config;
mod input;
mod install;
mod ipc;
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
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("kova.log"))
        .expect("cannot create log file");

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

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("Kova {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Kova {} — Fast GPU-accelerated terminal", env!("CARGO_PKG_VERSION"));
        println!();
        println!("Usage: kova [OPTIONS] [DIRECTORY]");
        println!();
        println!("Options:");
        println!("  --install             Register in app menu and 'Open With' for folders");
        println!("  --install --autostart Same + auto-start at login");
        println!("  --uninstall           Remove desktop integration");
        println!("  --list-sessions       List saved session backups");
        println!("  --session <N>         Restore session backup N");
        println!("  -h, --help            Show this help");
        println!();
        println!("Config: ~/.config/kova/config.toml");
        println!("Logs:   ~/.local/share/kova/logs/kova.log");
        return;
    }

    if args.iter().any(|a| a == "--list-sessions") {
        session::list_session_backups();
        return;
    }

    if args.iter().any(|a| a == "--install") {
        let autostart = args.iter().any(|a| a == "--autostart");
        install::install(autostart);
        return;
    }

    if args.iter().any(|a| a == "--uninstall") {
        install::uninstall();
        return;
    }

    let session_backup = args
        .windows(2)
        .find(|w| w[0] == "--session")
        .and_then(|w| w[1].parse::<usize>().ok());

    setup_logging();
    log::info!("Kova v{} starting", env!("CARGO_PKG_VERSION"));

    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {}", info);
    }));

    // First non-option argument is the starting directory
    let start_dir = args.iter().skip(1)
        .find(|a| !a.starts_with('-'))
        .map(|s| s.strip_prefix("file://").unwrap_or(s).to_string())
        .filter(|s| std::path::Path::new(s).is_dir());

    // Single-instance: if a directory was given, try sending it to existing instance
    if let Some(ref dir) = start_dir {
        if ipc::try_send(dir) {
            log::info!("Sent directory to existing instance, exiting");
            return;
        }
    }

    // Start IPC listener for future instances
    let ipc_rx = ipc::start_listener();

    let config = config::Config::load();

    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = app::App::new(config, session_backup, start_dir, ipc_rx);
    event_loop.run_app(&mut app).expect("event loop error");

    ipc::cleanup();
}
