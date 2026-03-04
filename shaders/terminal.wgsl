struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) bg_color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) bg_color: vec4<f32>,
};

struct Uniforms {
    viewport_size: vec2<f32>,
    atlas_size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Convert pixel coords to NDC: (0,0) top-left, (w,h) bottom-right
    let ndc_x = (in.position.x / uniforms.viewport_size.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (in.position.y / uniforms.viewport_size.y) * 2.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.bg_color = in.bg_color;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // If bg_color alpha > 0, this is a background quad
    if (in.bg_color.a > 0.0) {
        return in.bg_color;
    }

    let tex_color = textureSample(atlas_texture, atlas_sampler, in.tex_coords);

    // Color emoji: color.a == 2.0 signals color glyph — use texture directly
    if (in.color.a > 1.5) {
        return tex_color;
    }

    // LCD subpixel rendering: R,G,B channels are per-subpixel coverage masks.
    // Blend each color channel independently for sharper text.
    let r = in.color.r * tex_color.r;
    let g = in.color.g * tex_color.g;
    let b = in.color.b * tex_color.b;
    let a = tex_color.a;

    return vec4<f32>(r, g, b, a);
}
