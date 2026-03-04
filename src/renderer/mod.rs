pub mod glyph_atlas;
pub mod pipeline;
pub mod vertex;

pub const PANE_H_PADDING: f32 = 10.0;

/// Predefined tab color palette.
pub const TAB_COLORS: [[f32; 3]; 6] = [
    [0.82, 0.22, 0.22],
    [0.90, 0.55, 0.15],
    [0.85, 0.75, 0.15],
    [0.30, 0.70, 0.30],
    [0.25, 0.50, 0.85],
    [0.60, 0.35, 0.75],
];

use glyph_atlas::GlyphAtlas;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::SystemTime;
use vertex::Vertex;

use crate::config::Config;
use crate::pane::PaneId;
use crate::terminal::{CursorShape, FilterMatch, TerminalState};

/// Data passed to the renderer for drawing filter overlay.
pub struct FilterRenderData {
    pub query: String,
    pub matches: Vec<FilterMatch>,
}

/// Sub-region of the drawable where a pane is rendered (in pixels).
#[derive(Clone, Copy)]
pub struct PaneViewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

const MAX_VERTICES: usize = 16 * 1024 * 1024 / std::mem::size_of::<Vertex>();

/// Uniform buffer layout matching the WGSL struct.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    viewport_size: [f32; 2],
    atlas_size: [f32; 2],
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
    vertex_bufs: [wgpu::Buffer; 2],
    vertex_buf_idx: usize,
    atlas_texture: wgpu::Texture,
    atlas_sampler: wgpu::Sampler,
    atlas: GlyphAtlas,
    last_viewport: [f32; 2],
    blink_counter: u32,
    last_cursor_epoch: u32,
    bg_color: [f32; 3],
    cursor_color: [f32; 3],
    font_size: f64,
    font_name: String,
    cursor_blink_frames: u32,
    status_bar_enabled: bool,
    status_bar_bg: [f32; 3],
    status_bar_fg: [f32; 3],
    status_bar_cwd_color: [f32; 3],
    status_bar_branch_color: [f32; 3],
    status_bar_scroll_color: [f32; 3],
    global_bar_bg: [f32; 3],
    global_bar_time_color: [f32; 3],
    global_bar_scroll_color: [f32; 3],
    last_minute: u32,
    cached_time_str: String,
    selection_color: [f32; 3],
    tab_bar_bg: [f32; 3],
    tab_bar_fg: [f32; 3],
    tab_bar_active_bg: [f32; 3],
    pub hovered_url: Option<(usize, u16, u16)>,
    pub hovered_url_text: Option<String>,
    pub hovered_url_pane_id: Option<PaneId>,
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        scale: f64,
        config: &Config,
    ) -> Self {
        log::info!("Renderer::new: font_size={}, scale={}, effective_px={}", config.font.size, scale, config.font.size * scale);
        let atlas = GlyphAtlas::new(config.font.size * scale, &config.font.family);

        // Create atlas texture
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas_texture"),
            size: wgpu::Extent3d {
                width: atlas.atlas_width,
                height: atlas.atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload initial atlas data
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.atlas_buf,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.atlas_width * 4),
                rows_per_image: Some(atlas.atlas_height),
            },
            wgpu::Extent3d {
                width: atlas.atlas_width,
                height: atlas.atlas_height,
                depth_or_array_layers: 1,
            },
        );

        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let uniforms = Uniforms {
            viewport_size: [0.0, 0.0],
            atlas_size: [atlas.atlas_width as f32, atlas.atlas_height as f32],
        };

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform_buf"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terminal_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let atlas_view = atlas_texture.create_view(&Default::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terminal_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        let pipeline = pipeline::create_pipeline(device, surface_format, &bind_group_layout);

        let make_vertex_buf = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vertex_buf"),
                size: (MAX_VERTICES * std::mem::size_of::<Vertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };

        Renderer {
            pipeline,
            bind_group_layout,
            bind_group,
            uniform_buf,
            vertex_bufs: [make_vertex_buf(), make_vertex_buf()],
            vertex_buf_idx: 0,
            atlas_texture,
            atlas_sampler,
            atlas,
            last_viewport: [0.0; 2],
            blink_counter: 0,
            last_cursor_epoch: 0,
            bg_color: config.colors.background,
            cursor_color: config.colors.cursor,
            font_size: config.font.size,
            font_name: config.font.family.clone(),
            cursor_blink_frames: config.terminal.cursor_blink_frames,
            status_bar_enabled: config.status_bar.enabled,
            status_bar_bg: config.status_bar.bg_color,
            status_bar_fg: config.status_bar.fg_color,
            status_bar_cwd_color: config.status_bar.cwd_color,
            status_bar_branch_color: config.status_bar.branch_color,
            status_bar_scroll_color: config.status_bar.scroll_color,
            global_bar_bg: config.global_status_bar.bg_color,
            global_bar_time_color: config.global_status_bar.time_color,
            global_bar_scroll_color: config.global_status_bar.scroll_indicator_color,
            last_minute: u32::MAX,
            cached_time_str: String::new(),
            selection_color: [0.45, 0.42, 0.20],
            tab_bar_bg: config.tab_bar.bg_color,
            tab_bar_fg: config.tab_bar.fg_color,
            tab_bar_active_bg: config.tab_bar.active_bg,
            hovered_url: None,
            hovered_url_text: None,
            hovered_url_pane_id: None,
        }
    }

    /// Recreate atlas texture + bind group after atlas grows or DPI change.
    fn recreate_atlas_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas_texture"),
            size: wgpu::Extent3d {
                width: self.atlas.atlas_width,
                height: self.atlas.atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.atlas.atlas_buf,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.atlas.atlas_width * 4),
                rows_per_image: Some(self.atlas.atlas_height),
            },
            wgpu::Extent3d {
                width: self.atlas.atlas_width,
                height: self.atlas.atlas_height,
                depth_or_array_layers: 1,
            },
        );

        let atlas_view = self.atlas_texture.create_view(&Default::default());
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terminal_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                },
            ],
        });

        self.atlas.texture_dirty = false;
    }

    pub fn render_panes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &wgpu::Surface,
        panes: &[(Arc<RwLock<TerminalState>>, PaneViewport, bool, bool, PaneId, Option<String>)],
        separators: &[(f32, f32, f32, f32)],
        tab_titles: &[(String, bool, Option<usize>, bool, bool)],
        filter: Option<&FilterRenderData>,
        tab_bar_left_inset: f32,
        hidden_left: usize,
        hidden_right: usize,
    ) {
        // Reset blink on cursor movement
        if let Some((term, _, _, _, _, _)) = panes.iter().find(|(_, _, _, focused, _, _)| *focused) {
            let epoch = term.read().cursor_move_epoch.load(std::sync::atomic::Ordering::Relaxed);
            if epoch != self.last_cursor_epoch {
                self.last_cursor_epoch = epoch;
                self.blink_counter = 0;
            }
        }

        self.blink_counter = self.blink_counter.wrapping_add(1);
        let (blink_on, blink_changed) = if self.cursor_blink_frames >= 2 {
            let half = self.cursor_blink_frames / 2;
            (
                self.blink_counter % self.cursor_blink_frames < half,
                (self.blink_counter % half) == 0,
            )
        } else {
            (true, false)
        };

        // Check minute change for time display
        let minute_changed = {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            let current_minute = (now.as_secs() / 60) as u32;
            if current_minute != self.last_minute {
                self.last_minute = current_minute;
                let secs = now.as_secs();
                // Compute local time via libc
                let t = secs as libc::time_t;
                let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                unsafe { libc::localtime_r(&t, &mut tm) };
                self.cached_time_str = format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
                true
            } else {
                false
            }
        };

        // Check dirty flags
        let mut any_dirty = false;
        let mut any_not_ready = false;
        let mut any_sync_deferred = false;
        for (term, _, ready, _, _, _) in panes {
            if !ready { any_not_ready = true; }
            let t = term.read();
            if t.synchronized_output {
                if let Some(since) = t.sync_output_since {
                    if since.elapsed().as_millis() < 100 {
                        any_sync_deferred = true;
                        continue;
                    }
                }
            }
            drop(t);
            if term.read().dirty.swap(false, std::sync::atomic::Ordering::Relaxed) {
                any_dirty = true;
            }
        }
        let all_ready = !any_not_ready;
        let has_filter = filter.is_some();
        if all_ready && !any_dirty && !any_sync_deferred && !blink_changed && !minute_changed && !has_filter {
            return;
        }

        let output = match surface.get_current_texture() {
            Ok(o) => o,
            Err(e) => {
                log::warn!("Failed to get surface texture: {}", e);
                return;
            }
        };

        let view = output.texture.create_view(&Default::default());
        let viewport_w = output.texture.width() as f32;
        let viewport_h = output.texture.height() as f32;

        // Build vertices
        let mut all_vertices = Vec::new();
        let saved_hover_text = self.hovered_url_text.clone();
        let saved_hover_pos = self.hovered_url;
        for (term, vp, shell_ready, is_focused, pane_id, custom_title) in panes {
            let is_hover_pane = self.hovered_url_pane_id == Some(*pane_id);
            self.hovered_url_text = if is_hover_pane { saved_hover_text.clone() } else { None };
            self.hovered_url = if is_hover_pane { saved_hover_pos } else { None };
            if vp.x + vp.width <= 0.0 || vp.x >= viewport_w { continue; }
            if *is_focused && filter.is_some() { continue; }
            if *shell_ready {
                let t = term.read();
                let show_blink = if *is_focused { blink_on } else { true };
                let mut verts = self.build_vertices(&t, vp, show_blink, *is_focused, custom_title.as_deref());
                all_vertices.append(&mut verts);
            } else {
                let mut verts = self.build_loading_vertices(vp);
                all_vertices.append(&mut verts);
            }
        }
        self.hovered_url_text = saved_hover_text;
        self.hovered_url = saved_hover_pos;

        // Separators
        if !separators.is_empty() {
            let no_tex = [0.0_f32, 0.0];
            let white = [1.0_f32, 1.0, 1.0, 0.0];
            let sep_bg = [1.0_f32, 1.0, 1.0, 0.15];
            let thickness = 1.0_f32;
            for &(x1, y1, x2, y2) in separators {
                if (x1 - x2).abs() < 0.1 {
                    let lx = x1 - thickness * 0.5;
                    let rx = x1 + thickness * 0.5;
                    all_vertices.push(Vertex { position: [lx, y1], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [rx, y1], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [lx, y2], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [rx, y1], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [rx, y2], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [lx, y2], tex_coords: no_tex, color: white, bg_color: sep_bg });
                } else {
                    let ty = y1 - thickness * 0.5;
                    let by = y1 + thickness * 0.5;
                    all_vertices.push(Vertex { position: [x1, ty], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [x2, ty], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [x1, by], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [x2, ty], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [x2, by], tex_coords: no_tex, color: white, bg_color: sep_bg });
                    all_vertices.push(Vertex { position: [x1, by], tex_coords: no_tex, color: white, bg_color: sep_bg });
                }
            }
        }

        // Tab bar
        if !tab_titles.is_empty() {
            self.build_tab_bar_vertices(&mut all_vertices, viewport_w, tab_titles, tab_bar_left_inset);
        }

        // Global status bar
        self.build_global_status_bar_vertices(&mut all_vertices, viewport_w, viewport_h, hidden_left, hidden_right);

        // Filter overlay
        if let Some(filter_data) = filter {
            if let Some((_, vp, _, _, _, _)) = panes.iter().find(|(_, _, _, focused, _, _)| *focused) {
                self.build_filter_overlay_vertices(&mut all_vertices, vp, filter_data);
            }
        }

        // Update uniforms
        let viewport = [viewport_w, viewport_h];
        if viewport != self.last_viewport || self.atlas.texture_dirty {
            self.last_viewport = viewport;
            let uniforms = Uniforms {
                viewport_size: viewport,
                atlas_size: [self.atlas.atlas_width as f32, self.atlas.atlas_height as f32],
            };
            queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));
        }

        // Re-upload atlas if dirty
        if self.atlas.texture_dirty {
            self.recreate_atlas_texture(device, queue);
        }

        // Encode render pass
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terminal_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.bg_color[0] as f64,
                            g: self.bg_color[1] as f64,
                            b: self.bg_color[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            if !all_vertices.is_empty() {
                let vertex_data: &[u8] = bytemuck::cast_slice(&all_vertices);

                if all_vertices.len() > MAX_VERTICES {
                    log::error!("Too many vertices ({}), skipping frame", all_vertices.len());
                } else {
                    let buf_idx = self.vertex_buf_idx;
                    self.vertex_buf_idx = 1 - buf_idx;
                    queue.write_buffer(&self.vertex_bufs[buf_idx], 0, vertex_data);

                    render_pass.set_pipeline(&self.pipeline);
                    render_pass.set_bind_group(0, &self.bind_group, &[]);
                    render_pass.set_vertex_buffer(0, self.vertex_bufs[buf_idx].slice(..));
                    render_pass.draw(0..all_vertices.len() as u32, 0..1);
                }
            }
        }

        queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    fn build_vertices(
        &mut self,
        term: &TerminalState,
        vp: &PaneViewport,
        blink_on: bool,
        is_focused: bool,
        custom_title: Option<&str>,
    ) -> Vec<Vertex> {
        let display = term.visible_lines();
        let mut unknown_chars: Vec<char> = Vec::new();
        let mut unknown_clusters: Vec<Box<str>> = Vec::new();
        {
            let mut seen_chars = std::collections::HashSet::new();
            let mut seen_clusters = std::collections::HashSet::new();
            for line in display.iter() {
                for cell in line.iter() {
                    if let Some(ref cluster) = cell.cluster {
                        if self.atlas.cluster_glyph(cluster).is_none() && seen_clusters.insert(cluster.clone()) {
                            unknown_clusters.push(cluster.clone());
                        }
                    } else {
                        let c = cell.c;
                        if c != ' ' && c != '\0' && self.atlas.glyph(c).is_none() && seen_chars.insert(c) {
                            unknown_chars.push(c);
                        }
                    }
                }
            }
        }

        for c in unknown_chars { self.atlas.rasterize_char(c); }
        for cluster in unknown_clusters { self.atlas.rasterize_cluster(&cluster); }

        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let atlas_w = self.atlas.atlas_width as f32;
        let atlas_h = self.atlas.atlas_height as f32;
        let ox = vp.x + PANE_H_PADDING;
        let oy = vp.y;

        let y_offset_rows = term.y_offset_rows() as f32;
        let content_height = (term.rows as f32 - y_offset_rows) * cell_h;
        let y_offset = (y_offset_rows * cell_h).min((vp.height - content_height).max(0.0));

        let mut vertices = Vec::with_capacity(display.len() * term.cols as usize * 6);

        let has_selection = term.selection.is_some();
        let abs_line_base = if has_selection {
            term.scrollback_len() as i64 - term.scroll_offset() as i64
        } else {
            0
        };

        // Pass 1: backgrounds + selection
        for (row_idx, line) in display.iter().enumerate() {
            let abs_line = (abs_line_base + row_idx as i64) as usize;
            let y = (oy + y_offset + row_idx as f32 * cell_h).round();

            for col_idx in 0..term.cols as usize {
                let x = (ox + col_idx as f32 * cell_w).round();
                if col_idx < line.len() && line[col_idx].bg != self.bg_color {
                    Self::push_bg_quad(&mut vertices, x, y, cell_w, cell_h, line[col_idx].bg);
                }
                if has_selection && term.is_selected(abs_line, col_idx as u16) {
                    Self::push_bg_quad(&mut vertices, x, y, cell_w, cell_h, self.selection_color);
                }
            }
        }

        // Pass 2: glyphs
        for (row_idx, line) in display.iter().enumerate() {
            for col_idx in 0..term.cols as usize {
                let cell = if col_idx < line.len() { &line[col_idx] } else { continue };
                if cell.is_blank() { continue; }

                let glyph = if let Some(ref cluster) = cell.cluster {
                    match self.atlas.cluster_glyph(cluster) { Some(g) => *g, None => continue }
                } else {
                    match self.atlas.glyph(cell.c) { Some(g) => *g, None => continue }
                };

                if glyph.width == 0 || glyph.height == 0 { continue; }

                let gx = (ox + col_idx as f32 * cell_w).round();
                let gy = (oy + y_offset + row_idx as f32 * cell_h).round();
                let gw = glyph.width as f32;
                let gh = glyph.height as f32;

                let tx = glyph.x as f32 / atlas_w;
                let ty = glyph.y as f32 / atlas_h;
                let tw = glyph.width as f32 / atlas_w;
                let th = glyph.height as f32 / atlas_h;

                let alpha = if glyph.is_color { 2.0 } else { 1.0 };
                let fg = [cell.fg[0], cell.fg[1], cell.fg[2], alpha];
                let no_bg = [0.0, 0.0, 0.0, 0.0];

                vertices.push(Vertex { position: [gx, gy], tex_coords: [tx, ty], color: fg, bg_color: no_bg });
                vertices.push(Vertex { position: [gx + gw, gy], tex_coords: [tx + tw, ty], color: fg, bg_color: no_bg });
                vertices.push(Vertex { position: [gx, gy + gh], tex_coords: [tx, ty + th], color: fg, bg_color: no_bg });
                vertices.push(Vertex { position: [gx + gw, gy], tex_coords: [tx + tw, ty], color: fg, bg_color: no_bg });
                vertices.push(Vertex { position: [gx + gw, gy + gh], tex_coords: [tx + tw, ty + th], color: fg, bg_color: no_bg });
                vertices.push(Vertex { position: [gx, gy + gh], tex_coords: [tx, ty + th], color: fg, bg_color: no_bg });
            }
        }

        // URL underline
        if let Some((hover_row, col_start, col_end)) = self.hovered_url {
            let uy = (oy + y_offset + hover_row as f32 * cell_h + cell_h - 1.0).round();
            let ux = (ox + col_start as f32 * cell_w).round();
            let uw = (col_end - col_start) as f32 * cell_w;
            Self::push_bg_quad(&mut vertices, ux, uy, uw, 1.0, [0.4, 0.6, 1.0]);
        }

        // Cursor
        if term.cursor_visible && blink_on {
            let offset = term.scroll_offset();
            let screen_y = offset + term.cursor_y as i32;
            if screen_y >= 0 && screen_y < term.rows as i32 {
                let cx = (ox + term.cursor_x as f32 * cell_w).round();
                let cy = (oy + y_offset + screen_y as f32 * cell_h).round();
                match term.cursor_shape {
                    CursorShape::Block => Self::push_bg_quad(&mut vertices, cx, cy, cell_w, cell_h, self.cursor_color),
                    CursorShape::Underline => {
                        let thickness = (cell_h * 0.1).max(1.0);
                        Self::push_bg_quad(&mut vertices, cx, cy + cell_h - thickness, cell_w, thickness, self.cursor_color);
                    }
                    CursorShape::Bar => {
                        let thickness = (cell_w * 0.1).max(1.0);
                        Self::push_bg_quad(&mut vertices, cx, cy, thickness, cell_h, self.cursor_color);
                    }
                }
            }
        }

        // Dim unfocused
        if !is_focused {
            let dim4 = [0.0, 0.0, 0.0, 0.3];
            let no_tex = [0.0, 0.0];
            let white = [1.0, 1.0, 1.0, 0.0];
            let dim_h = if self.status_bar_enabled { vp.height - self.atlas.cell_height } else { vp.height };
            vertices.push(Vertex { position: [vp.x, vp.y], tex_coords: no_tex, color: white, bg_color: dim4 });
            vertices.push(Vertex { position: [vp.x + vp.width, vp.y], tex_coords: no_tex, color: white, bg_color: dim4 });
            vertices.push(Vertex { position: [vp.x, vp.y + dim_h], tex_coords: no_tex, color: white, bg_color: dim4 });
            vertices.push(Vertex { position: [vp.x + vp.width, vp.y], tex_coords: no_tex, color: white, bg_color: dim4 });
            vertices.push(Vertex { position: [vp.x + vp.width, vp.y + dim_h], tex_coords: no_tex, color: white, bg_color: dim4 });
            vertices.push(Vertex { position: [vp.x, vp.y + dim_h], tex_coords: no_tex, color: white, bg_color: dim4 });
        }

        if self.status_bar_enabled {
            self.build_status_bar_vertices(&mut vertices, vp, term, custom_title);
        }

        vertices
    }

    fn build_status_bar_vertices(&mut self, vertices: &mut Vec<Vertex>, vp: &PaneViewport, term: &TerminalState, custom_title: Option<&str>) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let bar_y = vp.y + vp.height - cell_h;

        Self::push_bg_quad(vertices, vp.x, bar_y, vp.width, cell_h, self.status_bar_bg);

        let no_bg = [0.0, 0.0, 0.0, 0.0];
        let cwd_fg = [self.status_bar_cwd_color[0], self.status_bar_cwd_color[1], self.status_bar_cwd_color[2], 1.0];
        let branch_fg = [self.status_bar_branch_color[0], self.status_bar_branch_color[1], self.status_bar_branch_color[2], 1.0];
        let scroll_fg = [self.status_bar_scroll_color[0], self.status_bar_scroll_color[1], self.status_bar_scroll_color[2], 1.0];
        let title_fg = [self.status_bar_fg[0], self.status_bar_fg[1], self.status_bar_fg[2], 1.0];

        let mut cursor_x = vp.x + PANE_H_PADDING + cell_w;
        if let Some(ref cwd) = term.cwd {
            let home = std::env::var("HOME").unwrap_or_default();
            let display_path = if !home.is_empty() && cwd.starts_with(&home) {
                format!("~{}", &cwd[home.len()..])
            } else {
                cwd.clone()
            };
            cursor_x = self.render_status_text(vertices, &display_path, cursor_x, bar_y, vp.x + vp.width * 0.4, cwd_fg, no_bg);
        }

        cursor_x += cell_w * 2.0;
        let branch_display = match term.git_branch {
            Some(ref b) => format!(" {}", b),
            None => " no git".to_string(),
        };
        let actual_branch_fg = match term.git_branch {
            Some(_) => branch_fg,
            None => [branch_fg[0] * 0.5, branch_fg[1] * 0.5, branch_fg[2] * 0.5, 0.5],
        };
        let left_end = self.render_status_text(vertices, &branch_display, cursor_x, bar_y, vp.x + vp.width * 0.6, actual_branch_fg, no_bg);

        let right_edge = vp.x + vp.width - cell_w;
        let scroll_off = term.scroll_offset();
        let right_after_scroll = if scroll_off > 0 {
            let scroll_str = format!("↑{}", scroll_off);
            let scroll_w = scroll_str.chars().count() as f32 * cell_w;
            let right_x = right_edge - scroll_w;
            self.render_status_text(vertices, &scroll_str, right_x, bar_y, right_edge + cell_w, scroll_fg, no_bg);
            right_x - cell_w * 2.0
        } else {
            right_edge
        };

        let right_text: Option<(String, [f32; 4])> = if let Some(ref url) = self.hovered_url_text {
            Some((url.clone(), [0.4, 0.6, 1.0, 1.0]))
        } else if let Some(title) = custom_title {
            Some((title.to_string(), title_fg))
        } else {
            term.title.as_ref().map(|t| (t.clone(), title_fg))
        };
        if let Some((text, fg)) = right_text {
            let char_count = text.chars().count();
            let text_w = char_count as f32 * cell_w;
            let title_x = right_after_scroll - text_w;
            if title_x >= left_end + cell_w * 2.0 {
                self.render_status_text(vertices, &text, title_x, bar_y, right_after_scroll, fg, no_bg);
            }
        }
    }

    fn build_global_status_bar_vertices(&mut self, vertices: &mut Vec<Vertex>, viewport_w: f32, viewport_h: f32, hidden_left: usize, hidden_right: usize) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let bar_y = viewport_h - cell_h;

        Self::push_bg_quad(vertices, 0.0, bar_y, viewport_w, cell_h, self.global_bar_bg);

        let no_bg = [0.0, 0.0, 0.0, 0.0];
        let time_fg = [self.global_bar_time_color[0], self.global_bar_time_color[1], self.global_bar_time_color[2], 1.0];
        let scroll_fg = [self.global_bar_scroll_color[0], self.global_bar_scroll_color[1], self.global_bar_scroll_color[2], 1.0];

        if hidden_left > 0 || hidden_right > 0 {
            let indicator = match (hidden_left > 0, hidden_right > 0) {
                (true, true) => format!("⟵ {} | {} ⟶", hidden_left, hidden_right),
                (true, false) => format!("⟵ {}", hidden_left),
                (false, true) => format!("{} ⟶", hidden_right),
                _ => unreachable!(),
            };
            let char_count = indicator.chars().count() as f32;
            let text_w = char_count * cell_w;
            let center_x = (viewport_w - text_w) / 2.0;
            self.render_status_text(vertices, &indicator, center_x, bar_y, viewport_w, scroll_fg, no_bg);
        }

        if !self.cached_time_str.is_empty() {
            let time_str = self.cached_time_str.clone();
            let time_w = time_str.chars().count() as f32 * cell_w;
            let right_x = viewport_w - time_w - cell_w;
            self.render_status_text(vertices, &time_str, right_x, bar_y, viewport_w, time_fg, no_bg);
        }
    }

    fn build_tab_bar_vertices(&mut self, vertices: &mut Vec<Vertex>, viewport_w: f32, tab_titles: &[(String, bool, Option<usize>, bool, bool)], left_inset: f32) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let bar_h = (cell_h * 2.0).round();
        let tab_count = tab_titles.len();

        Self::push_bg_quad(vertices, 0.0, 0.0, viewport_w, bar_h, self.tab_bar_bg);

        let max_tab_w = cell_w * 20.0;
        let full_available_w = viewport_w - left_inset;
        let tab_width = (full_available_w / tab_count as f32).min(max_tab_w);

        let version_label = format!("Kova v{}", env!("CARGO_PKG_VERSION"));
        let version_chars = version_label.chars().count() as f32;
        let version_padding = cell_w * (version_chars + 2.0);
        let tabs_right_edge = left_inset + tab_count as f32 * tab_width;
        let show_version = tabs_right_edge <= viewport_w - version_padding;
        let no_bg = [0.0, 0.0, 0.0, 0.0];

        for (i, (title, is_active, color_idx, is_renaming, has_bell)) in tab_titles.iter().enumerate() {
            let x = left_inset + i as f32 * tab_width;

            let tab_bg: Option<[f32; 3]> = if let Some(idx) = color_idx {
                Some(TAB_COLORS[*idx % TAB_COLORS.len()])
            } else if *is_active {
                Some(self.tab_bar_active_bg)
            } else {
                None
            };

            if let Some(bg) = tab_bg {
                Self::push_bg_quad(vertices, x, 0.0, tab_width, bar_h, bg);
            }

            if *is_active {
                let border_h = 6.0_f32;
                let border_color = if let Some(idx) = color_idx {
                    let c = TAB_COLORS[*idx % TAB_COLORS.len()];
                    [(c[0] + 1.0) * 0.5, (c[1] + 1.0) * 0.5, (c[2] + 1.0) * 0.5]
                } else {
                    [0.7, 0.7, 0.7]
                };
                Self::push_bg_quad(vertices, x, bar_h - border_h, tab_width, border_h, border_color);
            }

            let truncated: String;
            let max_title_chars = 25;
            let display_title = if title.chars().count() > max_title_chars {
                if *is_renaming {
                    let skip = title.chars().count() - max_title_chars;
                    truncated = title.chars().skip(skip).collect();
                    &truncated
                } else {
                    truncated = title.chars().take(max_title_chars).collect();
                    &truncated
                }
            } else {
                title
            };
            let label = format!("{}:{}", i + 1, display_title);
            let fg = if color_idx.is_some() || *is_active {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [self.tab_bar_fg[0], self.tab_bar_fg[1], self.tab_bar_fg[2], 1.0]
            };

            let text_w = label.chars().count() as f32 * cell_w;
            let text_x = x + (tab_width - text_w) / 2.0;
            let text_y = (bar_h - cell_h) / 2.0;
            let max_x = x + tab_width - cell_w;
            self.render_status_text(vertices, &label, text_x.max(x + cell_w * 0.5), text_y, max_x, fg, no_bg);

            if *has_bell && !is_active {
                let dot_x = x + tab_width - cell_w * 2.0;
                let dot_y = (bar_h - cell_h) / 2.0;
                let dot_color = if let Some(bg) = tab_bg {
                    let lum = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
                    if (lum([1.0, 0.45, 0.1]) - lum(bg)).abs() < 0.25 { [1.0, 1.0, 1.0, 1.0] } else { [1.0, 0.45, 0.1, 1.0] }
                } else {
                    [1.0, 0.45, 0.1, 1.0]
                };
                self.render_status_text(vertices, "●", dot_x, dot_y, x + tab_width, dot_color, no_bg);
            }
        }

        if show_version {
            let version_fg = [self.tab_bar_fg[0], self.tab_bar_fg[1], self.tab_bar_fg[2], 0.5];
            let version_x = viewport_w - version_padding + cell_w;
            let version_y = (bar_h - cell_h) / 2.0;
            self.render_status_text(vertices, &version_label, version_x, version_y, viewport_w - cell_w * 0.5, version_fg, no_bg);
        }
    }

    fn render_status_text(&mut self, vertices: &mut Vec<Vertex>, text: &str, start_x: f32, y: f32, max_x: f32, fg: [f32; 4], no_bg: [f32; 4]) -> f32 {
        let cell_w = self.atlas.cell_width;
        let atlas_w = self.atlas.atlas_width as f32;
        let atlas_h = self.atlas.atlas_height as f32;

        for c in text.chars() {
            if self.atlas.glyph(c).is_none() { self.atlas.rasterize_char(c); }
        }

        let mut x = start_x;
        for c in text.chars() {
            if x + cell_w > max_x { break; }
            let glyph = match self.atlas.glyph(c) { Some(g) => *g, None => { x += cell_w; continue; } };
            if glyph.width == 0 || glyph.height == 0 { x += cell_w; continue; }

            let gw = glyph.width as f32;
            let gh = glyph.height as f32;
            let tx = glyph.x as f32 / atlas_w;
            let ty = glyph.y as f32 / atlas_h;
            let tw = glyph.width as f32 / atlas_w;
            let th = glyph.height as f32 / atlas_h;

            vertices.push(Vertex { position: [x, y], tex_coords: [tx, ty], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x + gw, y], tex_coords: [tx + tw, ty], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x, y + gh], tex_coords: [tx, ty + th], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x + gw, y], tex_coords: [tx + tw, ty], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x + gw, y + gh], tex_coords: [tx + tw, ty + th], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x, y + gh], tex_coords: [tx, ty + th], color: fg, bg_color: no_bg });
            x += cell_w;
        }
        x
    }

    fn build_filter_overlay_vertices(&mut self, vertices: &mut Vec<Vertex>, vp: &PaneViewport, filter: &FilterRenderData) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;

        let overlay_bg = [0.0, 0.0, 0.0, 0.85];
        let no_tex = [0.0_f32, 0.0];
        let white = [1.0_f32, 1.0, 1.0, 0.0];
        vertices.push(Vertex { position: [vp.x, vp.y], tex_coords: no_tex, color: white, bg_color: overlay_bg });
        vertices.push(Vertex { position: [vp.x + vp.width, vp.y], tex_coords: no_tex, color: white, bg_color: overlay_bg });
        vertices.push(Vertex { position: [vp.x, vp.y + vp.height], tex_coords: no_tex, color: white, bg_color: overlay_bg });
        vertices.push(Vertex { position: [vp.x + vp.width, vp.y], tex_coords: no_tex, color: white, bg_color: overlay_bg });
        vertices.push(Vertex { position: [vp.x + vp.width, vp.y + vp.height], tex_coords: no_tex, color: white, bg_color: overlay_bg });
        vertices.push(Vertex { position: [vp.x, vp.y + vp.height], tex_coords: no_tex, color: white, bg_color: overlay_bg });

        let no_bg = [0.0, 0.0, 0.0, 0.0];
        let bar_bg = [0.2, 0.2, 0.25];
        Self::push_bg_quad(vertices, vp.x, vp.y, vp.width, cell_h, bar_bg);

        let bar_text = format!("/ {}▏", &filter.query);
        let bar_fg = [1.0, 0.8, 0.2, 1.0];
        self.render_status_text(vertices, &bar_text, vp.x + PANE_H_PADDING, vp.y, vp.x + vp.width - cell_w, bar_fg, no_bg);

        let count_text = format!("{} matches", filter.matches.len());
        let count_fg = [0.6, 0.6, 0.6, 1.0];
        let count_w = count_text.chars().count() as f32 * cell_w;
        self.render_status_text(vertices, &count_text, vp.x + vp.width - count_w - PANE_H_PADDING, vp.y, vp.x + vp.width, count_fg, no_bg);

        let max_visible = ((vp.height / cell_h).floor() as usize).saturating_sub(1);
        let match_fg = [0.85, 0.85, 0.85, 1.0];
        let highlight_fg = [1.0, 0.8, 0.2, 1.0];
        let query_lower = filter.query.to_lowercase();
        let max_chars = ((vp.width - 2.0 * PANE_H_PADDING) / cell_w) as usize;

        for (i, m) in filter.matches.iter().take(max_visible).enumerate() {
            let y = vp.y + (i + 1) as f32 * cell_h;
            let max_x = vp.x + vp.width - PANE_H_PADDING;

            let prefix = format!("{:>6}: ", m.abs_line);
            let prefix_fg = [0.5, 0.5, 0.5, 1.0];
            let after_prefix = self.render_status_text(vertices, &prefix, vp.x + PANE_H_PADDING, y, max_x, prefix_fg, no_bg);

            let prefix_chars = prefix.chars().count();
            let text_limit = max_chars.saturating_sub(prefix_chars);
            let display_text: String = m.text.chars().take(text_limit).collect();

            if query_lower.is_empty() {
                self.render_status_text(vertices, &display_text, after_prefix, y, max_x, match_fg, no_bg);
            } else {
                let text_lower: String = display_text.to_lowercase();
                let mut spans: Vec<(&str, bool)> = Vec::new();
                let mut pos = 0;
                while pos < display_text.len() {
                    if let Some(found) = text_lower[pos..].find(&query_lower) {
                        if found > 0 { spans.push((&display_text[pos..pos + found], false)); }
                        let end = pos + found + filter.query.len();
                        spans.push((&display_text[pos + found..end], true));
                        pos = end;
                    } else {
                        spans.push((&display_text[pos..], false));
                        break;
                    }
                }
                let mut x = after_prefix;
                for (span, is_hl) in spans {
                    let fg = if is_hl { highlight_fg } else { match_fg };
                    x = self.render_status_text(vertices, span, x, y, max_x, fg, no_bg);
                }
            }
        }
    }

    fn build_loading_vertices(&mut self, vp: &PaneViewport) -> Vec<Vertex> {
        let text = "starting...";
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let atlas_w = self.atlas.atlas_width as f32;
        let atlas_h = self.atlas.atlas_height as f32;

        let text_w = text.len() as f32 * cell_w;
        let start_x = vp.x + (vp.width - text_w) / 2.0;
        let start_y = vp.y + (vp.height - cell_h) / 2.0;

        let fg = [0.4, 0.4, 0.45, 1.0];
        let no_bg = [0.0, 0.0, 0.0, 0.0];
        let mut vertices = Vec::new();

        for (i, c) in text.chars().enumerate() {
            let glyph = match self.atlas.glyph(c) { Some(g) => *g, None => continue };
            if glyph.width == 0 || glyph.height == 0 { continue; }

            let x = start_x + i as f32 * cell_w;
            let y = start_y;
            let gw = glyph.width as f32;
            let gh = glyph.height as f32;
            let tx = glyph.x as f32 / atlas_w;
            let ty = glyph.y as f32 / atlas_h;
            let tw = glyph.width as f32 / atlas_w;
            let th = glyph.height as f32 / atlas_h;

            vertices.push(Vertex { position: [x, y], tex_coords: [tx, ty], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x + gw, y], tex_coords: [tx + tw, ty], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x, y + gh], tex_coords: [tx, ty + th], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x + gw, y], tex_coords: [tx + tw, ty], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x + gw, y + gh], tex_coords: [tx + tw, ty + th], color: fg, bg_color: no_bg });
            vertices.push(Vertex { position: [x, y + gh], tex_coords: [tx, ty + th], color: fg, bg_color: no_bg });
        }
        vertices
    }

    pub fn rebuild_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scale: f64) {
        self.atlas = GlyphAtlas::new(self.font_size * scale, &self.font_name);
        self.recreate_atlas_texture(device, queue);
    }

    pub fn cell_size(&self) -> (f32, f32) {
        (self.atlas.cell_width, self.atlas.cell_height)
    }

    pub fn status_bar_enabled(&self) -> bool {
        self.status_bar_enabled
    }

    fn push_bg_quad(vertices: &mut Vec<Vertex>, x: f32, y: f32, w: f32, h: f32, bg: [f32; 3]) {
        let bg4 = [bg[0], bg[1], bg[2], 1.0];
        let no_tex = [0.0, 0.0];
        let white = [1.0, 1.0, 1.0, 0.0];
        vertices.push(Vertex { position: [x, y], tex_coords: no_tex, color: white, bg_color: bg4 });
        vertices.push(Vertex { position: [x + w, y], tex_coords: no_tex, color: white, bg_color: bg4 });
        vertices.push(Vertex { position: [x, y + h], tex_coords: no_tex, color: white, bg_color: bg4 });
        vertices.push(Vertex { position: [x + w, y], tex_coords: no_tex, color: white, bg_color: bg4 });
        vertices.push(Vertex { position: [x + w, y + h], tex_coords: no_tex, color: white, bg_color: bg4 });
        vertices.push(Vertex { position: [x, y + h], tex_coords: no_tex, color: white, bg_color: bg4 });
    }
}
