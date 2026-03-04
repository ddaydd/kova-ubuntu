use freetype::face::LoadFlag;
use freetype::Library as FtLibrary;
use std::collections::HashMap;
use unicode_width::UnicodeWidthChar;

#[derive(Copy, Clone, Debug)]
pub struct GlyphInfo {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub is_color: bool,
}

pub struct GlyphAtlas {
    pub glyphs: HashMap<char, GlyphInfo>,
    pub cluster_glyphs: HashMap<Box<str>, GlyphInfo>,
    pub cell_width: f32,
    pub cell_height: f32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub atlas_buf: Vec<u8>,
    /// Flag: atlas texture needs re-upload to GPU
    pub texture_dirty: bool,
    // Dynamic atlas state
    next_x: u32,
    next_y: u32,
    glyph_cell_h: u32,
    // FreeType state
    ft_lib: FtLibrary,
    ft_face: freetype::Face,
    descent: f64,
    /// Fallback font paths discovered via fontconfig, keyed by char
    fallback_faces: HashMap<char, freetype::Face>,
    /// fontconfig handle (kept alive)
    _fc: fontconfig::Fontconfig,
    fc_ref: *const fontconfig::Fontconfig,
    font_name: String,
    font_size: f64,
}

// SAFETY: FreeType faces are only used from main thread (renderer).
// fontconfig is initialized once and queried on main thread only.
unsafe impl Send for GlyphAtlas {}
unsafe impl Sync for GlyphAtlas {}

impl GlyphAtlas {
    pub fn new(font_size: f64, font_name: &str) -> Self {
        let fc = fontconfig::Fontconfig::new().expect("failed to init fontconfig");
        let fc_ref: *const fontconfig::Fontconfig = &fc;

        // Find font file via fontconfig.
        // fontconfig always returns a "best match" even if the font isn't installed,
        // so we verify the result name contains our request (case-insensitive).
        // If not, fall back to "monospace" which fontconfig maps to an actual mono font.
        let font_path = {
            let name_lower = font_name.to_ascii_lowercase();
            let direct = unsafe { &*fc_ref }.find(font_name, None);
            let matched = direct.as_ref().and_then(|f| {
                let result_name = f.name.to_ascii_lowercase();
                if result_name.contains(&name_lower) || name_lower.contains(&result_name) {
                    Some(f.path.clone())
                } else {
                    None
                }
            });
            matched.unwrap_or_else(|| {
                log::warn!("Font '{}' not found, falling back to monospace", font_name);
                unsafe { &*fc_ref }
                    .find("monospace", None)
                    .map(|f| f.path.clone())
                    .unwrap_or_else(|| std::path::PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"))
            })
        };

        log::info!("Using font: {} -> {:?}", font_name, font_path);

        let ft_lib = FtLibrary::init().expect("failed to init FreeType");
        let ft_face = ft_lib
            .new_face(&font_path, 0)
            .expect("failed to load font face");

        // Set pixel size directly (simpler, DPI-independent)
        let pixel_size = font_size.round() as u32;
        ft_face
            .set_pixel_sizes(0, pixel_size)
            .expect("failed to set pixel size");

        // Get metrics
        let metrics = ft_face.size_metrics().expect("no size metrics");
        let ascent = metrics.ascender as f64 / 64.0;
        let descent_abs = (-metrics.descender as f64) / 64.0;
        let height = metrics.height as f64 / 64.0;
        let cell_height = height.ceil() as f32;

        // Get cell width from 'M'
        ft_face
            .load_char('M' as usize, LoadFlag::DEFAULT)
            .expect("failed to load 'M'");
        let cell_width = (ft_face.glyph().advance().x as f64 / 64.0).ceil() as f32;

        log::info!(
            "Glyph metrics: cell={:.1}x{:.1}, ascent={:.1}, descent={:.1}",
            cell_width,
            cell_height,
            ascent,
            descent_abs
        );

        let chars_per_row = 16u32;
        let glyph_cell_w = cell_width.round() as u32;
        let glyph_cell_h = cell_height.round() as u32;
        let num_chars = 95u32; // ' ' to '~'
        let rows = (num_chars + chars_per_row - 1) / chars_per_row;
        // Start with a decent atlas size
        let atlas_width = (chars_per_row * glyph_cell_w).max(2048);
        let atlas_height = (rows * glyph_cell_h).max(512);

        let atlas_bpr = atlas_width as usize * 4;
        let mut atlas_buf = vec![0u8; atlas_bpr * atlas_height as usize];
        let mut glyphs = HashMap::new();

        let bmp_w = cell_width as usize;
        let bmp_h = cell_height as usize;

        for (i, c) in (' '..='~').enumerate() {
            let col = (i as u32) % chars_per_row;
            let row = (i as u32) / chars_per_row;
            let atlas_x = col * glyph_cell_w;
            let atlas_y = row * glyph_cell_h;

            if ft_face.load_char(c as usize, LoadFlag::RENDER | LoadFlag::FORCE_AUTOHINT | LoadFlag::TARGET_LIGHT).is_err() {
                glyphs.insert(
                    c,
                    GlyphInfo {
                        x: atlas_x,
                        y: atlas_y,
                        width: glyph_cell_w,
                        height: glyph_cell_h,
                        is_color: false,
                    },
                );
                continue;
            }

            let glyph_slot = ft_face.glyph();
            let bitmap = glyph_slot.bitmap();
            let bitmap_left = glyph_slot.bitmap_left() as usize;
            let bitmap_top = glyph_slot.bitmap_top();

            // baseline_y = ascent (from top of cell to baseline)
            let baseline_y = ascent.round() as i32;
            let y_off = (baseline_y - bitmap_top).max(0) as usize;

            let bm_w = bitmap.width() as usize;
            let bm_h = bitmap.rows() as usize;
            let bm_pitch = bitmap.pitch().unsigned_abs() as usize;
            let bm_buf = bitmap.buffer();

            // Copy grayscale bitmap to atlas RGBA
            for py in 0..bm_h {
                let dst_y = atlas_y as usize + y_off + py;
                if dst_y >= atlas_height as usize || py >= bmp_h {
                    break;
                }
                for px in 0..bm_w {
                    let dst_x = atlas_x as usize + bitmap_left + px;
                    if dst_x >= atlas_width as usize || bitmap_left + px >= bmp_w {
                        break;
                    }
                    let alpha = bm_buf[py * bm_pitch + px];
                    if alpha > 0 {
                        let off = dst_y * atlas_bpr + dst_x * 4;
                        atlas_buf[off] = 255;
                        atlas_buf[off + 1] = 255;
                        atlas_buf[off + 2] = 255;
                        atlas_buf[off + 3] = alpha;
                    }
                }
            }

            glyphs.insert(
                c,
                GlyphInfo {
                    x: atlas_x,
                    y: atlas_y,
                    width: glyph_cell_w,
                    height: glyph_cell_h,
                    is_color: false,
                },
            );
        }

        // Track cursor position after initial ASCII glyphs
        let last_idx = num_chars - 1;
        let last_col = last_idx % chars_per_row;
        let last_row = last_idx / chars_per_row;
        let next_x_val = (last_col + 1) % chars_per_row;
        let next_y_val = if next_x_val == 0 {
            last_row + 1
        } else {
            last_row
        };

        log::info!(
            "Glyph atlas: {}x{}, cell: {:.1}x{:.1}",
            atlas_width,
            atlas_height,
            cell_width,
            cell_height
        );

        GlyphAtlas {
            glyphs,
            cluster_glyphs: HashMap::new(),
            cell_width,
            cell_height,
            atlas_width,
            atlas_height,
            atlas_buf,
            texture_dirty: true,
            next_x: next_x_val * glyph_cell_w,
            next_y: next_y_val * glyph_cell_h,
            glyph_cell_h,
            ft_lib,
            ft_face,
            descent: descent_abs,
            fallback_faces: HashMap::new(),
            _fc: fc,
            fc_ref,
            font_name: font_name.to_string(),
            font_size,
        }
    }

    pub fn glyph(&self, c: char) -> Option<&GlyphInfo> {
        self.glyphs.get(&c)
    }

    pub fn cluster_glyph(&self, cluster: &str) -> Option<&GlyphInfo> {
        self.cluster_glyphs.get(cluster)
    }

    /// Find a fallback font for a character using fontconfig.
    fn find_fallback_face(&mut self, c: char) -> Option<&freetype::Face> {
        if self.fallback_faces.contains_key(&c) {
            return self.fallback_faces.get(&c);
        }

        // Query fontconfig for a font containing this character
        let fc = unsafe { &*self.fc_ref };
        // Try to find a font that supports this codepoint
        // fontconfig doesn't have a direct "find font for char" in the Rust binding,
        // so we try common fallback fonts
        let fallback_names = ["Noto Color Emoji", "Noto Sans", "Noto Sans Mono", "DejaVu Sans", "Liberation Mono", "FreeMono"];
        for name in &fallback_names {
            if let Some(font) = fc.find(name, None) {
                if let Ok(face) = self.ft_lib.new_face(&font.path, 0) {
                    let pixel_size = self.font_size.round() as u32;
                    let _ = face.set_pixel_sizes(0, pixel_size);
                    let glyph_idx = face.get_char_index(c as usize);
                    if glyph_idx.is_some() {
                        log::trace!("Fallback for '{}' U+{:04X}: {} ({:?})", c, c as u32, name, font.path);
                        self.fallback_faces.insert(c, face);
                        return self.fallback_faces.get(&c);
                    }
                }
            }
        }

        log::warn!("No fallback font found for '{}' U+{:04X}", c, c as u32);
        None
    }

    /// Draw a block element or box-drawing character directly into a bitmap buffer.
    /// Returns true if the character was handled, false otherwise.
    fn draw_builtin_glyph(c: char, buf: &mut [u8], w: usize, h: usize) -> bool {
        let bpr = w * 4;

        let fill_rect = |buf: &mut [u8], x0: usize, y0: usize, x1: usize, y1: usize| {
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let off = y * bpr + x * 4;
                    buf[off] = 255;
                    buf[off + 1] = 255;
                    buf[off + 2] = 255;
                    buf[off + 3] = 255;
                }
            }
        };

        let hw = w / 2;
        let hh = h / 2;

        match c {
            // === Block Elements (U+2580-U+259F) ===
            '\u{2580}' => { fill_rect(buf, 0, 0, w, hh); true }
            '\u{2581}' => { let t = h - h/8; fill_rect(buf, 0, t, w, h); true }
            '\u{2582}' => { let t = h - h/4; fill_rect(buf, 0, t, w, h); true }
            '\u{2583}' => { let t = h - 3*h/8; fill_rect(buf, 0, t, w, h); true }
            '\u{2584}' => { fill_rect(buf, 0, hh, w, h); true }
            '\u{2585}' => { let t = h - 5*h/8; fill_rect(buf, 0, t, w, h); true }
            '\u{2586}' => { let t = h - 3*h/4; fill_rect(buf, 0, t, w, h); true }
            '\u{2587}' => { let t = h - 7*h/8; fill_rect(buf, 0, t, w, h); true }
            '\u{2588}' => { fill_rect(buf, 0, 0, w, h); true }
            '\u{2589}' => { let r = 7*w/8; fill_rect(buf, 0, 0, r, h); true }
            '\u{258A}' => { let r = 3*w/4; fill_rect(buf, 0, 0, r, h); true }
            '\u{258B}' => { let r = 5*w/8; fill_rect(buf, 0, 0, r, h); true }
            '\u{258C}' => { fill_rect(buf, 0, 0, hw, h); true }
            '\u{258D}' => { let r = 3*w/8; fill_rect(buf, 0, 0, r, h); true }
            '\u{258E}' => { let r = w/4; fill_rect(buf, 0, 0, r, h); true }
            '\u{258F}' => { let r = w/8; fill_rect(buf, 0, 0, r, h); true }
            '\u{2590}' => { fill_rect(buf, hw, 0, w, h); true }
            '\u{2591}' => {
                for y in 0..h { for x in 0..w { if (x + y) % 4 == 0 { let o = y*bpr+x*4; buf[o]=255; buf[o+1]=255; buf[o+2]=255; buf[o+3]=255; } } }
                true
            }
            '\u{2592}' => {
                for y in 0..h { for x in 0..w { if (x + y) % 2 == 0 { let o = y*bpr+x*4; buf[o]=255; buf[o+1]=255; buf[o+2]=255; buf[o+3]=255; } } }
                true
            }
            '\u{2593}' => {
                for y in 0..h { for x in 0..w { if (x + y) % 4 != 0 { let o = y*bpr+x*4; buf[o]=255; buf[o+1]=255; buf[o+2]=255; buf[o+3]=255; } } }
                true
            }
            '\u{2596}' => { fill_rect(buf, 0, hh, hw, h); true }
            '\u{2597}' => { fill_rect(buf, hw, hh, w, h); true }
            '\u{2598}' => { fill_rect(buf, 0, 0, hw, hh); true }
            '\u{2599}' => { fill_rect(buf, 0, 0, hw, hh); fill_rect(buf, 0, hh, w, h); true }
            '\u{259A}' => { fill_rect(buf, 0, 0, hw, hh); fill_rect(buf, hw, hh, w, h); true }
            '\u{259B}' => { fill_rect(buf, 0, 0, w, hh); fill_rect(buf, 0, hh, hw, h); true }
            '\u{259C}' => { fill_rect(buf, 0, 0, w, hh); fill_rect(buf, hw, hh, w, h); true }
            '\u{259D}' => { fill_rect(buf, hw, 0, w, hh); true }
            '\u{259E}' => { fill_rect(buf, hw, 0, w, hh); fill_rect(buf, 0, hh, hw, h); true }
            '\u{259F}' => { fill_rect(buf, hw, 0, w, hh); fill_rect(buf, 0, hh, w, h); true }

            // === Box-Drawing (U+2500-U+257F) ===
            '\u{2500}' | '\u{2501}' => {
                let thick = if c == '\u{2501}' { 3 } else { 1 };
                let y0 = hh.saturating_sub(thick / 2);
                fill_rect(buf, 0, y0, w, y0 + thick);
                true
            }
            '\u{2502}' | '\u{2503}' => {
                let thick = if c == '\u{2503}' { 3 } else { 1 };
                let x0 = hw.saturating_sub(thick / 2);
                fill_rect(buf, x0, 0, x0 + thick, h);
                true
            }
            '\u{250C}' => { fill_rect(buf, hw, hh, hw + 1, h); fill_rect(buf, hw, hh, w, hh + 1); true }
            '\u{2510}' => { fill_rect(buf, hw, hh, hw + 1, h); fill_rect(buf, 0, hh, hw + 1, hh + 1); true }
            '\u{2514}' => { fill_rect(buf, hw, 0, hw + 1, hh + 1); fill_rect(buf, hw, hh, w, hh + 1); true }
            '\u{2518}' => { fill_rect(buf, hw, 0, hw + 1, hh + 1); fill_rect(buf, 0, hh, hw + 1, hh + 1); true }
            '\u{251C}' => { fill_rect(buf, hw, 0, hw + 1, h); fill_rect(buf, hw, hh, w, hh + 1); true }
            '\u{2524}' => { fill_rect(buf, hw, 0, hw + 1, h); fill_rect(buf, 0, hh, hw + 1, hh + 1); true }
            '\u{252C}' => { fill_rect(buf, 0, hh, w, hh + 1); fill_rect(buf, hw, hh, hw + 1, h); true }
            '\u{2534}' => { fill_rect(buf, 0, hh, w, hh + 1); fill_rect(buf, hw, 0, hw + 1, hh + 1); true }
            '\u{253C}' => { fill_rect(buf, 0, hh, w, hh + 1); fill_rect(buf, hw, 0, hw + 1, h); true }
            '\u{256D}' => { fill_rect(buf, hw, hh, hw + 1, h); fill_rect(buf, hw, hh, w, hh + 1); true }
            '\u{256E}' => { fill_rect(buf, hw, hh, hw + 1, h); fill_rect(buf, 0, hh, hw + 1, hh + 1); true }
            '\u{256F}' => { fill_rect(buf, hw, 0, hw + 1, hh + 1); fill_rect(buf, 0, hh, hw + 1, hh + 1); true }
            '\u{2570}' => { fill_rect(buf, hw, 0, hw + 1, hh + 1); fill_rect(buf, hw, hh, w, hh + 1); true }
            '\u{2550}' => {
                let y0 = hh.saturating_sub(1);
                fill_rect(buf, 0, y0, w, y0 + 1);
                fill_rect(buf, 0, y0 + 2, w, y0 + 3);
                true
            }
            '\u{2551}' => {
                let x0 = hw.saturating_sub(1);
                fill_rect(buf, x0, 0, x0 + 1, h);
                fill_rect(buf, x0 + 2, 0, x0 + 3, h);
                true
            }
            '\u{2552}'..='\u{256C}' => false,
            '\u{2504}' | '\u{2505}' | '\u{2508}' | '\u{2509}' |
            '\u{254C}' | '\u{254D}' | '\u{254E}' | '\u{254F}' => false,
            _ => false,
        }
    }

    /// Rasterize a single character on-demand and add it to the atlas.
    pub fn rasterize_char(&mut self, c: char) -> Option<GlyphInfo> {
        if let Some(g) = self.glyphs.get(&c) {
            return Some(*g);
        }

        let width_cells = UnicodeWidthChar::width(c).unwrap_or(1).max(1);
        let bmp_w = self.cell_width as usize * width_cells;
        let bmp_h = self.cell_height as usize;
        let bmp_bpr = bmp_w * 4;

        // Try builtin drawing for block elements and box-drawing first
        let mut builtin_buf = vec![0u8; bmp_bpr * bmp_h];
        if Self::draw_builtin_glyph(c, &mut builtin_buf, bmp_w, bmp_h) {
            log::trace!("builtin glyph for '{}' U+{:04X}", c, c as u32);
            return self.insert_bitmap(c, &builtin_buf, bmp_w, bmp_h, false);
        }

        // Try primary font first
        let glyph_idx = self.ft_face.get_char_index(c as usize);
        let (face_ptr, is_fallback) = if glyph_idx.is_some() {
            (&self.ft_face as *const freetype::Face, false)
        } else {
            // Try fallback
            match self.find_fallback_face(c) {
                Some(face) => (face as *const freetype::Face, true),
                None => return None,
            }
        };

        let face = unsafe { &*face_ptr };

        // Try color emoji first (FT_LOAD_COLOR)
        let is_color = face.load_char(c as usize, LoadFlag::RENDER | LoadFlag::COLOR).is_ok()
            && face.glyph().bitmap().pixel_mode().unwrap_or(freetype::bitmap::PixelMode::Mono)
                == freetype::bitmap::PixelMode::Bgra;

        if !is_color {
            // Reload without COLOR flag for grayscale with light hinting
            if face.load_char(c as usize, LoadFlag::RENDER | LoadFlag::FORCE_AUTOHINT | LoadFlag::TARGET_LIGHT).is_err() {
                return None;
            }
        }

        let glyph_slot = face.glyph();
        let bitmap = glyph_slot.bitmap();
        let bitmap_left = glyph_slot.bitmap_left().max(0) as usize;
        let bitmap_top = glyph_slot.bitmap_top();

        let metrics = self.ft_face.size_metrics().unwrap();
        let ascent = (metrics.ascender as f64 / 64.0).round() as i32;
        let y_off = (ascent - bitmap_top).max(0) as usize;

        let bm_w = bitmap.width() as usize;
        let bm_h = bitmap.rows() as usize;
        let bm_pitch = bitmap.pitch().unsigned_abs() as usize;
        let bm_buf = bitmap.buffer();

        let mut render_buf = vec![0u8; bmp_bpr * bmp_h];

        if is_color {
            // BGRA bitmap from FreeType
            for py in 0..bm_h {
                let dst_y = y_off + py;
                if dst_y >= bmp_h { break; }
                for px in 0..bm_w {
                    let dst_x = bitmap_left + px;
                    if dst_x >= bmp_w { break; }
                    let src_off = py * bm_pitch + px * 4;
                    let dst_off = dst_y * bmp_bpr + dst_x * 4;
                    if src_off + 3 < bm_buf.len() {
                        // BGRA -> RGBA
                        render_buf[dst_off] = bm_buf[src_off + 2];     // R
                        render_buf[dst_off + 1] = bm_buf[src_off + 1]; // G
                        render_buf[dst_off + 2] = bm_buf[src_off];     // B
                        render_buf[dst_off + 3] = bm_buf[src_off + 3]; // A
                    }
                }
            }
        } else {
            // Grayscale bitmap
            for py in 0..bm_h {
                let dst_y = y_off + py;
                if dst_y >= bmp_h { break; }
                for px in 0..bm_w {
                    let dst_x = bitmap_left + px;
                    if dst_x >= bmp_w { break; }
                    let alpha = bm_buf[py * bm_pitch + px];
                    if alpha > 0 {
                        let dst_off = dst_y * bmp_bpr + dst_x * 4;
                        render_buf[dst_off] = 255;
                        render_buf[dst_off + 1] = 255;
                        render_buf[dst_off + 2] = 255;
                        render_buf[dst_off + 3] = alpha;
                    }
                }
            }
        }

        log::trace!(
            "rasterize '{}' U+{:04X}: bmp {}x{}, width_cells={}, is_color={}, fallback={}",
            c, c as u32, bmp_w, bmp_h, width_cells, is_color, is_fallback
        );

        self.insert_bitmap(c, &render_buf, bmp_w, bmp_h, is_color)
    }

    /// Rasterize a multi-codepoint grapheme cluster
    pub fn rasterize_cluster(&mut self, cluster: &str) -> Option<GlyphInfo> {
        if let Some(g) = self.cluster_glyphs.get(cluster) {
            return Some(*g);
        }

        use unicode_width::UnicodeWidthStr;
        let width_cells = UnicodeWidthStr::width(cluster).max(1);
        let bmp_w = self.cell_width as usize * width_cells;
        let bmp_h = self.cell_height as usize;
        let bmp_bpr = bmp_w * 4;

        // For clusters (emoji sequences), try to render each codepoint
        // This is a simplified approach - full shaping would require HarfBuzz
        let mut render_buf = vec![0u8; bmp_bpr * bmp_h];

        // Try the first char of the cluster as a color emoji
        let first_char = cluster.chars().next()?;
        let glyph_idx = self.ft_face.get_char_index(first_char as usize);

        let face: &freetype::Face = if glyph_idx.is_some() {
            &self.ft_face
        } else {
            self.find_fallback_face(first_char)?
        };

        let is_color = face.load_char(first_char as usize, LoadFlag::RENDER | LoadFlag::COLOR).is_ok()
            && face.glyph().bitmap().pixel_mode().unwrap_or(freetype::bitmap::PixelMode::Mono)
                == freetype::bitmap::PixelMode::Bgra;

        if !is_color {
            if face.load_char(first_char as usize, LoadFlag::RENDER | LoadFlag::FORCE_AUTOHINT | LoadFlag::TARGET_LIGHT).is_err() {
                return None;
            }
        }

        let glyph_slot = face.glyph();
        let bitmap = glyph_slot.bitmap();
        let bitmap_left = glyph_slot.bitmap_left().max(0) as usize;
        let bitmap_top = glyph_slot.bitmap_top();

        let metrics = self.ft_face.size_metrics().unwrap();
        let ascent = (metrics.ascender as f64 / 64.0).round() as i32;
        let y_off = (ascent - bitmap_top).max(0) as usize;

        let bm_w = bitmap.width() as usize;
        let bm_h = bitmap.rows() as usize;
        let bm_pitch = bitmap.pitch().unsigned_abs() as usize;
        let bm_buf = bitmap.buffer();

        if is_color {
            for py in 0..bm_h {
                let dst_y = y_off + py;
                if dst_y >= bmp_h { break; }
                for px in 0..bm_w {
                    let dst_x = bitmap_left + px;
                    if dst_x >= bmp_w { break; }
                    let src_off = py * bm_pitch + px * 4;
                    let dst_off = dst_y * bmp_bpr + dst_x * 4;
                    if src_off + 3 < bm_buf.len() {
                        render_buf[dst_off] = bm_buf[src_off + 2];
                        render_buf[dst_off + 1] = bm_buf[src_off + 1];
                        render_buf[dst_off + 2] = bm_buf[src_off];
                        render_buf[dst_off + 3] = bm_buf[src_off + 3];
                    }
                }
            }
        } else {
            for py in 0..bm_h {
                let dst_y = y_off + py;
                if dst_y >= bmp_h { break; }
                for px in 0..bm_w {
                    let dst_x = bitmap_left + px;
                    if dst_x >= bmp_w { break; }
                    let alpha = bm_buf[py * bm_pitch + px];
                    if alpha > 0 {
                        let dst_off = dst_y * bmp_bpr + dst_x * 4;
                        render_buf[dst_off] = 255;
                        render_buf[dst_off + 1] = 255;
                        render_buf[dst_off + 2] = 255;
                        render_buf[dst_off + 3] = alpha;
                    }
                }
            }
        }

        let info = self.insert_bitmap_raw(&render_buf, bmp_w, bmp_h, is_color || true)?;
        self.cluster_glyphs.insert(cluster.into(), info);
        Some(info)
    }

    fn insert_bitmap(
        &mut self,
        c: char,
        bmp_buf: &[u8],
        bmp_w: usize,
        bmp_h: usize,
        is_color: bool,
    ) -> Option<GlyphInfo> {
        let info = self.insert_bitmap_raw(bmp_buf, bmp_w, bmp_h, is_color)?;
        self.glyphs.insert(c, info);
        Some(info)
    }

    fn insert_bitmap_raw(
        &mut self,
        bmp_buf: &[u8],
        bmp_w: usize,
        bmp_h: usize,
        is_color: bool,
    ) -> Option<GlyphInfo> {
        let bmp_bpr = bmp_w * 4;
        let slot_w = bmp_w as u32;

        // Check if we need to wrap to next row
        if self.next_x + slot_w > self.atlas_width {
            self.next_x = 0;
            self.next_y += self.glyph_cell_h;
        }

        // Grow atlas if needed
        if self.next_y + self.glyph_cell_h > self.atlas_height {
            self.grow_atlas();
        }

        let atlas_x = self.next_x;
        let atlas_y = self.next_y;
        let atlas_bpr = self.atlas_width as usize * 4;

        // Copy bitmap to atlas
        for py in 0..bmp_h {
            let dst_y = atlas_y as usize + py;
            if dst_y >= self.atlas_height as usize {
                break;
            }
            let src_off = py * bmp_bpr;
            let dst_off = dst_y * atlas_bpr + atlas_x as usize * 4;
            let copy_bytes = (bmp_w * 4).min(atlas_bpr - atlas_x as usize * 4);
            self.atlas_buf[dst_off..dst_off + copy_bytes]
                .copy_from_slice(&bmp_buf[src_off..src_off + copy_bytes]);
        }

        self.texture_dirty = true;

        let info = GlyphInfo {
            x: atlas_x,
            y: atlas_y,
            width: bmp_w as u32,
            height: self.glyph_cell_h,
            is_color,
        };

        self.next_x += bmp_w as u32;
        Some(info)
    }

    fn grow_atlas(&mut self) {
        let new_height = self.atlas_height * 2;
        let atlas_bpr = self.atlas_width as usize * 4;
        self.atlas_buf.resize(atlas_bpr * new_height as usize, 0);
        self.atlas_height = new_height;
        self.texture_dirty = true;
        log::info!("Atlas grew to {}x{}", self.atlas_width, self.atlas_height);
    }
}
