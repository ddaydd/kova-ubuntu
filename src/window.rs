use parking_lot::RwLock;
use std::sync::Arc;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

use crate::app::WindowAction;
use crate::config::Config;
use crate::input;
use crate::keybindings::{Action, KeyCombo, Keybindings};
use crate::pane::{NavDirection, Pane, PaneId, Project, SplitDirection, SplitTree, Tab};
use crate::renderer::{FilterRenderData, PaneViewport, Renderer};
use crate::session::WindowSession;
use crate::terminal::{FilterMatch, TerminalState};

struct FilterState {
    query: String,
    matches: Vec<FilterMatch>,
}

/// View mode for the project bar: show a single project, all panes, or filtered views.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// Normal mode: show active project's tabs.
    Project,
    /// Show all panes from all projects.
    All,
    /// Show only panes running Claude Code.
    Claude,
    /// Show only panes that are NOT running Claude Code.
    Terminal,
}

struct RenameTabState {
    input: String,
}

struct RenameProjectState {
    project_idx: usize,
    input: String,
}

/// State for mouse text selection.
struct TextSelectState {
    /// Pane being selected in.
    pane_id: PaneId,
    /// Viewport of the pane (for coordinate conversion).
    viewport: PaneViewport,
}

/// Context menu items.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ContextMenuItem {
    Copy,
    Paste,
}

/// Context menu state (shown on right-click in pane area).
struct ContextMenu {
    /// Logical position of menu.
    x: f32,
    y: f32,
    /// Which item is hovered.
    hovered: Option<ContextMenuItem>,
    /// Whether there is a selection (enables Copy).
    has_selection: bool,
}

/// State for dragging a tab between projects.
struct DragState {
    /// Project index the tab is being dragged from.
    src_project: usize,
    /// Tab index being dragged.
    src_tab: usize,
    /// Whether we've moved enough to consider it a drag (vs a click).
    dragging: bool,
    /// Mouse position at press (logical pixels).
    start_x: f64,
    start_y: f64,
}

pub struct KovaWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    projects: Vec<Project>,
    active_project: usize,
    keybindings: Keybindings,
    config: Config,
    last_scale: f64,
    filter: Option<FilterState>,
    rename_tab: Option<RenameTabState>,
    rename_project: Option<RenameProjectState>,
    show_help: bool,
    modifiers: winit::event::Modifiers,
    closing: bool,
    /// Current mouse position in physical pixels.
    mouse_x: f64,
    mouse_y: f64,
    /// Tab drag & drop state.
    drag: Option<DragState>,
    /// Git branch poll counter (ticks since last poll).
    git_poll_counter: u32,
    git_poll_interval: u32,
    /// Frames remaining to show the "F1 for help" hint at startup.
    help_hint_frames: u32,
    /// View mode: Project (normal), All, Claude, or Terminal.
    view_mode: ViewMode,
    /// Mouse text selection state.
    text_select: Option<TextSelectState>,
    /// Context menu (right-click).
    context_menu: Option<ContextMenu>,
    /// Toast notification (frames remaining + message).
    toast_frames: u32,
    toast_text: String,
    /// Whether the window currently has OS focus.
    window_focused: bool,
    /// Cooldown frames before next system notification (avoids spam).
    notify_cooldown: u32,
}

impl KovaWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        config: &Config,
        projects: Vec<Project>,
        active_project: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let attrs = WindowAttributes::default()
            .with_title("Kova")
            .with_visible(false)
            .with_inner_size(LogicalSize::new(config.window.width, config.window.height));

        let window = Arc::new(event_loop.create_window(attrs)?);
        let scale = window.scale_factor();

        // Init wgpu
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });

        // SAFETY: window Arc keeps the window alive for the lifetime of the surface
        let surface = unsafe {
            instance.create_surface(wgpu::SurfaceTarget::from(window.clone()))
        }?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or("No suitable GPU adapter found")?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("kova_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        ))?;

        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let renderer = Renderer::new(&device, &queue, surface_format, scale, config);
        let keybindings = Keybindings::from_config(&config.keys);

        let fps = config.terminal.fps;

        let mut win = KovaWindow {
            window,
            surface,
            device,
            queue,
            surface_config,
            renderer,
            projects,
            active_project,
            keybindings,
            config: config.clone(),
            last_scale: scale,
            filter: None,
            rename_tab: None,
            rename_project: None,
            show_help: false,
            modifiers: Default::default(),
            closing: false,
            mouse_x: 0.0,
            mouse_y: 0.0,
            drag: None,
            git_poll_counter: 0,
            git_poll_interval: fps * 2,
            help_hint_frames: fps * 3,
            view_mode: ViewMode::Project,
            text_select: None,
            context_menu: None,
            toast_frames: 0,
            toast_text: String::new(),
            window_focused: true,
            notify_cooldown: 0,
        };

        // Initial resize + first render, then show window
        win.resize_all_panes();
        win.render();
        win.window.set_visible(true);

        Ok(win)
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn show_toast(&mut self, msg: &str) {
        let fps = self.config.terminal.fps;
        self.toast_text = msg.to_string();
        self.toast_frames = fps * 2; // 2 seconds
    }

    // --- Project/Tab helpers ---

    fn project(&self) -> &Project {
        &self.projects[self.active_project]
    }

    fn project_mut(&mut self) -> &mut Project {
        &mut self.projects[self.active_project]
    }

    fn tabs(&self) -> &[Tab] {
        &self.project().tabs
    }

    fn active_tab_idx(&self) -> usize {
        self.project().active_tab
    }

    /// Open a directory as a new tab in the active project (orphan tab).
    /// The user can then move it to another project with Super+Alt+Shift+Left/Right.
    pub fn open_project(&mut self, dir: &str) {
        match crate::pane::Tab::new_with_cwd(&self.config, Some(dir)) {
            Ok(tab) => {
                let proj = self.project_mut();
                proj.tabs.push(tab);
                proj.active_tab = proj.tabs.len() - 1;
                self.resize_all_panes();
            }
            Err(e) => log::error!("Failed to create tab for {}: {}", dir, e),
        }
    }

    /// Create a new empty project (shell in $HOME).
    fn do_new_project(&mut self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        match crate::pane::Tab::new_with_cwd(&self.config, Some(&home)) {
            Ok(tab) => {
                let proj = Project::new(home, tab);
                self.projects.push(proj);
                self.active_project = self.projects.len() - 1;
                self.resize_all_panes();
            }
            Err(e) => log::error!("Failed to create project: {}", e),
        }
    }

    /// Move the active tab from the current project to the next/previous project.
    fn do_move_tab_to_project(&mut self, delta: i32) {
        if self.projects.len() < 2 {
            return;
        }
        let src = self.active_project;
        let dst = ((src as i32 + delta).rem_euclid(self.projects.len() as i32)) as usize;
        if src == dst {
            return;
        }
        let src_proj = &mut self.projects[src];
        if src_proj.tabs.len() <= 1 {
            // Don't leave an empty project — move the whole project instead
            return;
        }
        let tab_idx = src_proj.active_tab;
        let tab = src_proj.tabs.remove(tab_idx);
        src_proj.active_tab = src_proj.active_tab.min(src_proj.tabs.len() - 1);

        let dst_proj = &mut self.projects[dst];
        dst_proj.tabs.push(tab);
        dst_proj.active_tab = dst_proj.tabs.len() - 1;
        self.active_project = dst;
        self.resize_all_panes();
    }

    pub fn session_data(&self) -> WindowSession {
        let pos = self.window.inner_position().ok();
        let size = self.window.inner_size();
        let frame = pos.map(|p| {
            (
                p.x as f64,
                p.y as f64,
                size.width as f64,
                size.height as f64,
            )
        });
        WindowSession::from_projects(&self.projects, self.active_project, frame)
    }

    /// Per-frame tick. Returns false if window should close.
    pub fn tick(&mut self) -> bool {
        if self.closing {
            return false;
        }

        // Reap dead panes
        let proj = &self.projects[self.active_project];
        let mut dead_ids = Vec::new();
        if let Some(tab) = proj.tabs.get(proj.active_tab) {
            dead_ids = tab.tree.exited_pane_ids();
        }
        for id in dead_ids {
            self.close_pane(id);
        }

        // Inject pending commands
        for proj in &self.projects {
            for tab in &proj.tabs {
                tab.tree.for_each_pane(&mut |pane| {
                    pane.inject_pending_command();
                });
            }
        }

        // Git branch polling
        self.git_poll_counter += 1;
        if self.git_poll_counter >= self.git_poll_interval {
            self.git_poll_counter = 0;
            if let Some(pane) = self.focused_pane() {
                if let Some(cwd) = pane.cwd() {
                    let branch = crate::terminal::parser::resolve_git_branch(&cwd);
                    let mut term = pane.terminal.write();
                    term.git_branch = branch;
                    term.cwd = Some(cwd);
                }
            }
        }

        // Check bells
        if self.notify_cooldown > 0 {
            self.notify_cooldown -= 1;
        }
        let mut new_bell = false;
        for proj in &mut self.projects {
            for tab in &mut proj.tabs {
                let had_bell = tab.has_bell;
                if tab.check_bell() && !had_bell {
                    new_bell = true;
                }
            }
        }
        if new_bell {
            log::info!("new_bell=true, window_focused={}, notify_cooldown={}", self.window_focused, self.notify_cooldown);
        }
        if new_bell && self.notify_cooldown == 0 {
            self.notify_cooldown = self.config.terminal.fps * 5; // 5s cooldown
            log::info!("Sending notify-send for bell");
            std::process::Command::new("notify-send")
                .args(["--app-name=Kova", "-i", "utilities-terminal", "Kova", "Bell received"])
                .spawn()
                .ok();
        }

        // Check command completions (OSC 133 shell integration)
        let threshold = self.config.terminal.notify_threshold_secs;
        if threshold > 0 {
            let mut notify_msg = None;
            for proj in &self.projects {
                for tab in &proj.tabs {
                    tab.tree.for_each_pane(&mut |pane| {
                        if let Some((elapsed, cmd)) = pane.terminal.read().command_completion.lock().take() {
                            if elapsed.as_secs() >= threshold {
                                let msg = if let Some(c) = cmd {
                                    format!("'{}' finished ({}s)", c, elapsed.as_secs())
                                } else {
                                    format!("Command finished ({}s)", elapsed.as_secs())
                                };
                                notify_msg = Some(msg);
                            }
                        }
                    });
                }
            }
            if let Some(msg) = notify_msg {
                log::info!("Command completion: {}", msg);
                std::process::Command::new("notify-send")
                    .args(["--app-name=Kova", "-i", "utilities-terminal", "Kova", &msg])
                    .spawn()
                    .ok();
                self.show_toast(&msg);
            }
        }

        // Help hint countdown
        if self.help_hint_frames > 0 {
            self.help_hint_frames -= 1;
        }

        // Toast countdown
        if self.toast_frames > 0 {
            self.toast_frames -= 1;
        }

        // Render
        self.render();

        !self.projects.is_empty()
    }

    fn render(&mut self) {
        if self.projects.is_empty() {
            return;
        }

        let size = self.window.inner_size();
        let viewport_w = size.width as f32;
        let viewport_h = size.height as f32;

        if viewport_w == 0.0 || viewport_h == 0.0 {
            return;
        }

        let (_, cell_h) = self.renderer.cell_size();
        let project_bar_h = (cell_h * 1.5).round();
        let tab_bar_h = if self.is_grid_view() { 0.0 } else { (cell_h * 2.0).round() };
        let global_bar_h = cell_h;
        let bars_h = project_bar_h + tab_bar_h;
        let pane_area_y = bars_h;
        let pane_area_h = viewport_h - bars_h - global_bar_h;

        // Collect tabs based on view mode
        let all_tabs: Vec<(&Tab, bool)> = self.visible_tabs();

        if all_tabs.is_empty() {
            // Filter mode has no matches — fall back to Project mode
            if self.view_mode != ViewMode::Project {
                self.view_mode = ViewMode::Project;
                return;
            }
            return;
        }

        let tab_count = all_tabs.len();

        // Grid layout (Termix algorithm)
        let (grid_cols, grid_rows) = if tab_count <= 1 {
            (1, 1)
        } else {
            let ratio = viewport_w / pane_area_h;
            let cols = (((tab_count as f32) * ratio).sqrt()).round() as usize;
            let cols = cols.max(1).min(tab_count);
            let rows = (tab_count + cols - 1) / cols;
            (cols, rows)
        };

        let gap = 2.0_f32;
        let cell_w_grid = (viewport_w - (grid_cols as f32 - 1.0) * gap) / grid_cols as f32;
        let cell_h_grid = (pane_area_h - (grid_rows as f32 - 1.0) * gap) / grid_rows as f32;

        let mut panes = Vec::new();
        let mut separators = Vec::new();

        for (tab_i, (tab, is_active_tab)) in all_tabs.iter().enumerate() {
            let col = tab_i % grid_cols;
            let row = tab_i / grid_cols;
            let cell_x = col as f32 * (cell_w_grid + gap);
            let cell_y = pane_area_y + row as f32 * (cell_h_grid + gap);

            let tab_vp = PaneViewport {
                x: cell_x,
                y: cell_y,
                width: cell_w_grid,
                height: cell_h_grid,
            };

            tab.tree.for_each_pane_with_viewport(tab_vp, &mut |pane, vp| {
                let is_focused = *is_active_tab && pane.id == tab.focused_pane;
                panes.push((
                    pane.terminal.clone(),
                    vp,
                    pane.is_ready(),
                    is_focused,
                    pane.id,
                    pane.custom_title.clone(),
                ));
            });

            tab.tree.collect_separators(tab_vp, &mut separators);
        }

        // Project titles: prepend filter tabs (All, Claude, Terminal) then projects
        let has_filter_tabs = self.projects.len() >= 2;
        let has_claude = self.has_any_claude_pane();
        let mut project_titles: Vec<(String, bool)> = Vec::new();
        if has_filter_tabs {
            project_titles.push(("All".to_string(), self.view_mode == ViewMode::All));
            if has_claude {
                project_titles.push(("Claude".to_string(), self.view_mode == ViewMode::Claude));
                project_titles.push(("Terminal".to_string(), self.view_mode == ViewMode::Terminal));
            }
        }
        let filter_offset = project_titles.len();
        project_titles.extend(self.projects.iter().enumerate().map(|(i, p)| {
            let name = if self.rename_project.as_ref().is_some_and(|r| r.project_idx == i) {
                format!("{}|", self.rename_project.as_ref().unwrap().input)
            } else {
                p.name()
            };
            (name, self.view_mode == ViewMode::Project && i == self.active_project)
        }));

        // Tab titles (hidden in grid view modes)
        let tab_titles: Vec<(String, bool, Option<usize>, bool, bool)> = if self.is_grid_view() {
            Vec::new()
        } else {
            let proj = &self.projects[self.active_project];
            let active_tab_idx = proj.active_tab;
            proj.tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let is_renaming = i == active_tab_idx && self.rename_tab.is_some();
                    let title = if is_renaming {
                        self.rename_tab.as_ref().unwrap().input.clone()
                    } else {
                        t.title()
                    };
                    (title, i == active_tab_idx, t.color, is_renaming, t.has_bell)
                })
                .collect()
        };

        let filter_data = self.filter.as_ref().map(|f| FilterRenderData {
            query: f.query.clone(),
            matches: f.matches.clone(),
        });

        // Drag label: tab title floating near cursor
        let drag_label: Option<(String, f32, f32)> = self.drag.as_ref().and_then(|d| {
            if !d.dragging { return None; }
            let title = self.projects.get(d.src_project)
                .and_then(|p| p.tabs.get(d.src_tab))
                .map(|t| t.title())?;
            Some((title, self.mouse_x as f32, self.mouse_y as f32))
        });
        let drag_ref = drag_label.as_ref().map(|(s, x, y)| (s.as_str(), *x, *y));

        let ctx_menu = self.context_menu.as_ref().map(|m| {
            let hovered = match m.hovered {
                Some(ContextMenuItem::Copy) => Some(0u8),
                Some(ContextMenuItem::Paste) => Some(1u8),
                None => None,
            };
            (m.x, m.y, m.has_selection, hovered)
        });

        self.renderer.render_panes(
            &self.device,
            &self.queue,
            &self.surface,
            &panes,
            &separators,
            &project_titles,
            &tab_titles,
            filter_data.as_ref(),
            self.show_help,
            self.help_hint_frames,
            0.0,
            0,
            0,
            drag_ref,
            ctx_menu,
            if self.toast_frames > 0 { Some((self.toast_text.as_str(), self.toast_frames)) } else { None },
        );
    }

    pub fn handle_event(&mut self, event: &WindowEvent, config: &Config) -> WindowAction {
        match event {
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.surface_config.width = size.width;
                    self.surface_config.height = size.height;
                    self.surface.configure(&self.device, &self.surface_config);
                    self.resize_all_panes();
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if (*scale_factor - self.last_scale).abs() > 0.01 {
                    self.last_scale = *scale_factor;
                    self.renderer.rebuild_atlas(&self.device, &self.queue, *scale_factor);
                    self.resize_all_panes();
                }
            }

            WindowEvent::Focused(focused) => {
                log::info!("Window focused: {}", focused);
                self.window_focused = *focused;
            }

            WindowEvent::CloseRequested => {
                self.closing = true;
                return WindowAction::CloseWindow;
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = *modifiers;
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return WindowAction::None;
                }

                // Dismiss startup hint on any keypress
                self.help_hint_frames = 0;

                // Handle help overlay mode
                if self.show_help {
                    match &event.logical_key {
                        Key::Named(NamedKey::Escape) | Key::Named(NamedKey::F1) => {
                            self.show_help = false;
                        }
                        _ => {}
                    }
                    return WindowAction::None;
                }

                // Handle rename project mode
                if let Some(ref mut rename) = self.rename_project {
                    match &event.logical_key {
                        Key::Named(NamedKey::Enter) => {
                            let idx = rename.project_idx;
                            let new_name = rename.input.clone();
                            if let Some(proj) = self.projects.get_mut(idx) {
                                proj.custom_name = if new_name.is_empty() {
                                    None
                                } else {
                                    Some(new_name)
                                };
                            }
                            self.rename_project = None;
                        }
                        Key::Named(NamedKey::Escape) => {
                            self.rename_project = None;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            rename.input.pop();
                        }
                        Key::Character(s) => {
                            rename.input.push_str(s);
                        }
                        _ => {}
                    }
                    return WindowAction::None;
                }

                // Handle rename tab mode
                if let Some(ref mut rename) = self.rename_tab {
                    match &event.logical_key {
                        Key::Named(NamedKey::Enter) => {
                            let new_title = rename.input.clone();
                            if let Some(tab) = self.project_mut().active_tab_mut() {
                                tab.custom_title = if new_title.is_empty() {
                                    None
                                } else {
                                    Some(new_title)
                                };
                            }
                            self.rename_tab = None;
                        }
                        Key::Named(NamedKey::Escape) => {
                            self.rename_tab = None;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            rename.input.pop();
                        }
                        Key::Character(s) => {
                            rename.input.push_str(s);
                        }
                        _ => {}
                    }
                    return WindowAction::None;
                }

                // Handle filter mode
                if let Some(ref mut filter) = self.filter {
                    match &event.logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.filter = None;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            filter.query.pop();
                            self.update_filter_matches();
                        }
                        Key::Character(s) => {
                            filter.query.push_str(s);
                            self.update_filter_matches();
                        }
                        _ => {}
                    }
                    return WindowAction::None;
                }

                // Check window-level keybindings
                let combo = KeyCombo::from_winit(&event.logical_key, &self.modifiers);
                log::debug!("Key event: {:?} -> combo: {:?}, super={}, text={:?}", event.logical_key, combo, self.modifiers.state().super_key(), event.text);

                if let Some(action) = self.keybindings.window_map.get(&combo).cloned() {
                    match action {
                        Action::NewWindow => return WindowAction::NewWindow,
                        Action::CloseWindow => {
                            self.closing = true;
                            return WindowAction::CloseWindow;
                        }
                        Action::KillWindow => {
                            self.closing = true;
                            return WindowAction::CloseWindow;
                        }
                        Action::NewTab => self.do_new_tab(),
                        Action::ClosePaneOrTab => self.do_close_pane_or_tab(),
                        Action::VSplit => self.do_split(SplitDirection::Horizontal),
                        Action::HSplit => self.do_split(SplitDirection::Vertical),
                        Action::VSplitRoot => self.do_split_root(SplitDirection::Horizontal),
                        Action::HSplitRoot => self.do_split_root(SplitDirection::Vertical),
                        Action::PrevTab => self.do_switch_tab_relative(-1),
                        Action::NextTab => self.do_switch_tab_relative(1),
                        Action::SwitchTab(idx) => {
                            let proj = self.project_mut();
                            if idx < proj.tabs.len() {
                                proj.active_tab = idx;
                                proj.tabs[idx].clear_bell();
                                self.resize_all_panes();
                            }
                        }
                        Action::Navigate(dir) => self.do_navigate(dir),
                        Action::SwapPane(dir) => self.do_swap_pane(dir),
                        Action::Resize(axis, delta) => {
                            if let Some(tab) = self.project_mut().active_tab_mut() {
                                let focused_id = tab.focused_pane;
                                if tab.tree.adjust_ratio_for_pane(focused_id, delta, axis) {
                                    self.resize_all_panes();
                                }
                            }
                        }
                        Action::ToggleFilter => self.toggle_filter(),
                        Action::ToggleFullscreen => {
                            let is_fullscreen = self.window.fullscreen().is_some();
                            log::info!("ToggleFullscreen: currently fullscreen={}", is_fullscreen);
                            if is_fullscreen {
                                self.window.set_fullscreen(None);
                            } else {
                                self.window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                            }
                        }
                        Action::ToggleHelp => self.show_help = !self.show_help,
                        Action::SaveSession => return WindowAction::SaveSession,
                        Action::ClearScrollback => {
                            if let Some(pane) = self.focused_pane() {
                                pane.terminal.write().clear_scrollback_and_screen();
                                pane.pty.write(b"\x0c");
                            }
                        }
                        Action::RenameTab => {
                            let current = self.project().active_tab()
                                .and_then(|t| t.custom_title.clone())
                                .unwrap_or_default();
                            self.rename_tab = Some(RenameTabState { input: current });
                        }
                        Action::RenamePane => {} // TODO
                        Action::DetachTab => {}  // TODO
                        Action::MergeWindow => {} // TODO
                        Action::MoveTabToNextProject => self.do_move_tab_to_project(1),
                        Action::MoveTabToPrevProject => self.do_move_tab_to_project(-1),
                        Action::ShowAllTerminals => {
                            if self.projects.len() >= 2 {
                                self.view_mode = if self.view_mode == ViewMode::All {
                                    ViewMode::Project
                                } else {
                                    ViewMode::All
                                };
                                self.resize_all_panes();
                            }
                        }
                        Action::Copy => {
                            if let Some(pane) = self.focused_pane() {
                                let mut term = pane.terminal.write();
                                let text = term.selected_text();
                                if !text.is_empty() {
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(&text);
                                    }
                                    term.clear_selection();
                                }
                            }
                        }
                        Action::Paste => {
                            if let Some(pane) = self.focused_pane() {
                                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                    if let Ok(text) = clipboard.get_text() {
                                        let bracketed = pane.terminal.read().bracketed_paste;
                                        if bracketed {
                                            pane.pty.write(b"\x1b[200~");
                                        }
                                        pane.pty.write(text.as_bytes());
                                        if bracketed {
                                            pane.pty.write(b"\x1b[201~");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    return WindowAction::None;
                }

                log::debug!("Key not matched by keybindings: {:?}", combo);

                // Forward to terminal input handler
                if let Some(pane) = self.focused_pane() {
                    let cursor_keys_app = pane.terminal.read().cursor_keys_application;
                    pane.terminal.write().reset_scroll();
                    input::handle_key_event(
                        &event.logical_key,
                        &self.modifiers,
                        &pane.pty,
                        cursor_keys_app,
                        &self.keybindings,
                        event.text.as_deref(),
                    );
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_x = position.x;
                self.mouse_y = position.y;
                // Check drag threshold (5 physical pixels)
                if let Some(ref mut drag) = self.drag {
                    if !drag.dragging {
                        let dx = position.x - drag.start_x;
                        let dy = position.y - drag.start_y;
                        if dx * dx + dy * dy > 25.0 {
                            drag.dragging = true;
                        }
                    }
                }
                // Update text selection during drag
                if let Some(ref sel) = self.text_select {
                    let x = position.x as f32;
                    let y = position.y as f32;
                    let pane_id = sel.pane_id;
                    let vp = sel.viewport;
                    // Find the pane by id to update selection
                    for proj in &self.projects {
                        for tab in &proj.tabs {
                            if let Some(pane) = tab.tree.pane(pane_id) {
                                let (col, abs_line) = self.mouse_to_grid(x, y, &vp, pane);
                                let mut term = pane.terminal.write();
                                if let Some(ref mut selection) = term.selection {
                                    selection.end = crate::terminal::GridPos { line: abs_line, col };
                                }
                                return WindowAction::None;
                            }
                        }
                    }
                }
                // Update context menu hover
                if let Some(ref mut menu) = self.context_menu {
                    let mx = self.mouse_x as f32;
                    let my = self.mouse_y as f32;
                    let (cell_w, cell_h) = self.renderer.cell_size();
                    let item_w = cell_w * 12.0;
                    let item_h = cell_h * 1.8;
                    if mx >= menu.x && mx < menu.x + item_w {
                        let rel_y = my - menu.y;
                        if rel_y >= 0.0 && rel_y < item_h {
                            menu.hovered = if menu.has_selection { Some(ContextMenuItem::Copy) } else { None };
                        } else if rel_y >= item_h && rel_y < item_h * 2.0 {
                            menu.hovered = Some(ContextMenuItem::Paste);
                        } else {
                            menu.hovered = None;
                        }
                    } else {
                        menu.hovered = None;
                    }
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let x = self.mouse_x;
                let y = self.mouse_y;
                let (cell_w, cell_h) = self.renderer.cell_size();

                // Check if clicking inside context menu
                if let Some(menu) = &self.context_menu {
                    let mx = x as f32;
                    let my = y as f32;
                    let item_w = cell_w * 12.0;
                    let item_h = cell_h * 1.8;
                    let mut action = None;
                    if mx >= menu.x && mx < menu.x + item_w {
                        let rel_y = my - menu.y;
                        if rel_y >= 0.0 && rel_y < item_h && menu.has_selection {
                            action = Some(ContextMenuItem::Copy);
                        } else if rel_y >= item_h && rel_y < item_h * 2.0 {
                            action = Some(ContextMenuItem::Paste);
                        }
                    }
                    self.do_context_action(action);
                    return WindowAction::None;
                }

                let project_bar_h = (cell_h * 1.5).round();
                let tab_bar_h = (cell_h * 2.0).round();
                let viewport_h = self.surface_config.height as f32;

                if (y as f32) < project_bar_h {
                    // Click in project bar
                    let viewport_w = self.surface_config.width as f64;
                    let max_proj_w = cell_w as f64 * 20.0;
                    let has_filter_tabs = self.projects.len() >= 2;
                    let has_claude = self.has_any_claude_pane();
                    // Filter slots: [All] [Claude] [Terminal] (Claude+Terminal only if claude panes exist)
                    let filter_count: usize = if has_filter_tabs {
                        if has_claude { 3 } else { 1 }
                    } else {
                        0
                    };
                    let slot_count = filter_count + self.projects.len() + 1;
                    let proj_width = (viewport_w / slot_count as f64).min(max_proj_w);
                    let clicked = (x / proj_width) as usize;
                    if has_filter_tabs && clicked < filter_count {
                        // Clicked a filter tab
                        let filter_modes = if has_claude {
                            vec![ViewMode::All, ViewMode::Claude, ViewMode::Terminal]
                        } else {
                            vec![ViewMode::All]
                        };
                        let target = filter_modes[clicked];
                        self.view_mode = if self.view_mode == target {
                            ViewMode::Project
                        } else {
                            target
                        };
                        self.resize_all_panes();
                    } else if clicked >= filter_count && clicked < filter_count + self.projects.len() {
                        // Switch to specific project
                        self.view_mode = ViewMode::Project;
                        self.active_project = clicked - filter_count;
                        self.resize_all_panes();
                    } else {
                        // "+" button
                        self.do_new_project();
                    }
                } else if !self.is_grid_view() && (y as f32) < project_bar_h + tab_bar_h {
                    // Click in tab bar (not visible in show_all mode)
                    let viewport_w = self.surface_config.width as f64;
                    let max_tab_w = cell_w as f64 * 20.0;
                    let tab_count = self.project().tabs.len();
                    let tab_width = (viewport_w / (tab_count + 1) as f64).min(max_tab_w);
                    let plus_x = tab_count as f64 * tab_width;
                    if x >= plus_x && x < plus_x + tab_width {
                        self.do_new_tab();
                    } else {
                        let clicked_tab = (x / tab_width) as usize;
                        if clicked_tab < tab_count {
                            // Start potential drag
                            self.drag = Some(DragState {
                                src_project: self.active_project,
                                src_tab: clicked_tab,
                                dragging: false,
                                start_x: x,
                                start_y: y,
                            });
                            // Switch to tab immediately
                            self.project_mut().active_tab = clicked_tab;
                            self.project_mut().tabs[clicked_tab].clear_bell();
                            self.resize_all_panes();
                        }
                    }
                } else {
                    // Click in pane area — dismiss context menu, switch focus, start text selection
                    self.context_menu = None;
                    let mut clicked_pane_id = None;
                    if let Some((pane, vp)) = self.pane_at(x as f32, y as f32) {
                        let pane_id = pane.id;
                        clicked_pane_id = Some(pane_id);
                        let (col, abs_line) = self.mouse_to_grid(x as f32, y as f32, &vp, pane);
                        // Start text selection
                        let anchor = crate::terminal::GridPos { line: abs_line, col };
                        pane.terminal.write().selection = Some(crate::terminal::Selection {
                            anchor,
                            end: anchor,
                        });
                        self.text_select = Some(TextSelectState { pane_id, viewport: vp });
                    }
                    // Switch focus to clicked pane (after pane_at borrow is dropped)
                    if let Some(pane_id) = clicked_pane_id {
                        'focus: for (pi, proj) in self.projects.iter_mut().enumerate() {
                            for (ti, tab) in proj.tabs.iter_mut().enumerate() {
                                if tab.tree.pane(pane_id).is_some() {
                                    self.active_project = pi;
                                    proj.active_tab = ti;
                                    tab.focused_pane = pane_id;
                                    break 'focus;
                                }
                            }
                        }
                    }
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                // End text selection
                self.text_select = None;

                if let Some(drag) = self.drag.take() {
                    if drag.dragging {
                        // Drop: check if mouse is over the project bar
                        let x = self.mouse_x;
                        let y = self.mouse_y;
                        let (cell_w, cell_h) = self.renderer.cell_size();
                        let project_bar_h = (cell_h * 1.5).round();

                        if (y as f32) < project_bar_h {
                            let viewport_w = self.surface_config.width as f64;
                            let max_proj_w = cell_w as f64 * 20.0;
                            let has_filter_tabs = self.projects.len() >= 2;
                            let has_claude = self.has_any_claude_pane();
                            let filter_count: usize = if has_filter_tabs {
                                if has_claude { 3 } else { 1 }
                            } else {
                                0
                            };
                            let slot_count = filter_count + self.projects.len() + 1;
                            let proj_width = (viewport_w / slot_count as f64).min(max_proj_w);
                            let clicked = (x / proj_width) as usize;
                            // Filter slots are not valid drop targets
                            let target_proj = if clicked >= filter_count { clicked - filter_count } else { usize::MAX };

                            if target_proj < self.projects.len() && target_proj != drag.src_project {
                                // Move tab from src to target project
                                let src = &mut self.projects[drag.src_project];
                                let tab = src.tabs.remove(drag.src_tab);
                                src.active_tab = src.active_tab.min(src.tabs.len().saturating_sub(1));

                                // If source project is now empty, remove it
                                if src.tabs.is_empty() {
                                    self.projects.remove(drag.src_project);
                                    if self.projects.len() < 2 {
                                        self.view_mode = ViewMode::Project;
                                    }
                                    // Adjust target index if source was before target
                                    let adjusted_target = if drag.src_project < target_proj {
                                        target_proj - 1
                                    } else {
                                        target_proj
                                    };
                                    let dst = &mut self.projects[adjusted_target];
                                    dst.tabs.push(tab);
                                    dst.active_tab = dst.tabs.len() - 1;
                                    self.active_project = adjusted_target;
                                } else {
                                    let dst = &mut self.projects[target_proj];
                                    dst.tabs.push(tab);
                                    dst.active_tab = dst.tabs.len() - 1;
                                    self.active_project = target_proj;
                                }
                                self.resize_all_panes();
                            }
                        }
                    }
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                // Right-click on project bar → rename project
                let x = self.mouse_x;
                let y = self.mouse_y;
                let (cell_w, cell_h) = self.renderer.cell_size();
                let project_bar_h = (cell_h * 1.5).round();

                if (y as f32) < project_bar_h {
                    // Right-click on project bar → rename project
                    let viewport_w = self.surface_config.width as f64;
                    let max_proj_w = cell_w as f64 * 20.0;
                    let has_filter_tabs = self.projects.len() >= 2;
                    let has_claude = self.has_any_claude_pane();
                    let filter_count: usize = if has_filter_tabs {
                        if has_claude { 3 } else { 1 }
                    } else {
                        0
                    };
                    let slot_count = filter_count + self.projects.len() + 1;
                    let proj_width = (viewport_w / slot_count as f64).min(max_proj_w);
                    let clicked = (x / proj_width) as usize;
                    if clicked >= filter_count && clicked < filter_count + self.projects.len() {
                        let idx = clicked - filter_count;
                        let current = self.projects[idx].name();
                        self.rename_project = Some(RenameProjectState {
                            project_idx: idx,
                            input: current,
                        });
                    }
                } else {
                    // Right-click in pane area → context menu
                    if let Some(menu) = &self.context_menu {
                        // If menu is already open, check if we clicked an item
                        let mx = x as f32;
                        let my = y as f32;
                        let (cw, ch) = self.renderer.cell_size();
                        let item_w = cw * 12.0;
                        let item_h = ch * 1.8;
                        let mut action = None;
                        if mx >= menu.x && mx < menu.x + item_w {
                            let rel_y = my - menu.y;
                            if rel_y >= 0.0 && rel_y < item_h && menu.has_selection {
                                action = Some(ContextMenuItem::Copy);
                            } else if rel_y >= item_h && rel_y < item_h * 2.0 {
                                action = Some(ContextMenuItem::Paste);
                            }
                        }
                        self.do_context_action(action);
                    } else {
                        let has_sel = self.focused_pane()
                            .map(|p| p.terminal.read().selection.is_some())
                            .unwrap_or(false);
                        self.context_menu = Some(ContextMenu {
                            x: x as f32,
                            y: y as f32,
                            hovered: None,
                            has_selection: has_sel,
                        });
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(pane) = self.focused_pane() {
                    let lines = match delta {
                        MouseScrollDelta::LineDelta(_, y) => {
                            (*y * self.config.terminal.scroll_sensitivity as f32) as i32
                        }
                        MouseScrollDelta::PixelDelta(pos) => {
                            let (_, cell_h) = self.renderer.cell_size();
                            (pos.y as f32 / cell_h * self.config.terminal.scroll_sensitivity as f32)
                                as i32
                        }
                    };
                    if lines != 0 {
                        let mut term = pane.terminal.write();
                        term.scroll(lines);
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                // Rendering is handled in tick()
            }

            _ => {}
        }

        WindowAction::None
    }

    /// Whether the current view mode shows a grid of all/filtered tabs
    /// (as opposed to a single project's tabs).
    fn is_grid_view(&self) -> bool {
        self.view_mode != ViewMode::Project
    }

    /// Collect visible tabs based on the current view mode.
    /// Returns tabs with their active status.
    fn visible_tabs(&self) -> Vec<(&Tab, bool)> {
        let active_proj = &self.projects[self.active_project];
        let active_tab_id = active_proj.tabs.get(active_proj.active_tab).map(|t| t.id);

        match self.view_mode {
            ViewMode::Project => {
                let proj = &self.projects[self.active_project];
                proj.tabs.iter().enumerate().map(|(i, t)| {
                    (t, i == proj.active_tab)
                }).collect()
            }
            ViewMode::All => {
                self.projects.iter().flat_map(|p| {
                    p.tabs.iter().map(move |t| (t, Some(t.id) == active_tab_id))
                }).collect()
            }
            ViewMode::Claude => {
                self.projects.iter().flat_map(|p| {
                    p.tabs.iter().filter(|t| t.tree.any_pane(&|pane| pane.is_claude()))
                        .map(move |t| (t, Some(t.id) == active_tab_id))
                }).collect()
            }
            ViewMode::Terminal => {
                self.projects.iter().flat_map(|p| {
                    p.tabs.iter().filter(|t| t.tree.any_pane(&|pane| !pane.is_claude()))
                        .map(move |t| (t, Some(t.id) == active_tab_id))
                }).collect()
            }
        }
    }

    /// Check if any pane across all projects is running Claude Code.
    fn has_any_claude_pane(&self) -> bool {
        self.projects.iter().any(|p| {
            p.tabs.iter().any(|t| t.tree.any_pane(&|pane| pane.is_claude()))
        })
    }

    fn focused_pane(&self) -> Option<&Pane> {
        let proj = self.projects.get(self.active_project)?;
        let tab = proj.tabs.get(proj.active_tab)?;
        tab.tree.pane(tab.focused_pane)
    }

    /// Hit-test mouse position (physical coords) against all visible panes.
    /// Returns the pane and its viewport.
    fn pane_at(&self, x: f32, y: f32) -> Option<(&Pane, PaneViewport)> {
        let size = self.window.inner_size();
        let viewport_w = size.width as f32;
        let viewport_h = size.height as f32;
        let (_, cell_h) = self.renderer.cell_size();
        let project_bar_h = (cell_h * 1.5).round();
        let tab_bar_h = if self.is_grid_view() { 0.0 } else { (cell_h * 2.0).round() };
        let global_bar_h = cell_h;
        let bars_h = project_bar_h + tab_bar_h;
        let pane_area_h = viewport_h - bars_h - global_bar_h;
        let local_y = y - bars_h;
        if local_y < 0.0 { return None; }

        let tabs: Vec<&Tab> = self.visible_tabs().into_iter().map(|(t, _)| t).collect();
        let tab_count = tabs.len();
        let (grid_cols, grid_rows) = if tab_count <= 1 {
            (1usize, 1usize)
        } else {
            let ratio = viewport_w / pane_area_h;
            let cols = (((tab_count as f32) * ratio).sqrt()).round() as usize;
            let cols = cols.max(1).min(tab_count);
            let rows = (tab_count + cols - 1) / cols;
            (cols, rows)
        };
        let gap = 2.0_f32;
        let cell_w_grid = (viewport_w - (grid_cols as f32 - 1.0) * gap) / grid_cols as f32;
        let cell_h_grid = (pane_area_h - (grid_rows as f32 - 1.0) * gap) / grid_rows as f32;

        for (tab_i, tab) in tabs.iter().enumerate() {
            let col = tab_i % grid_cols;
            let row = tab_i / grid_cols;
            let cx = col as f32 * (cell_w_grid + gap);
            let cy = row as f32 * (cell_h_grid + gap);
            let tab_vp = PaneViewport { x: cx, y: cy, width: cell_w_grid, height: cell_h_grid };
            if let Some((pane, vp)) = tab.tree.hit_test(x, local_y, tab_vp) {
                return Some((pane, vp));
            }
        }
        None
    }

    /// Convert logical mouse position to terminal grid (col, abs_line) for a pane.
    fn mouse_to_grid(&self, x: f32, y: f32, vp: &PaneViewport, pane: &Pane) -> (u16, usize) {
        let (cell_w, cell_h) = self.renderer.cell_size();
        let bars_h = {
            let project_bar_h = (cell_h * 1.5).round();
            let tab_bar_h = if self.is_grid_view() { 0.0 } else { (cell_h * 2.0).round() };
            project_bar_h + tab_bar_h
        };
        let ox = vp.x + crate::renderer::PANE_H_PADDING;
        let oy = vp.y + bars_h;
        let term = pane.terminal.read();
        // Account for y_offset (content pushed down when screen isn't full)
        let y_offset_rows = term.y_offset_rows() as f32;
        let content_height = (term.rows as f32 - y_offset_rows) * cell_h;
        let y_offset = (y_offset_rows * cell_h).min((vp.height - content_height).max(0.0));
        let col = ((x - ox) / cell_w).floor().max(0.0) as u16;
        let row = ((y - oy - y_offset) / cell_h).floor().max(0.0) as usize;
        let col = col.min(term.cols.saturating_sub(1));
        let abs_line = (term.scrollback_len() as i64 - term.scroll_offset() as i64 + row as i64).max(0) as usize;
        log::debug!(
            "mouse_to_grid: y={:.1} bars_h={:.1} vp.y={:.1} oy={:.1} y_offset={:.1} cell_h={:.1} row={} abs_line={} sb_len={} scroll_off={}",
            y, bars_h, vp.y, oy, y_offset, cell_h, row, abs_line, term.scrollback_len(), term.scroll_offset()
        );
        (col, abs_line)
    }

    fn do_context_action(&mut self, item: Option<ContextMenuItem>) {
        self.context_menu = None;
        match item {
            Some(ContextMenuItem::Copy) => {
                if let Some(pane) = self.focused_pane() {
                    let mut term = pane.terminal.write();
                    let text = term.selected_text();
                    if !text.is_empty() {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(&text);
                        }
                        term.clear_selection();
                    }
                }
            }
            Some(ContextMenuItem::Paste) => {
                if let Some(pane) = self.focused_pane() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        if let Ok(text) = clipboard.get_text() {
                            let bracketed = pane.terminal.read().bracketed_paste;
                            if bracketed {
                                pane.pty.write(b"\x1b[200~");
                            }
                            pane.pty.write(text.as_bytes());
                            if bracketed {
                                pane.pty.write(b"\x1b[201~");
                            }
                        }
                    }
                }
            }
            None => {}
        }
    }

    fn resize_all_panes(&mut self) {
        let size = self.window.inner_size();
        let viewport_w = size.width as f32;
        let viewport_h = size.height as f32;

        if viewport_w == 0.0 || viewport_h == 0.0 {
            return;
        }

        let (cell_w, cell_h) = self.renderer.cell_size();
        let project_bar_h = (cell_h * 1.5).round();
        let tab_bar_h = if self.is_grid_view() { 0.0 } else { (cell_h * 2.0).round() };
        let global_bar_h = cell_h;
        let status_bar_h = if self.renderer.status_bar_enabled() {
            cell_h
        } else {
            0.0
        };
        let bars_h = project_bar_h + tab_bar_h;
        let pane_area_h = viewport_h - bars_h - global_bar_h;

        // Collect tabs to resize
        let tabs_to_resize: Vec<&Tab> = self.visible_tabs().into_iter().map(|(t, _)| t).collect();
        if tabs_to_resize.is_empty() {
            return;
        }

        let tab_count = tabs_to_resize.len();
        let (grid_cols, grid_rows) = if tab_count <= 1 {
            (1, 1)
        } else {
            let ratio = viewport_w / pane_area_h;
            let cols = (((tab_count as f32) * ratio).sqrt()).round() as usize;
            let cols = cols.max(1).min(tab_count);
            let rows = (tab_count + cols - 1) / cols;
            (cols, rows)
        };

        let gap = 2.0_f32;
        let cell_w_grid = (viewport_w - (grid_cols as f32 - 1.0) * gap) / grid_cols as f32;
        let cell_h_grid = (pane_area_h - (grid_rows as f32 - 1.0) * gap) / grid_rows as f32;

        for (tab_i, tab) in tabs_to_resize.iter().enumerate() {
            let col = tab_i % grid_cols;
            let row = tab_i / grid_cols;
            let cx = col as f32 * (cell_w_grid + gap);
            let cy = bars_h + row as f32 * (cell_h_grid + gap);

            let tab_vp = PaneViewport {
                x: cx,
                y: cy,
                width: cell_w_grid,
                height: cell_h_grid,
            };

            tab.tree.for_each_pane_with_viewport(tab_vp, &mut |pane, vp| {
                let usable_h = vp.height - status_bar_h;
                let usable_w = vp.width - 2.0 * crate::renderer::PANE_H_PADDING;
                let cols = (usable_w / cell_w).floor().max(1.0) as u16;
                let rows = (usable_h / cell_h).floor().max(1.0) as u16;

                let mut term = pane.terminal.write();
                if term.cols != cols || term.rows != rows {
                    term.resize(cols, rows);
                    drop(term);
                    pane.pty.resize(cols, rows);
                }
            });
        }
    }

    fn do_new_tab(&mut self) {
        let cwd = self.focused_pane().and_then(|p| p.cwd());
        match crate::pane::Tab::new_with_cwd(&self.config, cwd.as_deref()) {
            Ok(tab) => {
                let proj = self.project_mut();
                proj.tabs.push(tab);
                proj.active_tab = proj.tabs.len() - 1;
                self.resize_all_panes();
            }
            Err(e) => log::error!("Failed to create tab: {}", e),
        }
    }

    fn do_close_pane_or_tab(&mut self) {
        let focused = {
            let proj = self.project();
            match proj.tabs.get(proj.active_tab) {
                Some(tab) => tab.focused_pane,
                None => return,
            }
        };
        self.close_pane(focused);
    }

    fn close_pane(&mut self, pane_id: PaneId) {
        let proj = self.project_mut();
        if proj.active_tab >= proj.tabs.len() {
            return;
        }

        let Tab { tree, mut focused_pane, id, custom_title, color, has_bell, scroll_offset_x, virtual_width_override } =
            proj.tabs.remove(proj.active_tab);

        match tree.remove_pane(pane_id) {
            Some(new_tree) => {
                if new_tree.pane(focused_pane).is_none() {
                    focused_pane = new_tree.first_pane().id;
                }
                let proj = self.project_mut();
                proj.tabs.insert(proj.active_tab, Tab {
                    id, tree: new_tree, focused_pane, custom_title, color, has_bell, scroll_offset_x, virtual_width_override,
                });
                self.resize_all_panes();
            }
            None => {
                // Tab has no more panes
                let proj = self.project_mut();
                if proj.tabs.is_empty() {
                    // Remove this project
                    let proj_idx = self.active_project;
                    self.projects.remove(proj_idx);
                    if self.projects.is_empty() {
                        self.closing = true;
                    } else {
                        if self.projects.len() < 2 {
                            self.view_mode = ViewMode::Project;
                        }
                        self.active_project = self.active_project.min(self.projects.len() - 1);
                        self.resize_all_panes();
                    }
                } else {
                    proj.active_tab = proj.active_tab.min(proj.tabs.len() - 1);
                    self.resize_all_panes();
                }
            }
        }
    }

    fn do_split(&mut self, direction: SplitDirection) {
        let proj = self.project_mut();
        if proj.active_tab >= proj.tabs.len() {
            return;
        }
        let Tab { tree, mut focused_pane, id, custom_title, color, has_bell, scroll_offset_x, virtual_width_override } =
            proj.tabs.remove(proj.active_tab);
        let cwd = tree.pane(focused_pane).and_then(|p| p.cwd());
        let new_tree = match Pane::spawn(
            self.config.terminal.columns,
            self.config.terminal.rows,
            &self.config,
            cwd.as_deref(),
        ) {
            Ok(new_pane) => {
                let old_focused = focused_pane;
                focused_pane = new_pane.id;
                tree.with_split(old_focused, new_pane, direction)
            }
            Err(e) => {
                log::error!("Failed to spawn pane: {}", e);
                tree
            }
        };
        let proj = self.project_mut();
        proj.tabs.insert(proj.active_tab, Tab {
            id, tree: new_tree, focused_pane, custom_title, color, has_bell, scroll_offset_x, virtual_width_override,
        });
        self.resize_all_panes();
    }

    fn do_split_root(&mut self, direction: SplitDirection) {
        let proj = self.project_mut();
        if proj.active_tab >= proj.tabs.len() {
            return;
        }
        let Tab { tree, mut focused_pane, id, custom_title, color, has_bell, scroll_offset_x, virtual_width_override } =
            proj.tabs.remove(proj.active_tab);
        let cwd = tree.pane(focused_pane).and_then(|p| p.cwd());
        let mut new_tree = match Pane::spawn(
            self.config.terminal.columns,
            self.config.terminal.rows,
            &self.config,
            cwd.as_deref(),
        ) {
            Ok(new_pane) => {
                focused_pane = new_pane.id;
                match direction {
                    SplitDirection::Horizontal => SplitTree::HSplit {
                        left: Box::new(tree),
                        right: Box::new(SplitTree::Leaf(new_pane)),
                        ratio: 0.5,
                        root: true,
                    },
                    SplitDirection::Vertical => SplitTree::VSplit {
                        top: Box::new(tree),
                        bottom: Box::new(SplitTree::Leaf(new_pane)),
                        ratio: 0.5,
                        root: true,
                    },
                }
            }
            Err(e) => {
                log::error!("Failed to spawn pane: {}", e);
                tree
            }
        };
        new_tree.equalize();
        let proj = self.project_mut();
        proj.tabs.insert(proj.active_tab, Tab {
            id, tree: new_tree, focused_pane, custom_title, color, has_bell, scroll_offset_x, virtual_width_override,
        });
        self.resize_all_panes();
    }

    fn do_switch_tab_relative(&mut self, delta: i32) {
        let proj = self.project_mut();
        if proj.tabs.is_empty() {
            return;
        }
        let len = proj.tabs.len() as i32;
        let new_idx = ((proj.active_tab as i32 + delta) % len + len) % len;
        proj.active_tab = new_idx as usize;
        proj.tabs[new_idx as usize].clear_bell();
        self.resize_all_panes();
    }

    fn do_navigate(&mut self, dir: NavDirection) {
        let proj = match self.projects.get(self.active_project) {
            Some(p) => p,
            None => return,
        };
        let tab = match proj.tabs.get(proj.active_tab) {
            Some(t) => t,
            None => return,
        };

        let size = self.window.inner_size();
        let (_, cell_h) = self.renderer.cell_size();
        let project_bar_h = (cell_h * 1.5).round();
        let tab_bar_h = (cell_h * 2.0).round();
        let global_bar_h = cell_h;
        let bars_h = project_bar_h + tab_bar_h;
        let total_vp = PaneViewport {
            x: 0.0,
            y: bars_h,
            width: size.width as f32,
            height: size.height as f32 - bars_h - global_bar_h,
        };

        if let Some(neighbor) = tab.tree.neighbor(tab.focused_pane, dir, total_vp) {
            self.project_mut().active_tab_mut().unwrap().focused_pane = neighbor;
        }
    }

    fn do_swap_pane(&mut self, dir: NavDirection) {
        let size = self.window.inner_size();
        let (_, cell_h) = self.renderer.cell_size();
        let project_bar_h = (cell_h * 1.5).round();
        let proj = match self.projects.get(self.active_project) {
            Some(p) => p,
            None => return,
        };
        let tab_bar_h = (cell_h * 2.0).round();
        let global_bar_h = cell_h;
        let bars_h = project_bar_h + tab_bar_h;
        let total_vp = PaneViewport {
            x: 0.0,
            y: bars_h,
            width: size.width as f32,
            height: size.height as f32 - bars_h - global_bar_h,
        };

        if let Some(tab) = self.project_mut().active_tab_mut() {
            let focused = tab.focused_pane;
            if let Some(neighbor) = tab.tree.neighbor(focused, dir, total_vp) {
                tab.tree.swap_panes(focused, neighbor);
                self.resize_all_panes();
            }
        }
    }

    fn toggle_filter(&mut self) {
        if self.filter.is_some() {
            self.filter = None;
        } else {
            self.filter = Some(FilterState {
                query: String::new(),
                matches: Vec::new(),
            });
        }
    }

    fn update_filter_matches(&mut self) {
        let query = match self.filter {
            Some(ref f) => f.query.clone(),
            None => return,
        };

        let matches = if let Some(pane) = self.focused_pane() {
            let term = pane.terminal.read();
            term.search_lines(&query)
        } else {
            return;
        };
        if let Some(ref mut f) = self.filter {
            f.matches = matches;
        }
    }
}
