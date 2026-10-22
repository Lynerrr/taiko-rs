//! wgpu renderer. Everything on screen is an instanced, analytically anti-aliased rounded box
//! (circles and capsules are rounded boxes too), so a frame is a single draw call and notes are
//! sub-pixel accurate at any resolution or scroll speed.

use std::sync::Arc;

use winit::window::Window;

use crate::font;

/// Virtual canvas height. Width follows the window aspect ratio.
pub const VIRTUAL_HEIGHT: f32 = 480.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Shape {
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub fill: [f32; 4],
    pub border: [f32; 4],
    /// x: corner radius, y: border width, z: glow/softness width, w: half clip (0 none, 1 left, 2 right).
    pub params: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentChoice {
    Mailbox,
    Immediate,
    Fifo,
}

impl PresentChoice {
    pub fn next(self) -> Self {
        match self {
            PresentChoice::Mailbox => PresentChoice::Immediate,
            PresentChoice::Immediate => PresentChoice::Fifo,
            PresentChoice::Fifo => PresentChoice::Mailbox,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            PresentChoice::Mailbox => "mailbox",
            PresentChoice::Immediate => "immediate",
            PresentChoice::Fifo => "vsync",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mailbox" => Some(PresentChoice::Mailbox),
            "immediate" => Some(PresentChoice::Immediate),
            "vsync" | "fifo" => Some(PresentChoice::Fifo),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    screen_px: [f32; 2],
    px_per_unit: f32,
    _pad: f32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: usize,
    supported_modes: Vec<wgpu::PresentMode>,
    capture_pipeline: wgpu::RenderPipeline,
    pub present: PresentChoice,
    pub adapter_name: String,
}

const SHADER: &str = r#"
struct Globals { screen_px: vec2<f32>, px_per_unit: f32, _pad: f32 };
@group(0) @binding(0) var<uniform> g: Globals;

struct Inst {
    @location(0) center: vec2<f32>,
    @location(1) half: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec4<f32>,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half: vec2<f32>,
    @location(2) @interpolate(flat) fill: vec4<f32>,
    @location(3) @interpolate(flat) border: vec4<f32>,
    @location(4) @interpolate(flat) params: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Inst) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u)) * 2.0 - 1.0;
    // Everything below is in physical pixels so anti-aliasing is exactly one pixel wide.
    let half_px = inst.half * g.px_per_unit;
    let pad = 1.5 + inst.params.z * g.px_per_unit;
    let local = corner * (half_px + vec2<f32>(pad));
    let center_px = inst.center * g.px_per_unit;
    let p = center_px + local;
    var out: VOut;
    out.pos = vec4<f32>(p.x / g.screen_px.x * 2.0 - 1.0, 1.0 - p.y / g.screen_px.y * 2.0, 0.0, 1.0);
    out.local = local;
    out.half = half_px;
    out.fill = inst.fill;
    out.border = inst.border;
    out.params = vec4<f32>(inst.params.xyz * g.px_per_unit, inst.params.w);
    return out;
}

fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let r = min(in.params.x, min(in.half.x, in.half.y));
    let d = sd_round_box(in.local, in.half, r);
    let glow = in.params.z;
    var color: vec4<f32>;
    if (glow > 0.0) {
        // Soft falloff outside the shape, used for hit flashes and kiai glow.
        let a = clamp(1.0 - d / glow, 0.0, 1.0);
        color = vec4<f32>(in.fill.rgb, in.fill.a * a * a);
    } else {
        let coverage = clamp(0.5 - d, 0.0, 1.0);
        let bw = in.params.y;
        let inner = clamp(0.5 - (d + bw), 0.0, 1.0);
        let c = mix(in.border, in.fill, select(1.0, inner, bw > 0.0));
        color = vec4<f32>(c.rgb, c.a * coverage);
    }
    let side = in.params.w;
    if (side > 0.5) {
        let dir = select(-1.0, 1.0, side > 1.5);
        color.a *= clamp(0.5 + in.local.x * dir, 0.0, 1.0);
    }
    return vec4<f32>(color.rgb * color.a, color.a);
}
"#;

impl Renderer {
    pub fn new(instance: &wgpu::Instance, window: Arc<Window>, present: PresentChoice) -> Result<Renderer, String> {
        let size = window.inner_size();
        let surface = instance.create_surface(window).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let adapter_name = format!("{} ({:?})", adapter.get_info().name, adapter.get_info().backend);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("taiko"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let alpha_mode = caps.alpha_modes[0];
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface not supported by adapter")?;
        config.format = format;
        config.alpha_mode = alpha_mode;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shapes"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4];
        let make_pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shapes"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Shape>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
        };
        let pipeline = make_pipeline(format);
        // Screenshots render into an offscreen target, independent of what the compositor allows.
        let capture_pipeline = make_pipeline(wgpu::TextureFormat::Rgba8Unorm);
        let capacity = 4096;
        let instances = Self::make_instance_buffer(&device, capacity);

        let mut r = Renderer {
            surface,
            device,
            queue,
            config,
            pipeline,
            globals,
            bind_group,
            instances,
            capacity,
            supported_modes: caps.present_modes,
            capture_pipeline,
            present,
            adapter_name,
        };
        r.set_present(present);
        Ok(r)
    }

    fn make_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (capacity * std::mem::size_of::<Shape>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Applies a present mode, falling back toward vsync if the compositor lacks it.
    pub fn set_present(&mut self, want: PresentChoice) {
        let order: &[PresentChoice] = match want {
            PresentChoice::Mailbox => &[PresentChoice::Mailbox, PresentChoice::Immediate, PresentChoice::Fifo],
            PresentChoice::Immediate => &[PresentChoice::Immediate, PresentChoice::Mailbox, PresentChoice::Fifo],
            PresentChoice::Fifo => &[PresentChoice::Fifo],
        };
        let to_wgpu = |p: PresentChoice| match p {
            PresentChoice::Mailbox => wgpu::PresentMode::Mailbox,
            PresentChoice::Immediate => wgpu::PresentMode::Immediate,
            PresentChoice::Fifo => wgpu::PresentMode::Fifo,
        };
        let chosen = order.iter().copied().find(|p| self.supported_modes.contains(&to_wgpu(*p))).unwrap_or(PresentChoice::Fifo);
        self.present = chosen;
        self.config.present_mode = to_wgpu(chosen);
        // One frame in flight for the uncapped modes keeps input-to-photon latency minimal.
        self.config.desired_maximum_frame_latency = if chosen == PresentChoice::Fifo { 2 } else { 1 };
        self.surface.configure(&self.device, &self.config);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Canvas width in virtual units.
    pub fn virtual_width(&self) -> f32 {
        VIRTUAL_HEIGHT * self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// Waits for a swapchain image. Under vsync this is where the frame may block, so callers
    /// should sample the game clock only after it returns.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => Some(f),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                None
            }
            _ => None,
        }
    }

    /// Draws `shapes` into an acquired frame and presents it; with `capture`, also returns the
    /// frame as tightly packed RGB.
    pub fn submit(&mut self, frame: wgpu::SurfaceTexture, shapes: &[Shape], clear: [f64; 3], capture: bool) -> Option<(u32, u32, Vec<u8>)> {
        if shapes.len() > self.capacity {
            self.capacity = shapes.len().next_power_of_two();
            self.instances = Self::make_instance_buffer(&self.device, self.capacity);
        }
        let globals = Globals {
            screen_px: [self.config.width as f32, self.config.height as f32],
            px_per_unit: self.config.height as f32 / VIRTUAL_HEIGHT,
            _pad: 0.0,
        };
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        if !shapes.is_empty() {
            self.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(shapes));
        }
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        self.encode_pass(&mut encoder, &view, &self.pipeline, shapes.len(), clear);

        let (w, h) = (self.config.width, self.config.height);
        let readback = capture.then(|| {
            let target = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("capture"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
            self.encode_pass(&mut encoder, &target_view, &self.capture_pipeline, shapes.len(), clear);
            let bpr = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("capture"),
                size: (bpr * h) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: None },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            (buf, bpr)
        });
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);

        let (buf, bpr) = readback?;
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok()?;
        let data = buf.slice(..).get_mapped_range().ok()?;
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for row in data.chunks_exact(bpr as usize) {
            for px in row[..(w * 4) as usize].chunks_exact(4) {
                rgb.extend_from_slice(&px[..3]);
            }
        }
        Some((w, h, rgb))
    }

    fn encode_pass(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, count: usize, clear: [f64; 3]) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0], g: clear[1], b: clear[2], a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..4, 0..count as u32);
    }
}

/// Immediate-mode drawing helpers. Coordinates are virtual units.
#[derive(Default)]
pub struct Canvas {
    pub shapes: Vec<Shape>,
    pub width: f32,
}

pub fn rgba(hex: u32, a: f32) -> [f32; 4] {
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0, a]
}

const CLEAR: [f32; 4] = [0.0; 4];

impl Canvas {
    pub fn clear(&mut self, width: f32) {
        self.shapes.clear();
        self.width = width;
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: [f32; 4]) {
        self.shapes.push(Shape { center: [x + w / 2.0, y + h / 2.0], half: [w / 2.0, h / 2.0], fill, border: fill, params: [0.0; 4] });
    }

    pub fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, fill: [f32; 4], border: [f32; 4], border_w: f32) {
        self.shapes.push(Shape {
            center: [x + w / 2.0, y + h / 2.0],
            half: [w / 2.0, h / 2.0],
            fill,
            border,
            params: [radius, border_w, 0.0, 0.0],
        });
    }

    pub fn circle(&mut self, x: f32, y: f32, r: f32, fill: [f32; 4], border: [f32; 4], border_w: f32) {
        self.shapes.push(Shape { center: [x, y], half: [r, r], fill, border, params: [r, border_w, 0.0, 0.0] });
    }

    pub fn ring(&mut self, x: f32, y: f32, r: f32, color: [f32; 4], width: f32) {
        self.circle(x, y, r, CLEAR, color, width);
    }

    /// Horizontal capsule from x0 to x1 (centres of the end caps).
    pub fn capsule(&mut self, x0: f32, x1: f32, y: f32, r: f32, fill: [f32; 4], border: [f32; 4], border_w: f32) {
        let (a, b) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        self.shapes.push(Shape {
            center: [(a + b) / 2.0, y],
            half: [(b - a) / 2.0 + r, r],
            fill,
            border,
            params: [r, border_w, 0.0, 0.0],
        });
    }

    /// Left (`side` 1) or right (`side` 2) half of a circle.
    pub fn half_circle(&mut self, x: f32, y: f32, r: f32, side: u8, fill: [f32; 4], border: [f32; 4], border_w: f32) {
        self.shapes.push(Shape { center: [x, y], half: [r, r], fill, border, params: [r, border_w, 0.0, side as f32] });
    }

    pub fn glow(&mut self, x: f32, y: f32, r: f32, softness: f32, color: [f32; 4]) {
        self.shapes.push(Shape { center: [x, y], half: [r, r], fill: color, border: color, params: [r, 0.0, softness, 0.0] });
    }

    /// Draws text with the built-in 5x7 font; `size` is the glyph height. Returns the advance width.
    pub fn text(&mut self, x: f32, y: f32, size: f32, color: [f32; 4], s: &str) -> f32 {
        let px = size / 7.0;
        let mut cx = x;
        for ch in s.chars() {
            let cols = font::glyph(ch);
            for (c, bits) in cols.iter().enumerate() {
                let mut row = 0;
                while row < 7 {
                    if bits & (1 << row) != 0 {
                        // Merge vertical runs into one rect to keep instance counts low.
                        let start = row;
                        while row < 7 && bits & (1 << row) != 0 {
                            row += 1;
                        }
                        let len = (row - start) as f32;
                        self.shapes.push(Shape {
                            center: [cx + (c as f32 + 0.5) * px, y + (start as f32 + len / 2.0) * px],
                            half: [px * 0.5, px * len * 0.5],
                            fill: color,
                            border: color,
                            params: [0.0; 4],
                        });
                    } else {
                        row += 1;
                    }
                }
            }
            cx += px * 6.0;
        }
        cx - x
    }

    pub fn text_width(size: f32, s: &str) -> f32 {
        s.chars().count() as f32 * size / 7.0 * 6.0
    }

    pub fn text_centered(&mut self, cx: f32, y: f32, size: f32, color: [f32; 4], s: &str) {
        let w = Self::text_width(size, s);
        self.text(cx - w / 2.0, y, size, color, s);
    }

    pub fn text_right(&mut self, right: f32, y: f32, size: f32, color: [f32; 4], s: &str) {
        let w = Self::text_width(size, s);
        self.text(right - w, y, size, color, s);
    }
}
