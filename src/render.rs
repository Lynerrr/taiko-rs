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
    /// x: corner radius, y: border width, z: glow/softness width, w: half clip (0 none, 1 left, 2 right)
    /// or a skin image (`SPRITE_BASE` + `Sprite`, tinted by `fill`).
    pub params: [f32; 4],
}

/// Skin images, packed into one texture so they draw in the same instanced call as the shapes.
#[derive(Clone, Copy, Debug)]
pub enum Sprite {
    HitCircle,
    HitCircleOverlay,
    BigCircle,
    BigCircleOverlay,
}

pub const SPRITES: usize = 4;
const SPRITE_BASE: f32 = 10.0;

/// Skin images side by side (premultiplied RGBA8), each with its UV rect and its size relative
/// to the note radius. The default (no images) brings back the drawn notes.
#[derive(Default)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub sprites: [Option<([f32; 4], [f32; 2])>; SPRITES],
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
    /// The shapes' bind group: globals, then the skin atlas (its UV rects, texture and sampler).
    shape_layout: wgpu::BindGroupLayout,
    sprite_rects: wgpu::Buffer,
    sampler: wgpu::Sampler,
    instances: wgpu::Buffer,
    capacity: usize,
    supported_modes: Vec<wgpu::PresentMode>,
    capture_pipeline: wgpu::RenderPipeline,
    /// Map background: drawn as a full-screen "cover" image under the shapes.
    bg_pipeline: wgpu::RenderPipeline,
    bg_capture_pipeline: wgpu::RenderPipeline,
    bg_layout: wgpu::BindGroupLayout,
    bg_sampler: wgpu::Sampler,
    /// Bind group and image aspect (w / h) of the current background.
    bg: Option<BgLayer>,
    /// The background being faded out while `bg` fades in.
    bg_prev: Option<BgLayer>,
    /// Progress of the cross-fade from `bg_prev` to `bg` (0 to 1).
    pub bg_mix: f32,
    /// 0 hides the background, 1 shows it at full brightness.
    pub bg_brightness: f32,
    pub present: PresentChoice,
    pub adapter_name: String,
}

/// A background image on the GPU.
struct BgLayer {
    bind_group: wgpu::BindGroup,
    aspect: f32,
    uniform: wgpu::Buffer,
}

const BG_SHADER: &str = r#"
struct Bg { uv: vec4<f32>, params: vec4<f32> };
@group(0) @binding(0) var<uniform> bg: Bg;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct V { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> V {
    let c = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    var o: V;
    o.pos = vec4<f32>(c.x * 2.0 - 1.0, 1.0 - c.y * 2.0, 0.0, 1.0);
    o.uv = mix(bg.uv.xy, bg.uv.zw, c);
    return o;
}

@fragment
fn fs(i: V) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, i.uv);
    // params: brightness, opacity (for cross-fading into a new background).
    return vec4<f32>(t.rgb * bg.params.x, bg.params.y);
}
"#;

const SHADER: &str = r#"
struct Globals { screen_px: vec2<f32>, px_per_unit: f32, _pad: f32 };
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<uniform> sprites: array<vec4<f32>, 4>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

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
    if (in.params.w > 9.5) {
        // A skin image (premultiplied), tinted by fill.
        let rect = sprites[min(u32(in.params.w - 9.5), 3u)];
        let uv = in.local / (2.0 * in.half) + 0.5;
        let t = textureSampleLevel(atlas, samp, rect.xy + clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * rect.zw, 0.0);
        let inside = select(0.0, 1.0, all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)));
        return vec4<f32>(t.rgb * in.fill.rgb, t.a) * in.fill.a * inside;
    }
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
        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                uniform(0, wgpu::ShaderStages::VERTEX),
                uniform(1, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sprite_rects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprites"),
            size: (SPRITES * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = Self::shape_bind_group(&device, &queue, &bgl, &globals, &sprite_rects, &sampler, &Atlas::default());
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
        // Background pipelines (one per target format, like the shape pipelines).
        let bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("background"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
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
        let bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("background"),
            source: wgpu::ShaderSource::Wgsl(BG_SHADER.into()),
        });
        let bg_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bg_layout)],
            immediate_size: 0,
        });
        let make_bg_pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("background"),
                layout: Some(&bg_pipeline_layout),
                vertex: wgpu::VertexState { module: &bg_shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &bg_shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let bg_pipeline = make_bg_pipeline(format);
        let bg_capture_pipeline = make_bg_pipeline(wgpu::TextureFormat::Rgba8Unorm);
        let bg_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("background"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
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
            shape_layout: bgl,
            sprite_rects,
            sampler,
            instances,
            capacity,
            supported_modes: caps.present_modes,
            capture_pipeline,
            bg_pipeline,
            bg_capture_pipeline,
            bg_layout,
            bg_sampler,
            bg: None,
            bg_prev: None,
            bg_mix: 1.0,
            bg_brightness: 0.0,
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

    /// Uploads a skin's images; an empty atlas removes them.
    pub fn set_sprites(&mut self, atlas: &Atlas) {
        self.bind_group = Self::shape_bind_group(&self.device, &self.queue, &self.shape_layout, &self.globals, &self.sprite_rects, &self.sampler, atlas);
    }

    fn shape_bind_group(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        globals: &wgpu::Buffer,
        rects: &wgpu::Buffer,
        sampler: &wgpu::Sampler,
        atlas: &Atlas,
    ) -> wgpu::BindGroup {
        // The texture can't be empty: no images is one transparent pixel.
        let (w, h, data) = if atlas.rgba.len() == (atlas.width * atlas.height * 4) as usize && atlas.width > 0 && atlas.height > 0 {
            (atlas.width, atlas.height, atlas.rgba.as_slice())
        } else {
            (1, 1, &[0u8; 4][..])
        };
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("skin"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            size,
        );
        let uv: [[f32; 4]; SPRITES] = atlas.sprites.map(|s| s.map(|(uv, _)| uv).unwrap_or_default());
        queue.write_buffer(rects, 0, bytemuck::cast_slice(&uv));
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: rects.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    /// Replaces the background image (RGBA8, `w` x `h`), or removes it with `None`. The old one
    /// stays drawn underneath while the new one fades in over it (`bg_mix`).
    pub fn set_background(&mut self, image: Option<(u32, u32, Vec<u8>)>) {
        if let Some(old) = self.bg.take() {
            self.bg_prev = Some(old);
        }
        let Some((w, h, data)) = image.filter(|(w, h, d)| *w > 0 && *h > 0 && d.len() == (*w * *h * 4) as usize) else {
            return;
        };
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("background"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // The image's sRGB bytes go straight to the (non-sRGB) surface, like shape colours.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Each layer has its own uniform: its crop depends on its own aspect.
        let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("background"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("background"),
            layout: &self.bg_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.bg_sampler) },
            ],
        });
        self.bg = Some(BgLayer { bind_group, aspect: w as f32 / h as f32, uniform });
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
    /// `clip`: shapes in that range are clipped to a screen rectangle given as fractions
    /// (x, y, w, h) — the playfield area — so notes can't spill outside it.
    pub fn submit(
        &mut self,
        frame: wgpu::SurfaceTexture,
        shapes: &[Shape],
        clear: [f64; 3],
        capture: bool,
        clips: Vec<(std::ops::Range<usize>, [f32; 4])>,
    ) -> Option<(u32, u32, Vec<u8>)> {
        // Clipped shape ranges (rects as fractions of the screen) -> pixel scissors, in order,
        // without overlaps.
        let mut clip: Vec<(std::ops::Range<usize>, [u32; 4])> = clips
            .into_iter()
            .filter(|(range, _)| range.start < range.end && range.end <= shapes.len())
            .map(|(range, r)| {
                let (sw, sh) = (self.config.width as f32, self.config.height as f32);
                let x = (r[0] * sw).floor().clamp(0.0, sw - 1.0);
                let y = (r[1] * sh).floor().clamp(0.0, sh - 1.0);
                let w = (r[2] * sw).ceil().clamp(1.0, sw - x);
                let h = (r[3] * sh).ceil().clamp(1.0, sh - y);
                (range, [x as u32, y as u32, w as u32, h as u32])
            })
            .collect();
        clip.sort_by_key(|(range, _)| range.start);
        let mut end = 0;
        clip.retain(|(range, _)| {
            let ok = range.start >= end;
            if ok {
                end = range.end;
            }
            ok
        });
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
        if self.bg_mix >= 1.0 {
            self.bg_prev = None;
        }
        // The old background stays opaque under the new one (or fades out if there's none).
        let mix = self.bg_mix.clamp(0.0, 1.0);
        let prev_alpha = if self.bg.is_some() { 1.0 } else { 1.0 - mix };
        let screen = self.config.width as f32 / self.config.height.max(1) as f32;
        for (layer, alpha) in [(&self.bg_prev, prev_alpha), (&self.bg, mix)] {
            if let Some(l) = layer {
                // "Cover" fit: crop the image's long side so it fills the screen without stretching.
                let (u, v) = if l.aspect > screen { (screen / l.aspect, 1.0) } else { (1.0, l.aspect / screen) };
                let uniform = [(1.0 - u) / 2.0, (1.0 - v) / 2.0, (1.0 + u) / 2.0, (1.0 + v) / 2.0, self.bg_brightness, alpha, 0.0, 0.0];
                self.queue.write_buffer(&l.uniform, 0, bytemuck::cast_slice(&uniform));
            }
        }
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        self.encode_pass(&mut encoder, &view, false, shapes.len(), clear, &clip);

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
            self.encode_pass(&mut encoder, &target_view, true, shapes.len(), clear, &clip);
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

    /// Renders shapes (with the background) into an offscreen `w` x `h` image and reads it back as
    /// RGBA rows. `clips` are rects as fractions of the image. Used for video frames.
    pub fn render_offscreen(&mut self, shapes: &[Shape], clips: &[(std::ops::Range<usize>, [f32; 4])], w: u32, h: u32, clear: [f64; 3]) -> Option<Vec<u8>> {
        let clip: Vec<(std::ops::Range<usize>, [u32; 4])> = clips
            .iter()
            .filter(|(range, _)| range.start < range.end && range.end <= shapes.len())
            .map(|(range, r)| {
                let (sw, sh) = (w as f32, h as f32);
                let x = (r[0] * sw).floor().clamp(0.0, sw - 1.0);
                let y = (r[1] * sh).floor().clamp(0.0, sh - 1.0);
                let cw = (r[2] * sw).ceil().clamp(1.0, sw - x);
                let ch = (r[3] * sh).ceil().clamp(1.0, sh - y);
                (range.clone(), [x as u32, y as u32, cw as u32, ch as u32])
            })
            .collect();
        if shapes.len() > self.capacity {
            self.capacity = shapes.len().next_power_of_two();
            self.instances = Self::make_instance_buffer(&self.device, self.capacity);
        }
        let globals = Globals { screen_px: [w as f32, h as f32], px_per_unit: h as f32 / VIRTUAL_HEIGHT, _pad: 0.0 };
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        if !shapes.is_empty() {
            self.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(shapes));
        }
        let screen = w as f32 / h.max(1) as f32;
        if let Some(l) = &self.bg {
            let (u, v) = if l.aspect > screen { (screen / l.aspect, 1.0) } else { (1.0, l.aspect / screen) };
            let uniform = [(1.0 - u) / 2.0, (1.0 - v) / 2.0, (1.0 + u) / 2.0, (1.0 + v) / 2.0, self.bg_brightness, 1.0, 0.0, 0.0];
            self.queue.write_buffer(&l.uniform, 0, bytemuck::cast_slice(&uniform));
        }
        // Only the current background (no cross-fade) in a video frame.
        let prev = self.bg_prev.take();
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        self.encode_pass(&mut encoder, &view, true, shapes.len(), clear, &clip);
        self.bg_prev = prev;
        let bpr = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen"),
            size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: None } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok()?;
        let data = buf.slice(..).get_mapped_range().ok()?;
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for row in data.chunks_exact(bpr as usize) {
            rgba.extend_from_slice(&row[..(w * 4) as usize]);
        }
        Some(rgba)
    }

    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        capture: bool,
        count: usize,
        clear: [f64; 3],
        clips: &[(std::ops::Range<usize>, [u32; 4])],
    ) {
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
        if self.bg_brightness > 0.0 {
            for layer in [&self.bg_prev, &self.bg].into_iter().flatten() {
                pass.set_pipeline(if capture { &self.bg_capture_pipeline } else { &self.bg_pipeline });
                pass.set_bind_group(0, &layer.bind_group, &[]);
                pass.draw(0..4, 0..1);
            }
        }
        pass.set_pipeline(if capture { &self.capture_pipeline } else { &self.pipeline });
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        // Unclipped runs between the clipped ranges.
        let mut at = 0u32;
        for (range, [x, y, w, h]) in clips {
            if (range.start as u32) > at {
                pass.draw(0..4, at..range.start as u32);
            }
            pass.set_scissor_rect(*x, *y, *w, *h);
            pass.draw(0..4, range.start as u32..range.end as u32);
            pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
            at = range.end as u32;
        }
        if (count as u32) > at {
            pass.draw(0..4, at..count as u32);
        }
    }
}

static ANIMATIONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// UI animations on or off (Settings -> Video). Off, every animation shows its end state.
pub fn set_animations(on: bool) {
    ANIMATIONS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn animations() -> bool {
    ANIMATIONS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Moves `value` toward `target`, covering ~63% of the way every `tau_ms` (jumps there with
/// animations off).
pub fn approach(value: &mut f32, target: f32, dt_ms: f32, tau_ms: f32) {
    if !animations() || !value.is_finite() {
        *value = target;
    } else {
        *value += (target - *value) * (1.0 - (-dt_ms / tau_ms).exp());
    }
}

/// Eased-out progress (0 to 1) of an animation started at `since` lasting `ms`; 1 with
/// animations off.
pub fn progress(since: std::time::Instant, ms: f32) -> f32 {
    if !animations() {
        return 1.0;
    }
    let t = (since.elapsed().as_secs_f32() * 1000.0 / ms).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Something clickable drawn this frame, for mouse navigation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// A button that does what this key does.
    Key(winit::keyboard::KeyCode),
    /// Top bar tab: 0 songs, 1 settings, 2 multiplayer.
    Tab(usize),
    /// A row of the song list (index into its visible list): a difficulty, or a song's own row
    /// (pointing at its first difficulty).
    SongRow(usize),
    SongHeader(usize),
    /// An item of the song list's right-click menu.
    SongMenu(usize),
    SettingRow(usize),
    /// The `<` (-1) / `>` (+1) of the selected settings row.
    SettingStep(usize, i32),
    PauseItem(usize),
    MpRow(usize),
    /// Opens the playfield preset list.
    Presets,
    /// A download tab result row / its Download button.
    DlRow(usize),
    DlGet(usize),
    /// Song select: next sort mode / open the collection list / a row of that list.
    SortCycle,
    CollectionPick,
    PickRow(usize),
    /// A row of the settings skin list.
    SkinRow(usize),
    /// A settings sidebar section.
    SettingsSection(usize),
    /// The update badge: install the new build.
    Update,
    /// Download tab status category / sort chips.
    DlCategory(usize),
    DlSort(usize),
    /// A row of the song select leaderboard (watches that replay) / its video button.
    ScoreRow(usize),
    ScoreVideo(usize),
    /// Lobby: give the host role to this player.
    GiveHost(u32),
    /// The friends overlay: its chip, its panel (swallows clicks), a row, a row's button, and a
    /// notification card.
    SocialToggle,
    SocialPanel,
    SocialRow(usize),
    SocialAction(usize, crate::social::Action),
    SocialNote(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct HitBox {
    pub rect: [f32; 4],
    pub hit: Hit,
    /// Shapes drawn before it, so a transform of later shapes (the playfield area) moves it too.
    shape: usize,
}

impl HitBox {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let [rx, ry, rw, rh] = self.rect;
        x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
    }
}

/// Immediate-mode drawing helpers. Coordinates are virtual units.
#[derive(Default)]
pub struct Canvas {
    pub shapes: Vec<Shape>,
    pub width: f32,
    /// Clickable areas, in drawing order (later ones are on top).
    pub hits: Vec<HitBox>,
    /// Pointer position (virtual units), for hover highlights.
    pub mouse: (f32, f32),
    /// Time since the previous frame (ms), for animations.
    pub dt_ms: f32,
    /// Shape ranges clipped to a rect (virtual units), e.g. scrolling lists.
    pub clips: Vec<(std::ops::Range<usize>, [f32; 4])>,
    /// Size (relative to the radius) of each skin image the renderer has; None = not in the skin.
    pub sprites: [Option<[f32; 2]>; SPRITES],
}

pub fn rgba(hex: u32, a: f32) -> [f32; 4] {
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0, a]
}

const CLEAR: [f32; 4] = [0.0; 4];

impl Canvas {
    pub fn clear(&mut self, width: f32) {
        self.shapes.clear();
        self.hits.clear();
        self.clips.clear();
        self.width = width;
    }

    /// Clips everything drawn since shape `start` to a rect (virtual units).
    pub fn clip_since(&mut self, start: usize, x: f32, y: f32, w: f32, h: f32) {
        let end = self.shapes.len();
        if end > start {
            self.clips.push((start..end, [x, y, w, h]));
        }
    }

    /// Makes a rect clickable. Call after drawing at least one of its shapes, so it belongs to
    /// the same transformed region.
    pub fn hit(&mut self, x: f32, y: f32, w: f32, h: f32, hit: Hit) {
        self.hits.push(HitBox { rect: [x, y, w, h], hit, shape: self.shapes.len() });
    }

    pub fn hovered(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let (mx, my) = self.mouse;
        mx >= x && mx <= x + w && my >= y && my <= y + h
    }

    /// A clickable button, highlighted under the pointer. `accent`: the main action.
    #[allow(clippy::too_many_arguments)]
    pub fn button(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str, size: f32, hit: Hit, accent: bool) {
        let hover = self.hovered(x, y, w, h);
        let (bg, border) = match (accent, hover) {
            (true, false) => (0x2b1a17, 0xEB452C),
            (true, true) => (0x4a221b, 0xFF6A50),
            (false, false) => (0x23232f, 0x3a3a4e),
            (false, true) => (0x34344a, 0x6a6a88),
        };
        self.round_rect(x, y, w, h, 6.0, rgba(bg, 1.0), rgba(border, 1.0), 1.0);
        self.text_centered(x + w / 2.0, y + (h - size) / 2.0, size, rgba(0xffffff, 1.0), label);
        self.hit(x, y, w, h, hit);
    }

    /// A centred row of buttons: (label, hit, accent).
    pub fn button_row(&mut self, y: f32, buttons: &[(&str, Hit, bool)]) {
        const SIZE: f32 = 8.0;
        const GAP: f32 = 8.0;
        let widths: Vec<f32> = buttons.iter().map(|(l, _, _)| Canvas::text_width(SIZE, l) + 28.0).collect();
        let total = widths.iter().sum::<f32>() + GAP * (buttons.len().saturating_sub(1)) as f32;
        let mut x = (self.width - total) / 2.0;
        for ((label, hit, accent), w) in buttons.iter().zip(widths) {
            self.button(x, y, w, 20.0, label, SIZE, *hit, *accent);
            x += w + GAP;
        }
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

    /// A skin image centred on (`x`, `y`) for something of radius `r`, tinted by `tint`. False
    /// (nothing drawn) when the skin doesn't have it.
    pub fn sprite(&mut self, sprite: Sprite, x: f32, y: f32, r: f32, tint: [f32; 4]) -> bool {
        let Some([sx, sy]) = self.sprites[sprite as usize] else { return false };
        self.shapes.push(Shape { center: [x, y], half: [r * sx, r * sy], fill: tint, border: tint, params: [0.0, 0.0, 0.0, SPRITE_BASE + sprite as usize as f32] });
        true
    }

    /// Scales every shape drawn since `start` about `anchor`, then moves it by (`dx`, `dy`). Radii,
    /// borders and glows scale with it, so the result stays crisp (used for the playfield area).
    pub fn transform_since(&mut self, start: usize, anchor: [f32; 2], scale: f32, dx: f32, dy: f32) {
        for sh in &mut self.shapes[start..] {
            sh.center = [anchor[0] + (sh.center[0] - anchor[0]) * scale + dx, anchor[1] + (sh.center[1] - anchor[1]) * scale + dy];
            sh.half = [sh.half[0] * scale, sh.half[1] * scale];
            sh.params[0] *= scale;
            sh.params[1] *= scale;
            sh.params[2] *= scale;
        }
        for h in self.hits.iter_mut().filter(|h| h.shape > start) {
            let [x, y, w, hh] = h.rect;
            h.rect = [anchor[0] + (x - anchor[0]) * scale + dx, anchor[1] + (y - anchor[1]) * scale + dy, w * scale, hh * scale];
        }
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
