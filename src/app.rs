use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use crate::config::Config;
use crate::window::KovaWindow;

pub struct App {
    config: Config,
    session_backup: Option<usize>,
    start_dir: Option<String>,
    ipc_rx: mpsc::Receiver<String>,
    windows: HashMap<WindowId, KovaWindow>,
    /// Frame timing
    last_frame: Instant,
    frame_interval: Duration,
    /// Tick counter for periodic session save (~30s)
    tick_count: u64,
    /// Whether we already created the initial window(s)
    initialized: bool,
}

impl App {
    pub fn new(config: Config, session_backup: Option<usize>, start_dir: Option<String>, ipc_rx: mpsc::Receiver<String>) -> Self {
        let fps = config.terminal.fps;
        App {
            config,
            session_backup,
            start_dir,
            ipc_rx,
            windows: HashMap::new(),
            last_frame: Instant::now(),
            frame_interval: Duration::from_secs_f64(1.0 / fps as f64),
            tick_count: 0,
            initialized: false,
        }
    }

    fn create_initial_windows(&mut self, event_loop: &ActiveEventLoop) {
        // If a start directory is given, skip session restore
        if let Some(ref dir) = self.start_dir {
            let tab = crate::pane::Tab::new_with_cwd(&self.config, Some(dir.as_str()))
                .expect("failed to create initial tab");
            let proj = crate::pane::Project::new(dir.clone(), tab);
            match KovaWindow::new(event_loop, &self.config, vec![proj], 0) {
                Ok(win) => { self.windows.insert(win.id(), win); }
                Err(e) => log::error!("Failed to create window: {}", e),
            }
            return;
        }

        let restored = crate::session::load(self.session_backup)
            .and_then(|s| crate::session::restore_session(s, &self.config));

        match restored {
            Some(restored_windows) => {
                log::info!("Restoring {} window(s) from session", restored_windows.len());
                for rw in restored_windows {
                    match KovaWindow::new(event_loop, &self.config, rw.projects, rw.active_project) {
                        Ok(win) => {
                            let id = win.id();
                            self.windows.insert(id, win);
                        }
                        Err(e) => log::error!("Failed to create window: {}", e),
                    }
                }
            }
            None => {
                let tab = crate::pane::Tab::new(&self.config)
                    .expect("failed to create initial tab");
                let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
                let proj = crate::pane::Project::new(home, tab);
                match KovaWindow::new(event_loop, &self.config, vec![proj], 0) {
                    Ok(win) => {
                        let id = win.id();
                        self.windows.insert(id, win);
                    }
                    Err(e) => log::error!("Failed to create window: {}", e),
                }
            }
        }
    }

    fn save_session(&self) {
        let sessions: Vec<crate::session::WindowSession> = self
            .windows
            .values()
            .map(|w| w.session_data())
            .collect();
        if !sessions.is_empty() {
            let sessions_clone = sessions;
            std::thread::spawn(move || {
                crate::session::save(&sessions_clone);
            });
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.initialized {
            self.initialized = true;
            self.create_initial_windows(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Handle new-window request from child
        if let WindowEvent::Destroyed = &event {
            // winit sends Destroyed after close — remove from map
            self.windows.remove(&window_id);
            if self.windows.is_empty() {
                self.save_session();
                crate::terminal::pty::shutdown_all();
                event_loop.exit();
            }
            return;
        }

        let Some(win) = self.windows.get_mut(&window_id) else {
            return;
        };

        let action = win.handle_event(&event, &self.config);
        match action {
            WindowAction::None => {}
            WindowAction::CloseWindow => {
                // Save session data before closing
                self.windows.remove(&window_id);
                if self.windows.is_empty() {
                    self.save_session();
                    crate::terminal::pty::shutdown_all();
                    event_loop.exit();
                }
            }
            WindowAction::SaveSession => {
                self.save_session();
                log::info!("Session saved manually");
            }
            WindowAction::NewWindow => {
                let tab =
                    crate::pane::Tab::new(&self.config).expect("failed to create tab");
                let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
                let proj = crate::pane::Project::new(home, tab);
                match KovaWindow::new(event_loop, &self.config, vec![proj], 0) {
                    Ok(win) => {
                        let id = win.id();
                        self.windows.insert(id, win);
                    }
                    Err(e) => log::error!("Failed to create new window: {}", e),
                }
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Check for IPC messages (new project directories from other instances)
        while let Ok(dir) = self.ipc_rx.try_recv() {
            log::info!("IPC: opening project for {}", dir);
            // Open in the first window
            if let Some(win) = self.windows.values_mut().next() {
                win.open_project(&dir);
                win.request_redraw();
            }
        }

        let now = Instant::now();
        if now.duration_since(self.last_frame) >= self.frame_interval {
            self.last_frame = now;
            self.tick_count += 1;

            // Tick all windows (render if dirty, reap dead panes)
            let mut dead_windows = Vec::new();
            for (id, win) in &mut self.windows {
                if !win.tick() {
                    dead_windows.push(*id);
                }
            }

            for id in dead_windows {
                self.windows.remove(&id);
            }

            if self.windows.is_empty() {
                self.save_session();
                crate::terminal::pty::shutdown_all();
                // We can't call event_loop.exit() here since we don't have it
                // Instead rely on the main loop to detect empty windows
                std::process::exit(0);
            }

            // Periodic session save (~30s)
            let fps = self.config.terminal.fps as u64;
            if fps > 0 && self.tick_count % (fps * 30) == 0 {
                self.save_session();
            }
        }

        // Request redraw on all windows
        for win in self.windows.values() {
            win.request_redraw();
        }
    }
}

/// Action returned from window event handling to the app level.
pub enum WindowAction {
    None,
    CloseWindow,
    NewWindow,
    SaveSession,
}
