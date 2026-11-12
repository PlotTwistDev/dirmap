//! GPU drawing of treemap tiles: one instanced quad per tile, with the cushion shading, the
//! pointer-following light, hover feedback and the bevel all computed per pixel in a shader.
//!
//! The CPU only uploads a tile set when it is rebuilt (after the camera has moved a good way);
//! every other frame sends one small block of uniforms per tile set. Works with both of eframe's
//! renderers: wgpu (WGSL) and OpenGL (GLSL 3.30 / ES 3.00).

use bytemuck::{Pod, Zeroable};
use eframe::egui::{self, Rect};
use eframe::{egui_glow, egui_wgpu, glow, wgpu};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One tile. `rect` is in "local" units: physical pixels at the scale the set was built for,
/// relative to the set's origin, so f32 is plenty precise however deep the zoom.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct Instance {
    pub rect: [f32; 4],
    /// Cushion gradient: dz/dx = grad[0] * u + grad[1], dz/dy = grad[2] * v + grad[3], where
    /// (u, v) is the local position. Dimensionless, so the shading looks the same at any zoom.
    pub grad: [f32; 4],
    /// sRGB colour (already including the brightness gain's base colour, not the shading).
    pub color: [u8; 4],
    pub flags: u32,
}

/// Painted flat, without cushion, light or bevel (filtered-out tiles, the background).
pub const FLAT: u32 = 1;

/// Per-draw settings. All vec4s so the layout is the same in WGSL and GLSL std140.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct Uniforms {
    /// ndc = local * xy + zw
    pub xform: [f32; 4],
    /// Physical pixels per local unit (x, y), layer alpha, flags (bit 0 bevel, bit 1 grid, bit 2 sRGB target).
    pub px: [f32; 4],
    /// Pointer light: position (local units), intensity 0.., radius in pixels.
    pub light: [f32; 4],
    /// Pointer light: height in pixels, diffuse strength, highlight strength, unused.
    pub light2: [f32; 4],
    /// Cushion shading: ambient, diffuse, gain, unused.
    pub shade: [f32; 4],
    /// Fixed cushion light direction (normalised), unused.
    pub sun: [f32; 4],
    /// Hovered tile (local rect) fading in, and the previously hovered one fading out.
    pub hover_a: [f32; 4],
    pub hover_b: [f32; 4],
    /// Their strengths (0..1), unused, unused.
    pub hover_w: [f32; 4],
    /// Grid colour (sRGB 0..1).
    pub grid: [f32; 4],
}

pub const BEVEL: u32 = 1;
pub const GRID: u32 = 2;
const SRGB_TARGET: u32 = 4;

/// A tile set to draw this frame.
#[derive(Clone)]
pub struct Draw {
    /// Identifies the instance data; uploaded once per id.
    pub id: u64,
    pub instances: Arc<Vec<Instance>>,
    pub uniforms: Uniforms,
}

/// Which GPU path is available, decided once at start-up.
#[derive(Clone)]
pub enum Backend {
    Wgpu,
    Glow(Arc<Mutex<GlState>>),
    /// No usable GPU path: the map is drawn from CPU-rendered images.
    None,
}

impl Backend {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        if std::env::var_os("DIRMAP_CPU_TREEMAP").is_some() {
            return Backend::None;
        }
        if let Some(rs) = &cc.wgpu_render_state {
            let r = WgpuState::new(&rs.device, rs.target_format);
            rs.renderer.write().callback_resources.insert(r);
            return Backend::Wgpu;
        }
        if let Some(gl) = &cc.gl {
            match GlState::new(gl) {
                Ok(s) => return Backend::Glow(Arc::new(Mutex::new(s))),
                Err(e) => eprintln!("treemap: OpenGL tile shader unavailable ({e}); drawing on the CPU"),
            }
        }
        Backend::None
    }

    pub fn is_gpu(&self) -> bool {
        !matches!(self, Backend::None)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Backend::Wgpu => "wgpu",
            Backend::Glow(_) => "OpenGL",
            Backend::None => "CPU",
        }
    }

    /// Add a paint callback drawing `draw` over `rect` (clipped to the painter's clip rect).
    /// `slot` must be distinct for each draw within one frame; `frame` increases every frame.
    pub fn paint(&self, painter: &egui::Painter, rect: Rect, draw: Draw, slot: usize, frame: u64) {
        match self {
            Backend::Wgpu => {
                painter.add(egui_wgpu::Callback::new_paint_callback(rect, WgpuDraw { draw, slot, frame }));
            }
            Backend::Glow(state) => {
                let state = state.clone();
                let cb = egui_glow::CallbackFn::new(move |_info, painter| {
                    state.lock().unwrap().paint(painter.gl(), &draw, frame);
                });
                painter.add(egui::PaintCallback { rect, callback: Arc::new(cb) });
            }
            Backend::None => {}
        }
    }
}

// ---------- shaders ----------

const WGSL: &str = r#"
struct U {
    xform: vec4<f32>,
    px: vec4<f32>,
    light: vec4<f32>,
    light2: vec4<f32>,
    shade: vec4<f32>,
    sun: vec4<f32>,
    hover_a: vec4<f32>,
    hover_b: vec4<f32>,
    hover_w: vec4<f32>,
    grid: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) p: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) grad: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) flags: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @location(0) rect: vec4<f32>, @location(1) grad: vec4<f32>,
           @location(2) color: vec4<f32>, @location(3) flags: u32) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let p = mix(rect.xy, rect.zw, corner);
    var o: VOut;
    o.pos = vec4<f32>(p * u.xform.xy + u.xform.zw, 0.0, 1.0);
    o.p = p;
    o.rect = rect;
    o.grad = grad;
    o.color = color;
    o.flags = flags;
    return o;
}

fn same_rect(a: vec4<f32>, b: vec4<f32>) -> bool {
    let d = abs(a - b);
    return max(max(d.x, d.y), max(d.z, d.w)) < 0.01;
}

// Roll off the brightest channel smoothly towards 1 and scale the others with it, so lit
// colours brighten without clipping or bleaching (the hue stays the same).
fn shoulder(c: vec3<f32>) -> vec3<f32> {
    let m = max(max(c.r, c.g), c.b);
    let k = 0.82;
    if (m <= k) { return c; }
    let t = k + (1.0 - k) * (1.0 - exp(-(m - k) / (1.0 - k)));
    return c * (t / m);
}

fn shade(in: VOut) -> vec4<f32> {
    let k = u.px.xy;
    let alpha = u.px.z;
    let opts = u32(u.px.w);
    let r = in.rect;
    let base = in.color.rgb;
    let d0 = (in.p - r.xy) * k;
    let d1 = (r.zw - in.p) * k;
    if ((in.flags & 1u) != 0u) {
        return vec4<f32>(base * alpha, alpha);
    }
    let size = (r.zw - r.xy) * k;
    let m = min(size.x, size.y);
    let g = vec2<f32>(in.grad.x * in.p.x + in.grad.y, in.grad.z * in.p.y + in.grad.w);
    let n = normalize(vec3<f32>(-g.x, -g.y, 1.0));
    // The original cushion shading (van Wijk & van de Wetering, WinDirStat's knobs).
    let cosa = clamp(dot(n, u.sun.xyz), 0.0, 1.0);
    var col = base * u.shade.z * (u.shade.x + u.shade.y * cosa);

    var hov = 0.0;
    if (same_rect(r, u.hover_a)) { hov = u.hover_w.x; }
    if (same_rect(r, u.hover_b)) { hov = max(hov, u.hover_w.y); }

    // Light following the pointer: a point light a little above the surface, so each tile's
    // cushion catches it on the side facing the pointer and the highlight slides across the
    // tile as the pointer moves. Smooth Gaussian falloff; small tiles respond less, and tiles
    // other than the one under the pointer get a restrained share.
    if (u.light.z > 0.001) {
        let dpx = (u.light.xy - in.p) * k;
        let lv = normalize(vec3<f32>(dpx, u.light2.x));
        let lam = max(dot(n, lv), 0.0);
        let fall = exp(-dot(dpx, dpx) / (u.light.w * u.light.w));
        let h = normalize(lv + vec3<f32>(0.0, 0.0, 1.0));
        let spec = pow(max(dot(n, h), 0.0), 14.0);
        let sizef = mix(0.22, 1.0, smoothstep(5.0, 160.0, m));
        let own = mix(0.45, 1.0, hov);
        let amt = u.light.z * fall * sizef * own;
        // Brighten without bleaching: scale the colour, then pull it back so no channel clips,
        // which keeps the hue (the file-type colour) intact. A faint, colour-tinted sheen on top.
        let lifted = col * (1.0 + u.light2.y * amt * lam);
        let sheen = mix(base, vec3<f32>(1.0), 0.5) * (u.light2.z * amt * spec * sizef);
        col = shoulder(lifted) + sheen;
    }

    // Hover: a little brighter, with a soft inner edge.
    if (hov > 0.0) {
        let edge = 1.0 - smoothstep(0.0, 1.6, min(min(d0.x, d0.y), min(d1.x, d1.y)));
        col = col * (1.0 + 0.07 * hov) + vec3<f32>(0.03 * hov);
        col = mix(col, vec3<f32>(1.0), 0.38 * hov * edge * smoothstep(4.0, 10.0, m));
    }

    // Very slight bevel: light top/left edge, shaded bottom/right edge.
    if ((opts & 1u) != 0u) {
        let b = smoothstep(3.0, 12.0, m);
        let tl = 1.0 - smoothstep(0.0, 1.5, min(d0.x, d0.y));
        let br = 1.0 - smoothstep(0.0, 1.8, min(d1.x, d1.y));
        col = col * (1.0 - 0.24 * br * b) + vec3<f32>(0.06 * tl * b);
    }
    if ((opts & 2u) != 0u && (d0.x < 1.0 || d0.y < 1.0)) {
        col = u.grid.rgb;
    }
    col = clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));
    if ((opts & 4u) != 0u) {
        col = pow(col, vec3<f32>(2.2));
    }
    return vec4<f32>(col * alpha, alpha);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    return shade(in);
}
"#;

/// Same shader in GLSL (body shared with the WGSL above by hand; keep them in step).
const GLSL_VS: &str = r#"
layout(std140) uniform U {
    vec4 xform; vec4 px; vec4 light; vec4 light2; vec4 shade; vec4 sun;
    vec4 hover_a; vec4 hover_b; vec4 hover_w; vec4 grid;
};
layout(location = 0) in vec4 a_rect;
layout(location = 1) in vec4 a_grad;
layout(location = 2) in vec4 a_color;
layout(location = 3) in uint a_flags;
out vec2 v_p;
flat out vec4 v_rect;
flat out vec4 v_grad;
flat out vec4 v_color;
flat out uint v_flags;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    vec2 p = mix(a_rect.xy, a_rect.zw, corner);
    gl_Position = vec4(p * xform.xy + xform.zw, 0.0, 1.0);
    v_p = p; v_rect = a_rect; v_grad = a_grad; v_color = a_color; v_flags = a_flags;
}
"#;

const GLSL_FS: &str = r#"
layout(std140) uniform U {
    vec4 xform; vec4 px; vec4 light; vec4 light2; vec4 shade; vec4 sun;
    vec4 hover_a; vec4 hover_b; vec4 hover_w; vec4 grid;
};
in vec2 v_p;
flat in vec4 v_rect;
flat in vec4 v_grad;
flat in vec4 v_color;
flat in uint v_flags;
out vec4 frag;
vec3 shoulder(vec3 c) {
    float m = max(max(c.r, c.g), c.b);
    float k = 0.82;
    if (m <= k) return c;
    float t = k + (1.0 - k) * (1.0 - exp(-(m - k) / (1.0 - k)));
    return c * (t / m);
}
bool same_rect(vec4 a, vec4 b) { vec4 d = abs(a - b); return max(max(d.x, d.y), max(d.z, d.w)) < 0.01; }
void main() {
    vec2 k = px.xy;
    float alpha = px.z;
    int opts = int(px.w);
    vec4 r = v_rect;
    vec3 base = v_color.rgb;
    vec2 d0 = (v_p - r.xy) * k;
    vec2 d1 = (r.zw - v_p) * k;
    if ((v_flags & 1u) != 0u) { frag = vec4(base * alpha, alpha); return; }
    vec2 size = (r.zw - r.xy) * k;
    float m = min(size.x, size.y);
    vec2 g = vec2(v_grad.x * v_p.x + v_grad.y, v_grad.z * v_p.y + v_grad.w);
    vec3 n = normalize(vec3(-g.x, -g.y, 1.0));
    float cosa = clamp(dot(n, sun.xyz), 0.0, 1.0);
    vec3 col = base * shade.z * (shade.x + shade.y * cosa);
    float hov = 0.0;
    if (same_rect(r, hover_a)) hov = hover_w.x;
    if (same_rect(r, hover_b)) hov = max(hov, hover_w.y);
    if (light.z > 0.001) {
        vec2 dpx = (light.xy - v_p) * k;
        vec3 lv = normalize(vec3(dpx, light2.x));
        float lam = max(dot(n, lv), 0.0);
        float fall = exp(-dot(dpx, dpx) / (light.w * light.w));
        vec3 h = normalize(lv + vec3(0.0, 0.0, 1.0));
        float spec = pow(max(dot(n, h), 0.0), 14.0);
        float sizef = mix(0.22, 1.0, smoothstep(5.0, 160.0, m));
        float own = mix(0.45, 1.0, hov);
        float amt = light.z * fall * sizef * own;
        vec3 lifted = col * (1.0 + light2.y * amt * lam);
        vec3 sheen = mix(base, vec3(1.0), 0.5) * (light2.z * amt * spec * sizef);
        col = shoulder(lifted) + sheen;
    }
    if (hov > 0.0) {
        float edge = 1.0 - smoothstep(0.0, 1.6, min(min(d0.x, d0.y), min(d1.x, d1.y)));
        col = col * (1.0 + 0.07 * hov) + vec3(0.03 * hov);
        col = mix(col, vec3(1.0), 0.38 * hov * edge * smoothstep(4.0, 10.0, m));
    }
    if ((opts & 1) != 0) {
        float b = smoothstep(3.0, 12.0, m);
        float tl = 1.0 - smoothstep(0.0, 1.5, min(d0.x, d0.y));
        float br = 1.0 - smoothstep(0.0, 1.8, min(d1.x, d1.y));
        col = col * (1.0 - 0.24 * br * b) + vec3(0.06 * tl * b);
    }
    if ((opts & 2) != 0 && (d0.x < 1.0 || d0.y < 1.0)) col = grid.rgb;
    col = clamp(col, vec3(0.0), vec3(1.0));
    frag = vec4(col * alpha, alpha);
}
"#;

// ---------- wgpu ----------

struct WgpuState {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    srgb: bool,
    /// Uniform buffer and bind group per draw slot.
    slots: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    /// Instance buffers by tile-set id, with the frame they were last used in.
    sets: HashMap<u64, (wgpu::Buffer, u32, u64)>,
}

impl WgpuState {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dirmap tiles"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dirmap tiles"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("dirmap tiles"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dirmap tiles"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Unorm8x4, 3 => Uint32],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::default(),
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::default(),
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout, srgb: format.is_srgb(), slots: Vec::new(), sets: HashMap::new() }
    }
}

struct WgpuDraw {
    draw: Draw,
    slot: usize,
    frame: u64,
}

impl egui_wgpu::CallbackTrait for WgpuDraw {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        res: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(st) = res.get_mut::<WgpuState>() else { return Vec::new() };
        while st.slots.len() <= self.slot {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dirmap tile uniforms"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dirmap tile uniforms"),
                layout: &st.layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
            });
            st.slots.push((buf, bg));
        }
        let mut u = self.draw.uniforms;
        if st.srgb {
            u.px[3] = ((u.px[3] as u32) | SRGB_TARGET) as f32;
        }
        queue.write_buffer(&st.slots[self.slot].0, 0, bytemuck::bytes_of(&u));
        let frame = self.frame;
        if let Some(e) = st.sets.get_mut(&self.draw.id) {
            e.2 = frame;
        } else if !self.draw.instances.is_empty() {
            let bytes: &[u8] = bytemuck::cast_slice(&self.draw.instances);
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dirmap tiles"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, bytes);
            st.sets.insert(self.draw.id, (buf, self.draw.instances.len() as u32, frame));
        }
        // Free tile sets not drawn for a couple of frames.
        st.sets.retain(|_, e| e.2 + 2 >= frame);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        let Some(st) = res.get::<WgpuState>() else { return };
        let (Some((buf, count, _)), Some((_, bg))) = (st.sets.get(&self.draw.id), st.slots.get(self.slot)) else { return };
        pass.set_pipeline(&st.pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.draw(0..4, 0..*count);
    }
}

// ---------- OpenGL ----------

pub struct GlState {
    program: glow::Program,
    vao: glow::VertexArray,
    ubo: glow::Buffer,
    sets: HashMap<u64, (glow::Buffer, i32, u64)>,
}

impl GlState {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        use glow::HasContext as _;
        let version = gl.version();
        let ok = if version.is_embedded { version.major >= 3 } else { (version.major, version.minor) >= (3, 3) };
        if !ok {
            return Err(format!("needs OpenGL 3.3 or ES 3.0, have {}.{}", version.major, version.minor));
        }
        let header = if version.is_embedded { "#version 300 es\nprecision highp float;\nprecision highp int;\n" } else { "#version 330 core\n" };
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [(glow::VERTEX_SHADER, GLSL_VS), (glow::FRAGMENT_SHADER, GLSL_FS)] {
                let sh = gl.create_shader(kind)?;
                gl.shader_source(sh, &format!("{header}{src}"));
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    return Err(gl.get_shader_info_log(sh));
                }
                gl.attach_shader(program, sh);
                shaders.push(sh);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            for sh in shaders {
                gl.detach_shader(program, sh);
                gl.delete_shader(sh);
            }
            let block = gl.get_uniform_block_index(program, "U").ok_or("no uniform block")?;
            gl.uniform_block_binding(program, block, 0);
            let vao = gl.create_vertex_array()?;
            let ubo = gl.create_buffer()?;
            gl.bind_buffer(glow::UNIFORM_BUFFER, Some(ubo));
            gl.buffer_data_size(glow::UNIFORM_BUFFER, std::mem::size_of::<Uniforms>() as i32, glow::DYNAMIC_DRAW);
            gl.bind_buffer(glow::UNIFORM_BUFFER, None);
            Ok(Self { program, vao, ubo, sets: HashMap::new() })
        }
    }

    fn paint(&mut self, gl: &glow::Context, draw: &Draw, frame: u64) {
        use glow::HasContext as _;
        unsafe {
            if !self.sets.contains_key(&draw.id) {
                if draw.instances.is_empty() {
                    return;
                }
                let Ok(buf) = gl.create_buffer() else { return };
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(buf));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&draw.instances), glow::STATIC_DRAW);
                self.sets.insert(draw.id, (buf, draw.instances.len() as i32, frame));
            }
            let stale: Vec<u64> = self.sets.iter().filter(|(id, e)| **id != draw.id && e.2 + 2 < frame).map(|(id, _)| *id).collect();
            for id in stale {
                if let Some((b, _, _)) = self.sets.remove(&id) {
                    gl.delete_buffer(b);
                }
            }
            let e = self.sets.get_mut(&draw.id).unwrap();
            e.2 = frame;
            let (buf, count) = (e.0, e.1);

            gl.use_program(Some(self.program));
            gl.bind_buffer(glow::UNIFORM_BUFFER, Some(self.ubo));
            gl.buffer_sub_data_u8_slice(glow::UNIFORM_BUFFER, 0, bytemuck::bytes_of(&draw.uniforms));
            gl.bind_buffer_base(glow::UNIFORM_BUFFER, 0, Some(self.ubo));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buf));
            let stride = std::mem::size_of::<Instance>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, 16);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::UNSIGNED_BYTE, true, stride, 32);
            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_i32(3, 1, glow::UNSIGNED_INT, stride, 36);
            for a in 0..4 {
                gl.vertex_attrib_divisor(a, 1);
            }
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func_separate(glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::ONE_MINUS_DST_ALPHA, glow::ONE);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, count);
            for a in 0..4 {
                gl.vertex_attrib_divisor(a, 0);
                gl.disable_vertex_attrib_array(a);
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::UNIFORM_BUFFER, None);
            gl.use_program(None);
        }
    }
}
