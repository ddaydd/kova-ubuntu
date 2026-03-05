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
use crate::pane::{NavDirection, Pane, PaneId, SplitDirection, SplitTree, Tab};
use crate::renderer::{FilterRenderData, PaneViewport, Renderer};
use crate::session::WindowSession;
use crate::terminal::{FilterMatch, TerminalState};

struct FilterState {
    query: String,
    matches: Vec<FilterMatch>,
}

struct RenameTabState {
    input: String,
}

pub struct KovaWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    tabs: Vec<Tab>,
    active_tab: usize,
    keybindings: Keybindings,
    config: Config,
    last_scale: f64,
    filter: Option<FilterState>,
    rename_tab: Option<RenameTabState>,
    show_help: bool,
    modifiers: winit::event::Modifiers,
    closing: bool,
    /// Current mouse position in physical pixels.
    mouse_x: f64,
    mouse_y: f64,
    /// Git branch poll counter (ticks since last poll).
    git_poll_counter: u32,
    git_poll_interval: u32,
    /// Frames remaining to show the "F1 for help" hint at startup.
    help_hint_frames: u32,
}

impl KovaWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        config: &Config,
        tabs: Vec<Tab>,
        active_tab: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let attrs = WindowAttributes::default()
            .with_title("Kova")
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
            tabs,
            active_tab,
            keybindings,
            config: config.clone(),
            last_scale: scale,
            filter: None,
            rename_tab: None,
            show_help: false,
            modifiers: Default::default(),
            closing: false,
            mouse_x: 0.0,
            mouse_y: 0.0,
            git_poll_counter: 0,
            git_poll_interval: fps * 2,
            help_hint_frames: fps * 3,
        };

        // Initial resize
        win.resize_all_panes();

        Ok(win)
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
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
        WindowSession::from_tabs(&self.tabs, self.active_tab, frame)
    }

    /// Per-frame tick. Returns false if window should close.
    pub fn tick(&mut self) -> bool {
        if self.closing {
            return false;
        }

        // Reap dead panes
        let mut dead_ids = Vec::new();
        if let Some(tab) = self.tabs.get(self.active_tab) {
            dead_ids = tab.tree.exited_pane_ids();
        }
        for id in dead_ids {
            self.close_pane(id);
        }

        // Inject pending commands
        for tab in &self.tabs {
            tab.tree.for_each_pane(&mut |pane| {
                pane.inject_pending_command();
            });
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
        for tab in &mut self.tabs {
            tab.check_bell();
        }

        // Help hint countdown
        if self.help_hint_frames > 0 {
            self.help_hint_frames -= 1;
        }

        // Render
        self.render();

        !self.tabs.is_empty()
    }

    fn render(&mut self) {
        let tab = match self.tabs.get(self.active_tab) {
            Some(t) => t,
            None => return,
        };

        let size = self.window.inner_size();
        let scale = self.window.scale_factor() as f32;
        let viewport_w = size.width as f32;
        let viewport_h = size.height as f32;

        if viewport_w == 0.0 || viewport_h == 0.0 {
            return;
        }

        let (cell_w, cell_h) = self.renderer.cell_size();
        let tab_bar_h = if self.tabs.len() > 1 { (cell_h * 2.0).round() } else { 0.0 };
        let global_bar_h = cell_h;
        let pane_area_y = tab_bar_h;
        let pane_area_h = viewport_h - tab_bar_h - global_bar_h;

        let total_vp = PaneViewport {
            x: 0.0,
            y: pane_area_y,
            width: viewport_w,
            height: pane_area_h,
        };

        // Collect pane data for rendering
        let mut panes = Vec::new();
        tab.tree.for_each_pane_with_viewport(total_vp, &mut |pane, vp| {
            panes.push((
                pane.terminal.clone(),
                vp,
                pane.is_ready(),
                pane.id == tab.focused_pane,
                pane.id,
                pane.custom_title.clone(),
            ));
        });

        // Collect separators
        let mut separators = Vec::new();
        tab.tree.collect_separators(total_vp, &mut separators);

        // Tab titles
        let tab_titles: Vec<(String, bool, Option<usize>, bool, bool)> = if self.tabs.len() > 1 {
            self.tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let is_renaming = i == self.active_tab && self.rename_tab.is_some();
                    let title = if is_renaming {
                        self.rename_tab.as_ref().unwrap().input.clone()
                    } else {
                        t.title()
                    };
                    (title, i == self.active_tab, t.color, is_renaming, t.has_bell)
                })
                .collect()
        } else {
            Vec::new()
        };

        let filter_data = self.filter.as_ref().map(|f| FilterRenderData {
            query: f.query.clone(),
            matches: f.matches.clone(),
        });

        self.renderer.render_panes(
            &self.device,
            &self.queue,
            &self.surface,
            &panes,
            &separators,
            &tab_titles,
            filter_data.as_ref(),
            self.show_help,
            self.help_hint_frames,
            0.0, // no traffic light inset on Linux
            0,
            0,
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

                // Handle rename tab mode
                if let Some(ref mut rename) = self.rename_tab {
                    match &event.logical_key {
                        Key::Named(NamedKey::Enter) => {
                            let new_title = rename.input.clone();
                            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
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
                log::debug!("Key event: {:?} -> combo: {:?}", event.logical_key, combo);

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
                            if idx < self.tabs.len() {
                                self.active_tab = idx;
                                self.tabs[idx].clear_bell();
                                self.resize_all_panes();
                            }
                        }
                        Action::Navigate(dir) => self.do_navigate(dir),
                        Action::SwapPane(dir) => self.do_swap_pane(dir),
                        Action::Resize(axis, delta) => {
                            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
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
                            let current = self.tabs.get(self.active_tab)
                                .and_then(|t| t.custom_title.clone())
                                .unwrap_or_default();
                            self.rename_tab = Some(RenameTabState { input: current });
                        }
                        Action::RenamePane => {} // TODO
                        Action::DetachTab => {}  // TODO
                        Action::MergeWindow => {} // TODO
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
                    );
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_x = position.x;
                self.mouse_y = position.y;
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let scale = self.window.scale_factor();
                let x = self.mouse_x / scale;
                let y = self.mouse_y / scale;
                let (_, cell_h) = self.renderer.cell_size();
                let bar_h = (cell_h * 2.0) as f64;

                if y < bar_h && !self.tabs.is_empty() {
                    // Click in tab bar — switch tab
                    let cell_w = self.renderer.cell_size().0 as f64;
                    let viewport_w = self.surface_config.width as f64 / scale;
                    let max_tab_w = cell_w * 20.0;
                    let tab_width = (viewport_w / self.tabs.len() as f64).min(max_tab_w);
                    let clicked_tab = (x / tab_width) as usize;
                    if clicked_tab < self.tabs.len() {
                        self.active_tab = clicked_tab;
                        self.resize_all_panes();
                    }
                }
                // TODO: implement mouse selection, separator drag
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
                        term.scroll(-lines);
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

    fn focused_pane(&self) -> Option<&Pane> {
        self.tabs
            .get(self.active_tab)
            .and_then(|tab| tab.tree.pane(tab.focused_pane))
    }

    fn resize_all_panes(&mut self) {
        let size = self.window.inner_size();
        let scale = self.window.scale_factor() as f32;
        let viewport_w = size.width as f32;
        let viewport_h = size.height as f32;

        if viewport_w == 0.0 || viewport_h == 0.0 {
            return;
        }

        let (cell_w, cell_h) = self.renderer.cell_size();
        let tab_bar_h = if self.tabs.len() > 1 {
            (cell_h * 2.0).round()
        } else {
            0.0
        };
        let global_bar_h = cell_h;
        let status_bar_h = if self.renderer.status_bar_enabled() {
            cell_h
        } else {
            0.0
        };
        let pane_area_y = tab_bar_h;
        let pane_area_h = viewport_h - tab_bar_h - global_bar_h;

        let total_vp = PaneViewport {
            x: 0.0,
            y: pane_area_y,
            width: viewport_w,
            height: pane_area_h,
        };

        if let Some(tab) = self.tabs.get(self.active_tab) {
            tab.tree
                .for_each_pane_with_viewport(total_vp, &mut |pane, vp| {
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
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
                self.resize_all_panes();
            }
            Err(e) => log::error!("Failed to create tab: {}", e),
        }
    }

    fn do_close_pane_or_tab(&mut self) {
        if let Some(tab) = self.tabs.get(self.active_tab) {
            let focused = tab.focused_pane;
            self.close_pane(focused);
        }
    }

    fn close_pane(&mut self, pane_id: PaneId) {
        if self.active_tab >= self.tabs.len() {
            return;
        }

        let Tab { tree, mut focused_pane, id, custom_title, color, has_bell, scroll_offset_x, virtual_width_override } =
            self.tabs.remove(self.active_tab);

        match tree.remove_pane(pane_id) {
            Some(new_tree) => {
                if new_tree.pane(focused_pane).is_none() {
                    focused_pane = new_tree.first_pane().id;
                }
                self.tabs.insert(self.active_tab, Tab {
                    id, tree: new_tree, focused_pane, custom_title, color, has_bell, scroll_offset_x, virtual_width_override,
                });
                self.resize_all_panes();
            }
            None => {
                // Tab has no more panes
                if self.tabs.is_empty() {
                    self.closing = true;
                } else {
                    self.active_tab = self.active_tab.min(self.tabs.len() - 1);
                    self.resize_all_panes();
                }
            }
        }
    }

    fn do_split(&mut self, direction: SplitDirection) {
        if self.active_tab >= self.tabs.len() {
            return;
        }
        let Tab { tree, mut focused_pane, id, custom_title, color, has_bell, scroll_offset_x, virtual_width_override } =
            self.tabs.remove(self.active_tab);
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
        self.tabs.insert(self.active_tab, Tab {
            id, tree: new_tree, focused_pane, custom_title, color, has_bell, scroll_offset_x, virtual_width_override,
        });
        self.resize_all_panes();
    }

    fn do_split_root(&mut self, direction: SplitDirection) {
        if self.active_tab >= self.tabs.len() {
            return;
        }
        let Tab { tree, mut focused_pane, id, custom_title, color, has_bell, scroll_offset_x, virtual_width_override } =
            self.tabs.remove(self.active_tab);
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
        self.tabs.insert(self.active_tab, Tab {
            id, tree: new_tree, focused_pane, custom_title, color, has_bell, scroll_offset_x, virtual_width_override,
        });
        self.resize_all_panes();
    }

    fn do_switch_tab_relative(&mut self, delta: i32) {
        if self.tabs.is_empty() {
            return;
        }
        let len = self.tabs.len() as i32;
        let new_idx = ((self.active_tab as i32 + delta) % len + len) % len;
        self.active_tab = new_idx as usize;
        self.tabs[self.active_tab].clear_bell();
        self.resize_all_panes();
    }

    fn do_navigate(&mut self, dir: NavDirection) {
        let tab = match self.tabs.get(self.active_tab) {
            Some(t) => t,
            None => return,
        };

        let size = self.window.inner_size();
        let (cell_w, cell_h) = self.renderer.cell_size();
        let tab_bar_h = if self.tabs.len() > 1 { (cell_h * 2.0).round() } else { 0.0 };
        let global_bar_h = cell_h;
        let total_vp = PaneViewport {
            x: 0.0,
            y: tab_bar_h,
            width: size.width as f32,
            height: size.height as f32 - tab_bar_h - global_bar_h,
        };

        if let Some(neighbor) = tab.tree.neighbor(tab.focused_pane, dir, total_vp) {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                tab.focused_pane = neighbor;
            }
        }
    }

    fn do_swap_pane(&mut self, dir: NavDirection) {
        let size = self.window.inner_size();
        let (cell_w, cell_h) = self.renderer.cell_size();
        let tab_bar_h = if self.tabs.len() > 1 { (cell_h * 2.0).round() } else { 0.0 };
        let global_bar_h = cell_h;
        let total_vp = PaneViewport {
            x: 0.0,
            y: tab_bar_h,
            width: size.width as f32,
            height: size.height as f32 - tab_bar_h - global_bar_h,
        };

        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
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
