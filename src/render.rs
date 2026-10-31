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
    /// or a skin image (`SPRITE_BASE` + `Sprite`, tinted by `fill`) or a character of the font
    /// (`GLYPH_BASE` + `font::index`, in `fill`, filling the quad's 5x7 font pixels).
    pub params: [f32; 4],
    /// Rotation (radians, clockwise on screen) about the centre; 0 for nearly everything.
    pub angle: f32,
}

/// Text: one quad per character, its pixels read from the font image in the shader.
const GLYPH_BASE: f32 = 100.0;

/// Skin images, packed into one texture so they draw in the same instanced call as the shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sprite {
    HitCircle,
    HitCircleOverlay,
    BigCircle,
    BigCircleOverlay,
    /// A drumroll's body, stretched along it.
    RollMiddle,
    /// A drumroll's end cap, just past its end.
    RollEnd,
    /// A spinner on its way in.
    SpinnerWarning,
    /// A spinner being hit, over the drum.
    SpinnerCircle,
    /// A ring closing in on the spinner circle until time runs out.
    SpinnerApproach,
    /// A drumroll's tick.
    RollTick,
    /// The lane, and its glow in kiai time.
    LaneBar,
    LaneKiai,
    Barline,
    /// The hit target: the ring notes are hit in, and the big one around it.
    HitTarget,
    HitTargetBig,
    /// The panel the drum sits in, and the drum's halves lit when hit: as the skin has them (the
    /// inner one's left, the outer one's right), and their mirror images.
    DrumPanel,
    DrumInner,
    DrumInnerFlipped,
    DrumOuter,
    DrumOuterFlipped,
    /// GREAT / OK / MISS.
    JudgeGreat,
    JudgeOk,
    JudgeMiss,
    /// The score font's digits, % and decimal point (score and accuracy).
    Score0,
    Score1,
    Score2,
    Score3,
    Score4,
    Score5,
    Score6,
    Score7,
    Score8,
    Score9,
    ScorePercent,
    ScoreDot,
    /// The combo font's digits (the combo on the drum).
    Combo0,
    Combo1,
    Combo2,
    Combo3,
    Combo4,
    Combo5,
    Combo6,
    Combo7,
    Combo8,
    Combo9,
    /// The health bar: behind, and the fill.
    HealthBg,
    HealthFill,
}

/// How many `Sprite`s there are (the shader's `sprites` array is this long too).
pub const SPRITES: usize = 47;

impl Sprite {
    /// Every sprite, in order.
    pub const ALL: [Sprite; SPRITES] = [
        Sprite::HitCircle,
        Sprite::HitCircleOverlay,
        Sprite::BigCircle,
        Sprite::BigCircleOverlay,
        Sprite::RollMiddle,
        Sprite::RollEnd,
        Sprite::SpinnerWarning,
        Sprite::SpinnerCircle,
        Sprite::SpinnerApproach,
        Sprite::RollTick,
        Sprite::LaneBar,
        Sprite::LaneKiai,
        Sprite::Barline,
        Sprite::HitTarget,
        Sprite::HitTargetBig,
        Sprite::DrumPanel,
        Sprite::DrumInner,
        Sprite::DrumInnerFlipped,
        Sprite::DrumOuter,
        Sprite::DrumOuterFlipped,
        Sprite::JudgeGreat,
        Sprite::JudgeOk,
        Sprite::JudgeMiss,
        Sprite::Score0,
        Sprite::Score1,
        Sprite::Score2,
        Sprite::Score3,
        Sprite::Score4,
        Sprite::Score5,
        Sprite::Score6,
        Sprite::Score7,
        Sprite::Score8,
        Sprite::Score9,
        Sprite::ScorePercent,
        Sprite::ScoreDot,
        Sprite::Combo0,
        Sprite::Combo1,
        Sprite::Combo2,
        Sprite::Combo3,
        Sprite::Combo4,
        Sprite::Combo5,
        Sprite::Combo6,
        Sprite::Combo7,
        Sprite::Combo8,
        Sprite::Combo9,
        Sprite::HealthBg,
        Sprite::HealthFill,
    ];

    /// The score font's glyph for `ch` (digits, '%' and '.').
    pub fn score_glyph(ch: char) -> Option<Sprite> {
        match ch {
            '0'..='9' => Some(Sprite::ALL[Sprite::Score0 as usize + ch as usize - '0' as usize]),
            '%' => Some(Sprite::ScorePercent),
            '.' => Some(Sprite::ScoreDot),
            _ => None,
        }
    }

    /// The combo font's glyph for `ch` (digits).
    pub fn combo_glyph(ch: char) -> Option<Sprite> {
        ch.to_digit(10).map(|d| Sprite::ALL[Sprite::Combo0 as usize + d as usize])
    }
}
const SPRITE_BASE: f32 = 10.0;

/// What a shape needs, besides its position, to come out mirrored left to right: half circles
/// swap sides, and a drumroll's end cap turns around (the other sprites are round).
fn flip_x(sh: &mut Shape) {
    let sprite = |s: Sprite| SPRITE_BASE + s as usize as f32;
    // The drum's halves have their mirror images in the atlas.
    let pairs = [(Sprite::DrumInner, Sprite::DrumInnerFlipped), (Sprite::DrumOuter, Sprite::DrumOuterFlipped)];
    // Turned the other way in the mirror (the pieces of a bent lane: left as they were, its
    // bends came out turned against the lane and cut into it).
    sh.angle = -sh.angle;
    if (0.5..2.5).contains(&sh.params[3]) {
        sh.params[3] = 3.0 - sh.params[3];
    } else if sh.params[3] == sprite(Sprite::RollEnd) {
        sh.angle += std::f32::consts::PI;
    } else if let Some(&(a, b)) = pairs.iter().find(|&&(a, b)| sh.params[3] == sprite(a) || sh.params[3] == sprite(b)) {
        sh.params[3] = if sh.params[3] == sprite(a) { sprite(b) } else { sprite(a) };
    }
}

/// What a skin says about laying its images out, besides their sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkinLayout {
    /// How far (SD px) each digit of the score and combo fonts overlaps the one before.
    pub score_overlap: f32,
    pub combo_overlap: f32,
    /// The drum skin is older than version 2.1: its halves are smaller and sit elsewhere.
    pub legacy_drum: bool,
}

/// A glyph of a skin font and its size (relative, like `Canvas::sprites`).
type Glyph = (Sprite, [f32; 2]);

/// A skin's number fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    /// Score and accuracy (digits, % and .).
    Score,
    /// The combo (digits).
    Combo,
}

/// Something for each `Sprite` (indexed by `sprite as usize`), defaulting to `T`'s default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerSprite<T>(pub [T; SPRITES]);

impl<T: Default + Copy> Default for PerSprite<T> {
    fn default() -> Self {
        PerSprite([T::default(); SPRITES])
    }
}

impl<T> std::ops::Deref for PerSprite<T> {
    type Target = [T; SPRITES];
    fn deref(&self) -> &[T; SPRITES] {
        &self.0
    }
}

impl<T> std::ops::DerefMut for PerSprite<T> {
    fn deref_mut(&mut self) -> &mut [T; SPRITES] {
        &mut self.0
    }
}

/// Skin images side by side (premultiplied RGBA8), each with its UV rect and its size relative
/// to the note radius. The default (no images) brings back the drawn look.
#[derive(Default)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub sprites: PerSprite<Option<([f32; 4], [f32; 2])>>,
    pub layout: SkinLayout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentChoice {
    Mailbox,
    Immediate,
    Fifo,
}

impl PresentChoice {
    /// In the order the settings list shows them.
    pub const ALL: [PresentChoice; 3] = [PresentChoice::Fifo, PresentChoice::Mailbox, PresentChoice::Immediate];

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
    /// None while there is no window to draw to (Android destroys it in the background).
    surface: Option<wgpu::Surface<'static>>,
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
    /// The font's glyphs (`font::atlas`), for text quads.
    font_view: wgpu::TextureView,
    instances: wgpu::Buffer,
    capacity: usize,
    supported_modes: Vec<wgpu::PresentMode>,
    capture_pipeline: wgpu::RenderPipeline,
    /// Map background: drawn as a full-screen "cover" image under the shapes.
    /// Indexed [capture][opaque]: an opaque layer is drawn without blending, so the GPU doesn't
    /// read back the pixels under it.
    bg_pipelines: [[wgpu::RenderPipeline; 2]; 2],
    disc_pipelines: [wgpu::RenderPipeline; 2],
    /// The music player's disc this frame: centre and radius in virtual units, its angle, and
    /// how wide the virtual screen is.
    pub disc: Option<([f32; 4], f32)>,
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
    /// 2: draw at half the screen's resolution, then scale up exactly 2x (crisp pixels, a
    /// quarter of the work); 1: full resolution.
    pub pixel_scale: u32,
    /// The last image came Suboptimal: configure the surface again before the next.
    reconfigure: bool,
    /// The half-resolution image and its bind group for the upscale, sized for the screen.
    low: Option<LowRes>,
    upscale_layout: wgpu::BindGroupLayout,
    upscale_pipeline: wgpu::RenderPipeline,
    /// TAIKO_PROFILE: how long the GPU spends on each frame (None if it can't tell).
    gpu_timer: Option<GpuTimer>,
}

/// GPU time of the frame's passes from timestamp queries, read back a few frames later so
/// nothing waits for them.
struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    /// Readback buffers, each with its state: 0 free, 1 mapping, 2 ready to read.
    slots: Vec<(wgpu::Buffer, Arc<std::sync::atomic::AtomicU8>)>,
    /// The slot this frame writes to (None: all busy, this frame isn't timed).
    current: Option<usize>,
    period_ns: f64,
    total_ms: f64,
    frames: u32,
}

const GPU_TIMER_SLOTS: usize = 4;

impl GpuTimer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuTimer {
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("gpu timer"),
            ty: wgpu::QueryType::Timestamp,
            count: 2 * GPU_TIMER_SLOTS as u32,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu timer"),
            size: wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT * GPU_TIMER_SLOTS as u64,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..GPU_TIMER_SLOTS)
            .map(|_| {
                let buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu timer"),
                    size: 16,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (buf, Arc::default())
            })
            .collect();
        GpuTimer { queries, resolve, slots, current: None, period_ns: queue.get_timestamp_period() as f64, total_ms: 0.0, frames: 0 }
    }

    /// Collects finished readbacks and picks this frame's slot.
    fn begin(&mut self, device: &wgpu::Device) {
        use std::sync::atomic::Ordering;
        let _ = device.poll(wgpu::PollType::Poll);
        for (buf, state) in &self.slots {
            if state.load(Ordering::Acquire) == 2 {
                if let Ok(data) = buf.slice(..).get_mapped_range() {
                    let t: &[u64] = bytemuck::cast_slice(&data);
                    if t[1] > t[0] {
                        self.total_ms += (t[1] - t[0]) as f64 * self.period_ns / 1e6;
                        self.frames += 1;
                    }
                }
                buf.unmap();
                state.store(0, Ordering::Release);
            }
        }
        self.current = self.slots.iter().position(|(_, s)| s.load(Ordering::Acquire) == 0);
    }

    fn writes(&self, begin: bool, end: bool) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let i = self.current? as u32;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: begin.then_some(2 * i),
            end_of_pass_write_index: end.then_some(2 * i + 1),
        })
    }

    fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(i) = self.current else { return };
        let at = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT * i as u64;
        encoder.resolve_query_set(&self.queries, 2 * i as u32..2 * i as u32 + 2, &self.resolve, at);
        encoder.copy_buffer_to_buffer(&self.resolve, at, &self.slots[i].0, 0, 16);
    }

    /// After the frame is submitted: read its slot back once the GPU is done with it.
    fn submitted(&mut self) {
        use std::sync::atomic::Ordering;
        let Some(i) = self.current.take() else { return };
        let (buf, state) = &self.slots[i];
        state.store(1, Ordering::Release);
        let state = state.clone();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |r| state.store(if r.is_ok() { 2 } else { 0 }, Ordering::Release));
    }
}

/// The image a frame is drawn into before it is scaled up to the screen.
struct LowRes {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// Integer upscale: each screen pixel copies texel (pixel / 2), so every texel becomes a sharp
/// 2x2 block (no filtering).
const UPSCALE_SHADER: &str = r#"
@group(0) @binding(0) var image: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the viewport.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = min(vec2<u32>(pos.xy) / 2u, textureDimensions(image) - 1u);
    return textureLoad(image, texel, 0);
}
"#;

/// A background image as the GPU keeps it: brightness at full resolution, colour at half (like
/// the JPEG it almost always came from), 1.5 bytes a pixel instead of 4. Drawing it every frame
/// reads the whole image, so this is most of what the background costs.
pub struct BgImage {
    pub width: u32,
    pub height: u32,
    /// Y, one byte a pixel.
    pub luma: Vec<u8>,
    /// Cb, Cr pairs for each 2x2 block (`chroma_size`).
    pub chroma: Vec<u8>,
}

impl BgImage {
    /// Converts RGBA8 (JPEG's full-range BT.601, so a JPEG's colours come back as they were).
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> BgImage {
        let (w, h) = (width as usize, height as usize);
        let mut luma = Vec::with_capacity(w * h);
        for px in rgba.chunks_exact(4) {
            let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
            luma.push((0.299 * r + 0.587 * g + 0.114 * b + 0.5) as u8);
        }
        let (cw, ch) = Self::chroma_size(width, height);
        let (cw, ch) = (cw as usize, ch as usize);
        let mut chroma = Vec::with_capacity(cw * ch * 2);
        for cy in 0..ch {
            for cx in 0..cw {
                // The block's average colour (an odd last row or column averages what it has).
                let (mut r, mut g, mut b, mut n) = (0.0, 0.0, 0.0, 0.0);
                for y in (cy * 2)..(cy * 2 + 2).min(h) {
                    for x in (cx * 2)..(cx * 2 + 2).min(w) {
                        let px = &rgba[(y * w + x) * 4..];
                        (r, g, b, n) = (r + px[0] as f32, g + px[1] as f32, b + px[2] as f32, n + 1.0);
                    }
                }
                let (r, g, b) = (r / n, g / n, b / n);
                chroma.push((128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b + 0.5).clamp(0.0, 255.0) as u8);
                chroma.push((128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b + 0.5).clamp(0.0, 255.0) as u8);
            }
        }
        BgImage { width, height, luma, chroma }
    }

    pub fn chroma_size(width: u32, height: u32) -> (u32, u32) {
        (width.div_ceil(2), height.div_ceil(2))
    }

    fn is_valid(&self) -> bool {
        let (cw, ch) = Self::chroma_size(self.width, self.height);
        self.width > 0 && self.height > 0 && self.luma.len() == (self.width * self.height) as usize && self.chroma.len() == (cw * ch * 2) as usize
    }
}

/// What a pass draws into: its size in pixels, the clipped shape ranges (pixel scissors) and the
/// pixels an opaque shape will cover (x0, y0, x1, y1), where the background can be left out.
struct Target<'a> {
    size: (u32, u32),
    clips: &'a [(std::ops::Range<usize>, [u32; 4])],
    hole: Option<[u32; 4]>,
}

/// A background image on the GPU.
struct BgLayer {
    bind_group: wgpu::BindGroup,
    aspect: f32,
    uniform: wgpu::Buffer,
    /// Its opacity this frame (1: it covers everything under it).
    alpha: f32,
    /// The same image for the music player's disc, with its own uniform.
    disc_bind_group: wgpu::BindGroup,
    disc_uniform: wgpu::Buffer,
}

const BG_SHADER: &str = r#"
struct Bg { uv: vec4<f32>, params: vec4<f32> };
@group(0) @binding(0) var<uniform> bg: Bg;
@group(0) @binding(1) var luma: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var chroma: texture_2d<f32>;

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
    // YCbCr (JPEG's full-range BT.601) back to RGB.
    let y = textureSample(luma, samp, i.uv).r;
    let c = textureSample(chroma, samp, i.uv).rg - vec2<f32>(0.5);
    let rgb = vec3<f32>(y + 1.402 * c.y, y - 0.344136 * c.x - 0.714136 * c.y, y + 1.772 * c.x);
    // params: brightness, opacity (for cross-fading into a new background).
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * bg.params.x, bg.params.y);
}
"#;

/// The background once more, as the music player's turning disc: a circle of it, cut square from
/// its middle. `rect`: the disc's centre and half size in clip space; `turn`: cos and sin of its
/// angle and how much of the image's width and height the square takes.
const DISC_SHADER: &str = r#"
struct Disc { rect: vec4<f32>, turn: vec4<f32> };
@group(0) @binding(0) var<uniform> disc: Disc;
@group(0) @binding(1) var luma: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var chroma: texture_2d<f32>;

struct V { @builtin(position) pos: vec4<f32>, @location(0) p: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> V {
    let c = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u)) * 2.0 - vec2<f32>(1.0);
    var o: V;
    o.pos = vec4<f32>(disc.rect.x + c.x * disc.rect.z, disc.rect.y - c.y * disc.rect.w, 0.0, 1.0);
    o.p = c;
    return o;
}

@fragment
fn fs(i: V) -> @location(0) vec4<f32> {
    let r = length(i.p);
    let edge = fwidth(r);
    let alpha = 1.0 - smoothstep(1.0 - edge, 1.0, r);
    let q = vec2<f32>(i.p.x * disc.turn.x + i.p.y * disc.turn.y, i.p.y * disc.turn.x - i.p.x * disc.turn.y);
    let uv = vec2<f32>(0.5) + q * disc.turn.zw * 0.5;
    // The image is much bigger than the disc and has no mipmaps: averaging what one screen pixel
    // covers keeps it from shimmering as it turns.
    let dx = dpdx(uv);
    let dy = dpdy(uv);
    var y = 0.0;
    var c = vec2<f32>(0.0);
    for (var a = 0; a < 4; a++) {
        for (var b = 0; b < 4; b++) {
            let at = uv + dx * ((f32(a) + 0.5) / 4.0 - 0.5) + dy * ((f32(b) + 0.5) / 4.0 - 0.5);
            y += textureSample(luma, samp, at).r;
            c += textureSample(chroma, samp, at).rg;
        }
    }
    y /= 16.0;
    c = c / 16.0 - vec2<f32>(0.5);
    let rgb = vec3<f32>(y + 1.402 * c.y, y - 0.344136 * c.x - 0.714136 * c.y, y + 1.772 * c.x);
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), alpha);
}
"#;

/// The shapes shader, its `sprites` array as long as there are sprites and the font atlas laid
/// out like `font::atlas`.
fn shape_shader() -> String {
    SHADER
        .replace("SPRITE_COUNT", &SPRITES.to_string())
        .replace("SPRITE_LAST", &(SPRITES - 1).to_string())
        .replace("FONT_COLS", &crate::font::ATLAS_COLS.to_string())
        .replace("FONT_CELL_W", &crate::font::CELL_W.to_string())
}

const SHADER: &str = r#"
struct Globals { screen_px: vec2<f32>, px_per_unit: f32, _pad: f32 };
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<uniform> sprites: array<vec4<f32>, SPRITE_COUNT>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var font: texture_2d<f32>;

struct Inst {
    @location(0) center: vec2<f32>,
    @location(1) half: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec4<f32>,
    @location(5) angle: f32,
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
    let pad = 1.5 + max(inst.params.z, 0.0) * g.px_per_unit;
    let local = corner * (half_px + vec2<f32>(pad));
    let center_px = inst.center * g.px_per_unit;
    // Rotated shapes turn their quad; the fragment works in the shape's own (unrotated) frame.
    let cs = vec2<f32>(cos(inst.angle), sin(inst.angle));
    let p = center_px + vec2<f32>(local.x * cs.x - local.y * cs.y, local.x * cs.y + local.y * cs.x);
    var out: VOut;
    out.pos = vec4<f32>(p.x / g.screen_px.x * 2.0 - 1.0, 1.0 - p.y / g.screen_px.y * 2.0, 0.0, 1.0);
    out.local = local;
    out.half = half_px;
    out.fill = inst.fill;
    out.border = inst.border;
    out.params = vec4<f32>(inst.params.xyz * g.px_per_unit, inst.params.w);
    if (inst.fill.a <= 0.0 && inst.border.a <= 0.0) {
        // Invisible (e.g. a fade at rest): no pixels at all.
        out.pos = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    }
    return out;
}

fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let c = shade(in);
    // Pixels that wouldn't change anything (corners, ring holes, glow tails, the gaps in letters)
    // skip blending, which would read and write the screen for nothing.
    if (c.a < 0.001 && max(c.r, max(c.g, c.b)) < 0.001) {
        discard;
    }
    return c;
}

fn shade(in: VOut) -> vec4<f32> {
    if (in.params.w > 99.5) {
        // A character: how much of this pixel the font pixels under it cover (a box filter, so
        // edges look like single rects did, with no seams between neighbouring font pixels).
        // Glyphs are 7 font pixels tall and as many wide as their quad (5, or 7 for Japanese),
        // in cells of the atlas (`font::atlas`).
        let g = i32(in.params.w - 99.5);
        let fpx = in.half.y / 3.5;
        let cols = i32(round(in.half.x * 2.0 / fpx));
        let cell = vec2<i32>(g % FONT_COLS * FONT_CELL_W, g / FONT_COLS * 7);
        let p = (in.local + in.half) / fpx;
        let r = 0.5 / fpx;
        let lo = p - vec2<f32>(r);
        let hi = p + vec2<f32>(r);
        var cover = 0.0;
        for (var y = max(i32(floor(lo.y)), 0); y < min(i32(ceil(hi.y)), 7); y++) {
            let oy = min(hi.y, f32(y + 1)) - max(lo.y, f32(y));
            for (var x = max(i32(floor(lo.x)), 0); x < min(i32(ceil(hi.x)), cols); x++) {
                let ox = min(hi.x, f32(x + 1)) - max(lo.x, f32(x));
                cover += textureLoad(font, cell + vec2<i32>(x, y), 0).r * ox * oy;
            }
        }
        let a = in.fill.a * cover / (4.0 * r * r);
        return vec4<f32>(in.fill.rgb * a, a);
    }
    if (in.params.w > 9.5) {
        // A skin image (premultiplied), tinted by fill.
        let rect = sprites[min(u32(in.params.w - 9.5), SPRITE_LASTu)];
        let uv = in.local / (2.0 * in.half) + 0.5;
        let t = textureSampleLevel(atlas, samp, rect.xy + clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * rect.zw, 0.0);
        let inside = select(0.0, 1.0, all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)));
        if (in.params.z < -0.5) {
            // Light (`Canvas::light`): fill added over the image's shape, alpha 0 so it adds.
            return vec4<f32>(in.fill.rgb * t.a * in.fill.a * inside, 0.0);
        }
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
    if (glow < -0.5) {
        return vec4<f32>(color.rgb * color.a, 0.0);
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
        let time_gpu = std::env::var_os("TAIKO_PROFILE").is_some() && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("taiko"),
            required_features: if time_gpu { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() },
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
            source: wgpu::ShaderSource::Wgsl(shape_shader().into()),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let font_view = {
            let (w, h, px) = crate::font::atlas();
            let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("font"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                &px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
                size,
            );
            texture.create_view(&wgpu::TextureViewDescriptor::default())
        };
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
        let bind_group = Self::shape_bind_group(&device, &queue, &bgl, &globals, &sprite_rects, &sampler, &font_view, &Atlas::default());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32];
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
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
        let make_bg_pipeline = |format: wgpu::TextureFormat, opaque: bool| {
            let blend = if opaque { None } else { Some(wgpu::BlendState::ALPHA_BLENDING) };
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
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let bg_pipelines = [format, wgpu::TextureFormat::Rgba8Unorm].map(|f| [make_bg_pipeline(f, false), make_bg_pipeline(f, true)]);
        let disc_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("disc"),
            source: wgpu::ShaderSource::Wgsl(DISC_SHADER.into()),
        });
        let disc_pipelines = [format, wgpu::TextureFormat::Rgba8Unorm].map(|format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("disc"),
                layout: Some(&bg_pipeline_layout),
                vertex: wgpu::VertexState { module: &disc_shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &disc_shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let bg_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("background"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let capacity = 4096;
        let instances = Self::make_instance_buffer(&device, capacity);
        let upscale_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscale"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let upscale_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("upscale"),
            source: wgpu::ShaderSource::Wgsl(UPSCALE_SHADER.into()),
        });
        let upscale_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("upscale"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&upscale_layout)],
                immediate_size: 0,
            })),
            vertex: wgpu::VertexState { module: &upscale_shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &upscale_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let gpu_timer = time_gpu.then(|| GpuTimer::new(&device, &queue));
        let mut r = Renderer {
            surface: Some(surface),
            device,
            queue,
            config,
            pipeline,
            globals,
            bind_group,
            shape_layout: bgl,
            sprite_rects,
            sampler,
            font_view,
            instances,
            capacity,
            supported_modes: caps.present_modes,
            capture_pipeline,
            bg_pipelines,
            disc_pipelines,
            disc: None,
            bg_layout,
            bg_sampler,
            bg: None,
            bg_prev: None,
            bg_mix: 1.0,
            bg_brightness: 0.0,
            present,
            adapter_name,
            pixel_scale: 1,
            reconfigure: false,
            low: None,
            upscale_layout,
            upscale_pipeline,
            gpu_timer,
        };
        r.set_present(present);
        Ok(r)
    }

    /// Average GPU time per frame (ms) since the last call, when it is being measured.
    pub fn take_gpu_ms(&mut self) -> Option<f64> {
        let t = self.gpu_timer.as_mut()?;
        let avg = (t.frames > 0).then(|| t.total_ms / t.frames as f64);
        (t.total_ms, t.frames) = (0.0, 0);
        avg
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
        self.bind_group = Self::shape_bind_group(&self.device, &self.queue, &self.shape_layout, &self.globals, &self.sprite_rects, &self.sampler, &self.font_view, atlas);
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_bind_group(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        globals: &wgpu::Buffer,
        rects: &wgpu::Buffer,
        sampler: &wgpu::Sampler,
        font: &wgpu::TextureView,
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
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(font) },
            ],
        })
    }

    /// Puts this frame's disc in its uniform.
    fn write_disc(&self) {
        let (Some(([x, y, r, angle], width)), Some(l)) = (self.disc, &self.bg) else { return };
        let (sin, cos) = angle.sin_cos();
        // A square from the image's middle.
        let (u, v) = if l.aspect > 1.0 { (1.0 / l.aspect, 1.0) } else { (1.0, l.aspect) };
        let half_y = VIRTUAL_HEIGHT / 2.0;
        let uniform = [x / width * 2.0 - 1.0, 1.0 - y / half_y, r / width * 2.0, r / half_y, cos, sin, u, v];
        self.queue.write_buffer(&l.disc_uniform, 0, bytemuck::cast_slice(&uniform));
    }

    /// Replaces the background image, or removes it with `None`. The old one stays drawn
    /// underneath while the new one fades in over it (`bg_mix`).
    pub fn set_background(&mut self, image: Option<BgImage>) {
        if let Some(old) = self.bg.take() {
            self.bg_prev = Some(old);
        }
        let Some(img) = image.filter(BgImage::is_valid) else {
            return;
        };
        let (w, h) = (img.width, img.height);
        let plane = |label, w: u32, h: u32, format, bytes: u32, data: &[u8]| {
            let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                texture.as_image_copy(),
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes * w), rows_per_image: Some(h) },
                size,
            );
            texture.create_view(&wgpu::TextureViewDescriptor::default())
        };
        let (cw, ch) = BgImage::chroma_size(w, h);
        let luma = plane("background luma", w, h, wgpu::TextureFormat::R8Unorm, 1, &img.luma);
        let chroma = plane("background chroma", cw, ch, wgpu::TextureFormat::Rg8Unorm, 2, &img.chroma);
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
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&luma) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.bg_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&chroma) },
            ],
        });
        let disc_uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("disc"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let disc_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("disc"),
            layout: &self.bg_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: disc_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&luma) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.bg_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&chroma) },
            ],
        });
        self.bg = Some(BgLayer { bind_group, aspect: w as f32 / h as f32, uniform, alpha: 1.0, disc_bind_group, disc_uniform });
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
        self.configure();
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.configure();
    }

    /// Swaps in a surface for a new window (or drops it with `None` when the window goes away),
    /// keeping everything else on the GPU.
    pub fn set_surface(&mut self, surface: Option<wgpu::Surface<'static>>, width: u32, height: u32) {
        self.surface = surface;
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
        }
        self.configure();
    }

    fn configure(&self) {
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
    }

    /// Render pixels per virtual unit for crisp text (`set_text_grid`) when drawing at half
    /// resolution.
    pub fn text_grid(&self) -> Option<f32> {
        (self.pixel_scale > 1).then(|| (self.config.height / self.pixel_scale).max(1) as f32 / VIRTUAL_HEIGHT)
    }

    /// The screen in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Canvas width in virtual units.
    pub fn virtual_width(&self) -> f32 {
        VIRTUAL_HEIGHT * self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// Waits for a swapchain image. Under vsync this is where the frame may block, so callers
    /// should sample the game clock only after it returns.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        if std::mem::take(&mut self.reconfigure) {
            self.configure();
        }
        match self.surface.as_ref()?.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => Some(f),
            // Still drawable, but the window changed under it (resized, moved to another screen):
            // this one is used, the next comes in the right size.
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.reconfigure = true;
                Some(f)
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.configure();
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
        opaque: Option<[f32; 4]>,
    ) -> Option<(u32, u32, Vec<u8>)> {
        // What the shapes are drawn at: the screen, or half of it to be scaled up after.
        let scaled = self.pixel_scale > 1;
        let (w, h) = if scaled {
            ((self.config.width / self.pixel_scale).max(1), (self.config.height / self.pixel_scale).max(1))
        } else {
            (self.config.width, self.config.height)
        };
        if scaled {
            self.ensure_low(w, h);
        }
        // Clipped shape ranges (rects as fractions of the screen) -> pixel scissors, in order,
        // without overlaps.
        let mut clip: Vec<(std::ops::Range<usize>, [u32; 4])> = clips
            .into_iter()
            .filter(|(range, _)| range.start < range.end && range.end <= shapes.len())
            .map(|(range, r)| {
                let (sw, sh) = (w as f32, h as f32);
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
        let globals = Globals { screen_px: [w as f32, h as f32], px_per_unit: h as f32 / VIRTUAL_HEIGHT, _pad: 0.0 };
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
        let screen = w as f32 / h.max(1) as f32;
        for (layer, alpha) in [(&mut self.bg_prev, prev_alpha), (&mut self.bg, mix)] {
            if let Some(l) = layer {
                // "Cover" fit: crop the image's long side so it fills the screen without stretching.
                let (u, v) = if l.aspect > screen { (screen / l.aspect, 1.0) } else { (1.0, l.aspect / screen) };
                let uniform = [(1.0 - u) / 2.0, (1.0 - v) / 2.0, (1.0 + u) / 2.0, (1.0 + v) / 2.0, self.bg_brightness, alpha, 0.0, 0.0];
                self.queue.write_buffer(&l.uniform, 0, bytemuck::cast_slice(&uniform));
                l.alpha = alpha;
            }
        }
        self.write_disc();
        // The pixels wholly inside the opaque rect (its anti-aliased edge still shows what is
        // under it).
        let hole = opaque.and_then(|[x, y, rw, rh]| {
            let (sw, sh) = (w as f32, h as f32);
            let (x0, y0) = ((x * sw).ceil() + 1.0, (y * sh).ceil() + 1.0);
            let (x1, y1) = (((x + rw) * sw).floor() - 1.0, ((y + rh) * sh).floor() - 1.0);
            let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(sw), y1.min(sh));
            (x1 > x0 && y1 > y0).then_some([x0 as u32, y0 as u32, x1 as u32, y1 as u32])
        });
        if let Some(t) = &mut self.gpu_timer {
            t.begin(&self.device);
        }
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let timer = self.gpu_timer.as_ref();
        let into = Target { size: (w, h), clips: &clip, hole };
        match self.low.as_ref().filter(|_| scaled) {
            Some(low) => {
                self.encode_pass(&mut encoder, &low.view, false, shapes.len(), clear, &into, timer.and_then(|t| t.writes(true, false)));
                self.encode_upscale(&mut encoder, &view, low, clear, timer.and_then(|t| t.writes(false, true)));
            }
            None => self.encode_pass(&mut encoder, &view, false, shapes.len(), clear, &into, timer.and_then(|t| t.writes(true, true))),
        }
        if let Some(t) = timer {
            t.resolve(&mut encoder);
        }

        // Screenshots are taken at the size the frame was drawn at.
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
            self.encode_pass(&mut encoder, &target_view, true, shapes.len(), clear, &into, None);
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
        if let Some(t) = &mut self.gpu_timer {
            t.submitted();
        }
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
        if let Some(l) = &mut self.bg {
            let (u, v) = if l.aspect > screen { (screen / l.aspect, 1.0) } else { (1.0, l.aspect / screen) };
            let uniform = [(1.0 - u) / 2.0, (1.0 - v) / 2.0, (1.0 + u) / 2.0, (1.0 + v) / 2.0, self.bg_brightness, 1.0, 0.0, 0.0];
            self.queue.write_buffer(&l.uniform, 0, bytemuck::cast_slice(&uniform));
            l.alpha = 1.0;
        }
        self.write_disc();
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
        self.encode_pass(&mut encoder, &view, true, shapes.len(), clear, &Target { size: (w, h), clips: &clip, hole: None }, None);
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

    /// (Re)creates the half-resolution image when the screen size changed.
    fn ensure_low(&mut self, width: u32, height: u32) {
        if self.low.as_ref().is_some_and(|l| (l.width, l.height) == (width, height)) {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("half resolution"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // The screen's format, so the same pipelines draw into it.
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("half resolution"),
            layout: &self.upscale_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        });
        self.low = Some(LowRes { view, bind_group, width, height });
    }

    /// Scales the half-resolution image up to the screen, exactly 2x. An odd last row or column
    /// of the screen keeps the clear colour.
    fn encode_upscale(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        low: &LowRes,
        clear: [f64; 3],
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("upscale"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0], g: clear[1], b: clear[2], a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes,
            ..Default::default()
        });
        pass.set_viewport(0.0, 0.0, (low.width * 2) as f32, (low.height * 2) as f32, 0.0, 1.0);
        pass.set_pipeline(&self.upscale_pipeline);
        pass.set_bind_group(0, &low.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Draws the background and shapes into `view`, which is `size` pixels.
    #[allow(clippy::too_many_arguments)]
    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        capture: bool,
        count: usize,
        clear: [f64; 3],
        target: &Target,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites>,
    ) {
        let size = target.size;
        let layers: Vec<&BgLayer> = if self.bg_brightness > 0.0 { [&self.bg_prev, &self.bg].into_iter().flatten().collect() } else { Vec::new() };
        // A background layer always covers the whole target: under an opaque one, clearing first
        // would only cost a full-screen write.
        let load = if layers.first().is_some_and(|l| l.alpha >= 1.0) {
            // SAFETY: the opaque layer writes every pixel before anything reads them.
            wgpu::LoadOp::DontCare(unsafe { wgpu::LoadOpDontCare::enabled() })
        } else {
            wgpu::LoadOp::Clear(wgpu::Color { r: clear[0], g: clear[1], b: clear[2], a: 1.0 })
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            timestamp_writes,
            ..Default::default()
        });
        // The background around the hole (bands above and below it, and beside it), or all of it.
        let (w, h) = size;
        let bands: Vec<[u32; 4]> = match target.hole {
            Some([x0, y0, x1, y1]) => [[0, 0, w, y0], [0, y1, w, h - y1], [0, y0, x0, y1 - y0], [x1, y0, w - x1, y1 - y0]]
                .into_iter()
                .filter(|r| r[2] > 0 && r[3] > 0)
                .collect(),
            None => vec![[0, 0, w, h]],
        };
        for layer in &layers {
            let opaque = layer.alpha >= 1.0;
            pass.set_pipeline(&self.bg_pipelines[capture as usize][opaque as usize]);
            pass.set_bind_group(0, &layer.bind_group, &[]);
            for &[x, y, bw, bh] in &bands {
                pass.set_scissor_rect(x, y, bw, bh);
                pass.draw(0..4, 0..1);
            }
        }
        if !layers.is_empty() {
            pass.set_scissor_rect(0, 0, w, h);
        }
        if let (Some(_), Some(l)) = (self.disc, &self.bg) {
            pass.set_pipeline(&self.disc_pipelines[capture as usize]);
            pass.set_bind_group(0, &l.disc_bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
        pass.set_pipeline(if capture { &self.capture_pipeline } else { &self.pipeline });
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        // Unclipped runs between the clipped ranges.
        let mut at = 0u32;
        for (range, [x, y, w, h]) in target.clips {
            if (range.start as u32) > at {
                pass.draw(0..4, at..range.start as u32);
            }
            pass.set_scissor_rect(*x, *y, *w, *h);
            pass.draw(0..4, range.start as u32..range.end as u32);
            pass.set_scissor_rect(0, 0, size.0, size.1);
            at = range.end as u32;
        }
        if (count as u32) > at {
            pass.draw(0..4, at..count as u32);
        }
    }
}

thread_local! {
    /// Render pixels per virtual unit when text snaps to whole pixels, else 0 (`set_text_grid`).
    static TEXT_GRID: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// Half resolution: text is laid on a grid of `px_per_unit` render pixels per virtual unit, each
/// font pixel a whole number of them (rounded up, so small text gets bigger) and each glyph on a
/// pixel boundary, so it stays crisp after the 2x scale. `None`: text at its exact size. Per
/// thread: the render thread sets it before building each frame.
pub fn set_text_grid(px_per_unit: Option<f32>) {
    TEXT_GRID.with(|g| g.set(px_per_unit.filter(|p| *p > 0.0).unwrap_or(0.0)));
}

/// One font pixel (in virtual units) of text `size` units tall.
fn font_px(size: f32) -> f32 {
    let grid = TEXT_GRID.with(std::cell::Cell::get);
    if grid > 0.0 {
        // A hair of slack so 2.01 pixels stays 2.
        (size / 7.0 * grid - 0.05).ceil().max(1.0) / grid
    } else {
        size / 7.0
    }
}

/// `v` moved onto the text grid (unchanged without one).
pub fn snap_to_grid(v: f32) -> f32 {
    let grid = TEXT_GRID.with(std::cell::Cell::get);
    if grid > 0.0 { (v * grid).round() / grid } else { v }
}

/// `want` (virtual units) as a whole number of screen pixels, so pixel art stays square and
/// even at any window size (unchanged without a grid).
pub fn pixel_size(want: f32) -> f32 {
    let grid = TEXT_GRID.with(std::cell::Cell::get);
    if grid > 0.0 { (want * grid).round().max(1.0) / grid } else { want }
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

thread_local! {
    /// The time the frame being built is for (`set_frame_time`).
    static FRAME_TIME: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Animations in the frame being built use this time instead of each reading the clock (which
/// can cost microseconds a read). Per thread: the render thread sets it before each frame.
pub fn set_frame_time(now: Option<std::time::Instant>) {
    FRAME_TIME.with(|t| t.set(now));
}

/// Eased-out progress (0 to 1) of an animation started at `since` lasting `ms`; 1 with
/// animations off.
pub fn progress(since: std::time::Instant, ms: f32) -> f32 {
    if !animations() {
        return 1.0;
    }
    let now = FRAME_TIME.with(std::cell::Cell::get).unwrap_or_else(std::time::Instant::now);
    let t = (now.saturating_duration_since(since).as_secs_f32() * 1000.0 / ms).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Something clickable drawn this frame, for mouse navigation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// A button that does what this key does.
    Key(winit::keyboard::KeyCode),
    /// Top bar tab: 0 songs, 1 settings, 2 multiplayer.
    Tab(usize),
    /// A note of the main menu (index into `menu::ITEMS`).
    MenuItem(usize),
    /// A music button of the main menu or the player, and the player's timeline.
    Music(crate::player::Control),
    MusicBar,
    /// The player: its search box, a song (index into the list's tracks), its `+`, a row of the
    /// queue, the volume, a speed mod (`player::Speed::ALL`) and the pitch (resets it).
    PlayerSearch,
    PlayerRow(usize),
    PlayerQueue(usize),
    PlayerUnqueue(usize),
    PlayerVolume,
    PlayerSpeed(usize),
    PlayerPitch,
    /// A row of the song list (index into its visible list): a difficulty, or a song's own row
    /// (pointing at its first difficulty).
    SongRow(usize),
    SongHeader(usize),
    /// A heading of the song list (index into its sections): opens or closes it.
    SectionHeader(usize),
    /// An item of the song list's right-click menu.
    SongMenu(usize),
    /// A song's "update available" (library index of one of its difficulties).
    UpdateSet(usize),
    /// A mod in the mod select (index into `mods::ALL`) / the song select's Mods button.
    ModToggle(usize),
    ModsButton,
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
    SortList,
    CollectionPick,
    PickRow(usize),
    /// A row of a setting's list of choices.
    ChoiceRow(usize),
    /// Multiverse setup: a universe's (row) colours, shape, speed or a mod (column).
    UniverseCell(usize, usize),
    /// Shapeshift setup: a choice (row, column) / move the selected shape earlier or later.
    ShapeshiftCell(usize, usize),
    ShapeshiftMove(i32),
    /// Mod customization: a tab (index into the customizable mods that are on).
    CustomTab(usize),
    /// Multiverse setup: load a preset / delete one of the player's.
    UniversePreset(usize),
    UniversePresetDelete(usize),
    /// Song select with no maps: choose the Songs folder.
    SongsFolder,
    /// The song list's scroll bar.
    SongScrollbar,
    /// A settings sidebar section.
    SettingsSection(usize),
    /// The update badge: install the new build.
    Update,
    /// Download tab status category / sort chips.
    DlCategory(usize),
    DlSort(usize),
    /// A row of the song select leaderboard (opens its results) / its video button, and a row
    /// of the online leaderboard.
    ScoreRow(usize),
    ScoreVideo(usize),
    OnlineScoreRow(usize),
    /// The What's new chip, and an overlay over everything (takes clicks meant for underneath).
    Changelog,
    /// The account badge: the website profile (or logging in).
    Account,
    /// Song select's dividers: between the song list and the panel, and between the panel and
    /// the leaderboard (dragged).
    SplitList,
    SplitPreview,
    Overlay,
    /// The leaderboard's tabs: local (false) or online (true).
    BoardTab(bool),
    /// Lobby: give the host role to this player.
    GiveHost(u32),
    /// The friends overlay: its chip, its panel (swallows clicks), a tab, a row, a row's button, and a
    /// notification card.
    SocialToggle,
    SocialPanel,
    SocialTab(crate::social::Tab),
    SocialRow(usize),
    SocialAction(usize, crate::social::Action),
    SocialNote(usize),
    /// A chat line naming a map set (`/np`): downloads it, or picks it in song select.
    ChatMap(u32),
    /// A text field (a search box, the message box): on Android, tapping it brings up the
    /// on-screen keyboard.
    TextField,
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
    /// The music player's disc: centre, radius, angle (drawn under every shape, see `DISC_SHADER`).
    pub disc: Option<[f32; 4]>,
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
    pub sprites: PerSprite<Option<[f32; 2]>>,
    pub skin_layout: SkinLayout,
    /// Layout checks (`debug::layout_problems`): record every text and rounded box drawn.
    pub audit: bool,
    /// Texts drawn while auditing: their glyph shapes and the string.
    pub texts: Vec<(std::ops::Range<usize>, String)>,
    /// Rounded boxes drawn while auditing (shape indices): the frames text should stay inside.
    pub boxes: Vec<usize>,
    /// A plain opaque rect (shape index) that hides the background under it, so the renderer
    /// needn't draw the background there (`opaque_rect`).
    pub opaque: Option<usize>,
}

pub fn rgba(hex: u32, a: f32) -> [f32; 4] {
    let acc = accent();
    if acc != DEFAULT_ACCENT {
        if let Some([r, g, b]) = tint_grey(hex, acc) {
            return [r, g, b, a];
        }
    }
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0, a]
}

/// The interface's red, and the accents it can have instead (Settings -> Skin -> UI colour).
pub const DEFAULT_ACCENT: u32 = 0xEB452C;
pub const ACCENTS: [(&str, u32); 8] = [
    ("red", DEFAULT_ACCENT),
    ("pink", 0xFF5FA2),
    ("purple", 0xC77DFF),
    ("blue", 0x4FB4FF),
    ("teal", 0x5EE6C8),
    ("green", 0x86E05A),
    ("yellow", 0xFFD34D),
    ("orange", 0xFF8A3D),
];

static ACCENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(DEFAULT_ACCENT);

pub fn set_accent(hex: u32) {
    ACCENT.store(hex, std::sync::atomic::Ordering::Relaxed);
}

/// The interface's accent: borders of what's selected, headings, sliders.
pub fn accent() -> u32 {
    ACCENT.load(std::sync::atomic::Ordering::Relaxed)
}

/// `default` with the default accent; with another, that accent mixed `t` of the way to `toward`
/// (for darker or lighter shades of it).
pub fn themed(default: u32, toward: u32, t: f32) -> u32 {
    let acc = accent();
    if acc == DEFAULT_ACCENT {
        return default;
    }
    let ch = |hex: u32, s: u32| ((hex >> s) & 0xff) as f32;
    [16, 8, 0].into_iter().fold(0, |out, s| out | (((ch(acc, s) + (ch(toward, s) - ch(acc, s)) * t).round() as u32) << s))
}

/// The interface's greys all have the same slight blue cast (equal red and green, a little more
/// blue). With an accent other than the default, the cast takes the accent's hue instead, a bit
/// stronger, at the same brightness: the whole interface leans that colour.
fn tint_grey(hex: u32, acc: u32) -> Option<[f32; 3]> {
    let (r, g, b) = ((hex >> 16) & 0xff, (hex >> 8) & 0xff, hex & 0xff);
    if r != g || b <= r || b - r > 0x20 {
        return None;
    }
    let a = [(acc >> 16) & 0xff, (acc >> 8) & 0xff, acc & 0xff].map(|c| c as f32 / 255.0);
    let (lo, hi) = (a[0].min(a[1]).min(a[2]), a[0].max(a[1]).max(a[2]));
    // Same brightness as the original, which is grey plus a third of its cast.
    let cast = (b - r) as f32 / 255.0;
    let mean = r as f32 / 255.0 + cast / 3.0;
    if hi - lo < 0.01 {
        return Some([mean; 3]);
    }
    let dir = a.map(|c| (c - lo) / (hi - lo));
    let dir_mean = (dir[0] + dir[1] + dir[2]) / 3.0;
    let k = cast * 1.8;
    Some(dir.map(|d| (mean + k * (d - dir_mean)).clamp(0.0, 1.0)))
}

/// `hex` turned `degrees` round the colour wheel (same saturation and brightness).
pub fn rotate_hue(hex: u32, degrees: f32) -> u32 {
    let [r, g, b] = [(hex >> 16) & 0xff, (hex >> 8) & 0xff, hex & 0xff].map(|c| c as f32 / 255.0);
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    let d = hi - lo;
    let h = if d == 0.0 {
        0.0
    } else if hi == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if hi == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let h = (h + degrees).rem_euclid(360.0);
    let x = d * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (d, x, 0.0),
        1 => (x, d, 0.0),
        2 => (0.0, d, x),
        3 => (0.0, x, d),
        4 => (x, 0.0, d),
        _ => (d, 0.0, x),
    };
    [r, g, b].into_iter().fold(0, |out, c| (out << 8) | ((c + lo) * 255.0).round() as u32)
}

/// What the window is cleared to under everything (a grey, so it follows the accent too).
pub fn clear_colour() -> [f64; 3] {
    let [r, g, b, _] = rgba(0x121219, 1.0);
    [r as f64, g as f64, b as f64]
}

/// An accent as the config and settings write it: a preset's name, else "#rrggbb".
pub fn accent_name(hex: u32) -> String {
    ACCENTS.iter().find(|a| a.1 == hex).map_or_else(|| format!("#{hex:06x}"), |a| a.0.to_string())
}

/// `accent_name`'s inverse.
pub fn parse_accent(s: &str) -> Option<u32> {
    let s = s.trim();
    ACCENTS.iter().find(|a| a.0.eq_ignore_ascii_case(s)).map(|a| a.1).or_else(|| {
        let h = s.strip_prefix('#')?;
        (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten()
    })
}

const CLEAR: [f32; 4] = [0.0; 4];

impl Canvas {
    pub fn clear(&mut self, width: f32) {
        self.shapes.clear();
        self.disc = None;
        self.hits.clear();
        self.clips.clear();
        self.texts.clear();
        self.boxes.clear();
        self.opaque = None;
        self.width = width;
    }

    /// Marks the shape just drawn (a `rect` with alpha 1) as hiding the background under it.
    pub fn mark_opaque(&mut self) {
        self.opaque = self.shapes.len().checked_sub(1);
    }

    /// Where the `opaque` rect ended up (virtual units, after transforms and clips), if it is
    /// still a plain, unrotated opaque rect.
    pub fn opaque_rect(&self, frame_clip: Option<(&std::ops::Range<usize>, [f32; 4])>) -> Option<[f32; 4]> {
        let i = self.opaque?;
        let sh = self.shapes.get(i)?;
        if sh.fill[3] < 1.0 || sh.params != [0.0; 4] || sh.border != sh.fill || sh.angle != 0.0 {
            return None;
        }
        let (mut x0, mut y0) = (sh.center[0] - sh.half[0], sh.center[1] - sh.half[1]);
        let (mut x1, mut y1) = (sh.center[0] + sh.half[0], sh.center[1] + sh.half[1]);
        for (range, [cx, cy, cw, ch]) in self.clips.iter().map(|(r, c)| (r, *c)).chain(frame_clip) {
            if range.contains(&i) {
                (x0, y0, x1, y1) = (x0.max(cx), y0.max(cy), x1.min(cx + cw), y1.min(cy + ch));
            }
        }
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
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
            (true, false) => (themed(0x2b1a17, 0x15151d, 0.84), crate::render::accent()),
            (true, true) => (themed(0x4a221b, 0x15151d, 0.7), themed(0xFF6A50, 0xffffff, 0.2)),
            (false, false) => (0x23232f, 0x3a3a4e),
            (false, true) => (0x34344a, 0x6a6a88),
        };
        self.round_rect(x, y, w, h, 6.0, rgba(bg, 1.0), rgba(border, 1.0), 1.0);
        // A label too long for the button shrinks a little, then gets cut short.
        let room = w - 8.0;
        let size = Self::fit_size(size, size * 0.8, room, label);
        self.text_centered(x + w / 2.0, y + (h - size) / 2.0, size, rgba(0xffffff, 1.0), &Self::fit(size, room, label));
        self.hit(x, y, w, h, hit);
    }

    /// A centred row of buttons: (label, hit, accent).
    pub fn button_row(&mut self, y: f32, buttons: &[(&str, Hit, bool)]) {
        const SIZE: f32 = 8.0;
        const GAP: f32 = 8.0;
        let gaps = GAP * (buttons.len().saturating_sub(1)) as f32;
        let room = self.width - 32.0 - gaps;
        // Too wide for the screen: less padding, then every button narrower alike (labels
        // shrink and get cut in `button`).
        let text: Vec<f32> = buttons.iter().map(|(l, _, _)| Canvas::text_width(SIZE, l)).collect();
        let padded = |pad: f32| text.iter().map(|t| t + pad).collect::<Vec<f32>>();
        let mut widths = padded(28.0);
        if widths.iter().sum::<f32>() > room {
            widths = padded(14.0);
            let sum = widths.iter().sum::<f32>();
            if sum > room {
                widths.iter_mut().for_each(|w| *w *= room / sum);
            }
        }
        let total = widths.iter().sum::<f32>() + gaps;
        let mut x = (self.width - total) / 2.0;
        for ((label, hit, accent), w) in buttons.iter().zip(widths) {
            self.button(x, y, w, 20.0, label, SIZE, *hit, *accent);
            x += w + GAP;
        }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: [f32; 4]) {
        self.shapes.push(Shape { center: [x + w / 2.0, y + h / 2.0], half: [w / 2.0, h / 2.0], fill, border: fill, params: [0.0; 4], angle: 0.0 });
    }

    pub fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, fill: [f32; 4], border: [f32; 4], border_w: f32) {
        if self.audit {
            self.boxes.push(self.shapes.len());
        }
        self.shapes.push(Shape {
            center: [x + w / 2.0, y + h / 2.0],
            half: [w / 2.0, h / 2.0],
            fill,
            border,
            params: [radius, border_w, 0.0, 0.0],
            angle: 0.0,
        });
    }

    pub fn circle(&mut self, x: f32, y: f32, r: f32, fill: [f32; 4], border: [f32; 4], border_w: f32) {
        self.shapes.push(Shape { center: [x, y], half: [r, r], fill, border, params: [r, border_w, 0.0, 0.0], angle: 0.0 });
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
            angle: 0.0,
        });
    }

    /// Left (`side` 1) or right (`side` 2) half of a circle.
    pub fn half_circle(&mut self, x: f32, y: f32, r: f32, side: u8, fill: [f32; 4], border: [f32; 4], border_w: f32) {
        self.shapes.push(Shape { center: [x, y], half: [r, r], fill, border, params: [r, border_w, 0.0, side as f32], angle: 0.0 });
    }

    /// A skin image centred on (`x`, `y`) for something of radius `r`, tinted by `tint`. False
    /// (nothing drawn) when the skin doesn't have it.
    pub fn sprite(&mut self, sprite: Sprite, x: f32, y: f32, r: f32, tint: [f32; 4]) -> bool {
        let Some([sx, sy]) = self.sprites[sprite as usize] else { return false };
        self.sprite_box(sprite, x, y, r * sx, r * sy, tint);
        true
    }

    /// The skin image's size relative to the radius of what it's drawn for (`sprite`), if the
    /// skin has it.
    pub fn sprite_size(&self, sprite: Sprite) -> Option<[f32; 2]> {
        self.sprites[sprite as usize]
    }

    /// `s` in the skin's `font`: each glyph's sprite, the overlap, and the scale (units per
    /// sprite size unit) that makes a `0` `h` tall. None when the font lacks a glyph `s` needs.
    fn glyphs(&self, font: Font, h: f32, s: &str) -> Option<(Vec<Glyph>, f32, f32)> {
        let (glyph, zero, overlap): (fn(char) -> Option<Sprite>, Sprite, f32) = match font {
            Font::Score => (Sprite::score_glyph, Sprite::Score0, self.skin_layout.score_overlap),
            Font::Combo => (Sprite::combo_glyph, Sprite::Combo0, self.skin_layout.combo_overlap),
        };
        let k = h / self.sprites[zero as usize]?[1];
        let glyphs = s.chars().map(|ch| glyph(ch).and_then(|g| Some((g, self.sprites[g as usize]?)))).collect::<Option<Vec<_>>>()?;
        Some((glyphs, overlap / 128.0 * k, k))
    }

    /// How wide `s` is in the skin's `font` with a `0` `h` tall. None when the skin has no such
    /// font (or not every glyph `s` needs).
    pub fn number_width(&self, font: Font, h: f32, s: &str) -> Option<f32> {
        let (glyphs, overlap, k) = self.glyphs(font, h, s)?;
        Some(glyphs.iter().map(|(_, [w, _])| w * k).sum::<f32>() - overlap * glyphs.len().saturating_sub(1) as f32)
    }

    /// `s` in the skin's `font`, its left edge at `x` and a `0`'s top at `y`, `h` tall; glyphs
    /// sit on the same baseline. False (nothing drawn) when the skin can't draw it.
    pub fn number(&mut self, font: Font, x: f32, y: f32, h: f32, tint: [f32; 4], s: &str) -> bool {
        let Some((glyphs, overlap, k)) = self.glyphs(font, h, s) else { return false };
        let mut x = x;
        for (g, [w, gh]) in glyphs {
            let (w, gh) = (w * k, gh * k);
            self.sprite_box(g, x + w / 2.0, y + h - gh / 2.0, w / 2.0, gh / 2.0, tint);
            x += w - overlap;
        }
        true
    }

    /// Adds `color` (its alpha the strength) over skin image `sprite`'s shape centred on (`x`,
    /// `y`) for something of radius `r`, or a disc without it. Light, like osu!'s kiai flash: a
    /// stack of them brightens what's under it instead of covering it.
    pub fn light(&mut self, sprite: Sprite, x: f32, y: f32, r: f32, color: [f32; 4]) {
        let start = self.shapes.len();
        if !self.sprite(sprite, x, y, r, color) {
            self.circle(x, y, r, color, CLEAR, 0.0);
        }
        // Negative where a glow's softness goes; big, so scaling the playfield keeps it below -0.5.
        self.shapes[start].params[2] = -1.0e6;
    }

    /// A skin image stretched over the box centred on (`x`, `y`), `hw` by `hh` either side.
    pub fn sprite_box(&mut self, sprite: Sprite, x: f32, y: f32, hw: f32, hh: f32, tint: [f32; 4]) {
        self.shapes.push(Shape { center: [x, y], half: [hw, hh], fill: tint, border: tint, params: [0.0, 0.0, 0.0, SPRITE_BASE + sprite as usize as f32], angle: 0.0 });
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

    /// Turns every shape drawn since `start` by `angle` (radians, clockwise on screen) and scales
    /// it by `scale` about `pivot`, which lands on `dest`. Hit areas aren't moved.
    pub fn rotate_since(&mut self, start: usize, pivot: [f32; 2], angle: f32, scale: f32, dest: [f32; 2]) {
        let (sin, cos) = angle.sin_cos();
        for sh in &mut self.shapes[start..] {
            let (x, y) = ((sh.center[0] - pivot[0]) * scale, (sh.center[1] - pivot[1]) * scale);
            sh.center = [dest[0] + x * cos - y * sin, dest[1] + x * sin + y * cos];
            sh.half = [sh.half[0] * scale, sh.half[1] * scale];
            sh.params[0] *= scale;
            sh.params[1] *= scale;
            sh.params[2] *= scale;
            sh.angle += angle;
        }
    }

    /// Squeezes every shape drawn since `start` horizontally towards x = `cx` by `fx`; a negative
    /// `fx` flips them over (half circles swap sides).
    pub fn squash_x_since(&mut self, start: usize, cx: f32, fx: f32) {
        for sh in &mut self.shapes[start..] {
            sh.center[0] = cx + (sh.center[0] - cx) * fx;
            sh.half[0] *= fx.abs();
            if fx < 0.0 {
                flip_x(sh);
            }
        }
    }

    /// Stretches every shape drawn since `start` by (`sx`, `sy`) about `at`: circles become
    /// stadiums (used for squishy notes).
    pub fn stretch_since(&mut self, start: usize, at: [f32; 2], sx: f32, sy: f32) {
        for sh in &mut self.shapes[start..] {
            sh.center = [at[0] + (sh.center[0] - at[0]) * sx, at[1] + (sh.center[1] - at[1]) * sy];
            sh.half = [sh.half[0] * sx, sh.half[1] * sy];
        }
    }

    /// Flips every shape drawn since `start` left to right across a canvas `w` wide (Mirror
    /// mod; text drawn this way would read backwards, so it is drawn afterwards).
    pub fn mirror_since(&mut self, start: usize, w: f32) {
        for sh in &mut self.shapes[start..] {
            sh.center[0] = w - sh.center[0];
            flip_x(sh);
        }
    }

    /// Fades every shape drawn since `start` to `alpha` of its opacity and darkens it to `dim`
    /// of its brightness (the Multiverse playfields out of focus).
    pub fn fade_since(&mut self, start: usize, alpha: f32, dim: f32) {
        for sh in &mut self.shapes[start..] {
            for col in [&mut sh.fill, &mut sh.border] {
                col[0] *= dim;
                col[1] *= dim;
                col[2] *= dim;
                col[3] *= alpha;
            }
        }
    }

    pub fn glow(&mut self, x: f32, y: f32, r: f32, softness: f32, color: [f32; 4]) {
        self.shapes.push(Shape { center: [x, y], half: [r, r], fill: color, border: color, params: [r, 0.0, softness, 0.0], angle: 0.0 });
    }

    /// Draws text with the built-in 5x7 font; `size` is the glyph height. Returns the advance width.
    /// On a text grid (`set_text_grid`) the glyphs may come out bigger: they stay centred on the
    /// line `size` would have taken.
    pub fn text(&mut self, x: f32, y: f32, size: f32, color: [f32; 4], s: &str) -> f32 {
        let px = font_px(size);
        let x = snap_to_grid(x);
        let y = snap_to_grid(y + (size - px * 7.0) / 2.0);
        let mut cx = x;
        let first = self.shapes.len();
        if !self.audit {
            // One quad per character, as wide as its glyph; the shader fills in its font pixels.
            for ch in s.chars() {
                let i = font::index(ch);
                let cols = font::columns(i) as f32;
                // Blank (space): nothing to draw.
                if i != 0 {
                    let half = [px * cols / 2.0, px * 3.5];
                    self.shapes.push(Shape {
                        center: [cx + half[0], y + half[1]],
                        half,
                        fill: color,
                        border: color,
                        params: [0.0, 0.0, 0.0, GLYPH_BASE + i as f32],
                        angle: 0.0,
                    });
                }
                cx += px * (cols + 1.0);
            }
            return cx - x;
        }
        // Layout checks (and their CPU-drawn pictures) see each font pixel run as its own rect.
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
                            angle: 0.0,
                        });
                    } else {
                        row += 1;
                    }
                }
            }
            cx += px * (cols.len() + 1) as f32;
        }
        if self.audit {
            self.texts.push((first..self.shapes.len(), s.to_string()));
        }
        cx - x
    }

    pub fn text_width(size: f32, s: &str) -> f32 {
        s.chars().map(font::advance).sum::<usize>() as f32 * font_px(size)
    }

    /// The advance of one Latin character of text `size` tall (Japanese takes 4/3 of it).
    pub fn char_width(size: f32) -> f32 {
        font_px(size) * 6.0
    }

    /// The longest start of `s` (in bytes) that is at most `max_w` wide at `size`.
    pub fn fitting_prefix(size: f32, max_w: f32, s: &str) -> usize {
        let room = (max_w / font_px(size) + 1e-3).floor() as usize;
        let mut w = 0;
        for (at, ch) in s.char_indices() {
            w += font::advance(ch);
            if w > room {
                return at;
            }
        }
        s.len()
    }

    /// How tall text `size` comes out (`size`, or more on a text grid).
    pub fn text_height(size: f32) -> f32 {
        font_px(size) * 7.0
    }

    /// The size to draw text at so it comes out at most `max_h` tall: `size`, or on a text grid
    /// (where it may grow) the biggest whole-pixel size that fits. For secondary lines that
    /// would crowd the line above when everything grows.
    pub fn fit_height(size: f32, max_h: f32) -> f32 {
        if Self::text_height(size) <= max_h {
            return size;
        }
        let grid = TEXT_GRID.with(std::cell::Cell::get);
        if grid > 0.0 { (max_h / 7.0 * grid).floor().max(1.0) / grid * 7.0 } else { max_h }
    }

    pub fn text_centered(&mut self, cx: f32, y: f32, size: f32, color: [f32; 4], s: &str) {
        let w = Self::text_width(size, s);
        self.text(cx - w / 2.0, y, size, color, s);
    }

    pub fn text_right(&mut self, right: f32, y: f32, size: f32, color: [f32; 4], s: &str) {
        let w = Self::text_width(size, s);
        self.text(right - w, y, size, color, s);
    }

    /// `s` as it fits in `max_w` at `size`: whole, or cut short with "...".
    pub fn fit(size: f32, max_w: f32, s: &str) -> String {
        if Self::text_width(size, s) <= max_w {
            return s.to_string();
        }
        let dots = Self::text_width(size, "...");
        if max_w < dots + Self::char_width(size) {
            return String::new();
        }
        format!("{}...", s[..Self::fitting_prefix(size, max_w - dots, s)].trim_end())
    }

    /// Like `fit`, but keeps the end (what is being typed) and cuts the start.
    pub fn fit_end(size: f32, max_w: f32, s: &str) -> String {
        if Self::text_width(size, s) <= max_w {
            return s.to_string();
        }
        let dots = Self::text_width(size, "...");
        if max_w < dots + Self::char_width(size) {
            return String::new();
        }
        // The longest end that fits after the dots.
        let room = max_w - dots;
        let start = s.char_indices().map(|(at, _)| at).find(|&at| Self::text_width(size, &s[at..]) <= room).unwrap_or(s.len());
        format!("...{}", &s[start..])
    }

    /// A line of key hints ("Enter play  Esc back"; items apart by two or more spaces) as it
    /// fits in `max_w`: the first items that fit, whole.
    pub fn fit_hints(size: f32, max_w: f32, s: &str) -> String {
        if Self::text_width(size, s) <= max_w {
            return s.to_string();
        }
        let gap = s.find("  ").map(|i| s[i..].len() - s[i..].trim_start().len()).unwrap_or(2);
        let mut out = String::new();
        for item in s.split("  ").map(str::trim).filter(|i| !i.is_empty()) {
            let next = if out.is_empty() { item.to_string() } else { format!("{out}{}{item}", " ".repeat(gap)) };
            if Self::text_width(size, &next) > max_w {
                break;
            }
            out = next;
        }
        out
    }

    /// The size to draw `s` at so it is at most `max_w` wide: `size`, or smaller down to `min`.
    pub fn fit_size(size: f32, min: f32, max_w: f32, s: &str) -> f32 {
        let w = Self::text_width(size, s);
        if w <= max_w {
            return size;
        }
        let grid = TEXT_GRID.with(std::cell::Cell::get);
        if grid > 0.0 {
            // Font pixels come in whole render pixels: the biggest that fits.
            let per_px = max_w / s.chars().map(font::advance).sum::<usize>().max(1) as f32;
            return ((per_px * grid).floor().max(1.0) / grid * 7.0).max(min);
        }
        (size * max_w / w).max(min)
    }
}

#[cfg(test)]
mod tests {
    use wgpu::naga;

    #[test]
    fn accents_tint_the_greys_and_round_trip() {
        use super::*;
        // Pink: the blue cast turns pink, brightness stays.
        let [r, g, b] = tint_grey(0x6f6f86, 0xFF5FA2).unwrap();
        assert!(r > b && b > g, "{r} {g} {b}");
        assert!(((r + g + b) / 3.0 - (0x6f as f32 + 0x17 as f32 / 3.0) / 255.0).abs() < 0.01);
        // White, black, notes and other colours aren't the greys.
        for hex in [0xffffff, 0x000000, 0x438EAD, 0xFFD34D, 0x2b2433] {
            assert!(tint_grey(hex, 0xFF5FA2).is_none(), "{hex:06x}");
        }
        for (_, hex) in ACCENTS {
            assert_eq!(parse_accent(&accent_name(hex)), Some(hex));
            assert_eq!(rotate_hue(hex, 360.0), hex);
        }
        assert_eq!(parse_accent("#12ab9F"), Some(0x12ab9f));
        assert_eq!(accent_name(0x12ab9f), "#12ab9f");
        assert_eq!(parse_accent("#12ab9"), None);
        assert_eq!(rotate_hue(0xff0000, 120.0), 0x00ff00);
    }

    #[test]
    fn shaders_compile() {
        let shapes = super::shape_shader();
        for (name, src) in [("shapes", shapes.as_str()), ("background", super::BG_SHADER), ("disc", super::DISC_SHADER), ("upscale", super::UPSCALE_SHADER)] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }
}
