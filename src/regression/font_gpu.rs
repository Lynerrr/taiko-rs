//! Hardware check of the production text shader, texture layout and uploads. Run with
//! `cargo test --locked --features gpu-tests font_gpu` on a machine with a GPU.
//! The regular headless regression suite also checks that canvas transforms preserve UV sizes.
use super::*;
use wgpu::util::DeviceExt;

struct ResetFont;
impl Drop for ResetFont {
    fn drop(&mut self) {
        crate::system_font::select("");
        crate::system_font::set_pixel_scale(1.0);
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    group: wgpu::BindGroup,
    font: wgpu::Texture,
    cards: wgpu::Texture,
    revision: u64,
}

impl Gpu {
    fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("gpu-tests requires a GPU or software graphics adapter");
        eprintln!("font GPU regression: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production shapes"),
            source: wgpu::ShaderSource::Wgsl(shape_shader().into()),
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("font regression"),
            layout: None,
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
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let rects = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (SPRITES * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let (view, font) = make_font_texture(&device, &queue);
        let cards = card_texture(&device);
        let group = Renderer::shape_bind_group(
            &device,
            &queue,
            &pipeline.get_bind_group_layout(0),
            &globals,
            &rects,
            &sampler,
            &view,
            &view,
            &cards.create_view(&Default::default()),
            &Atlas::default(),
        );
        Self {
            device,
            queue,
            pipeline,
            globals,
            group,
            font,
            cards,
            revision: 0,
        }
    }

    fn draw(&mut self, shapes: &[Shape], scale: f32) -> (u32, u32, Vec<u8>) {
        upload_font_glyphs(&self.queue, &self.font, &mut self.revision);
        let (w, h) = ((256.0 * scale) as u32, (128.0 * scale) as u32);
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                screen_px: [w as f32, h as f32],
                px_per_unit: scale,
                _pad: 0.0,
                origin: [0.0; 2],
                _pad2: [0.0; 2],
            }),
        );
        let instances = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(shapes),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let bpr = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(bpr * h),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            pass.draw(0..4, 0..shapes.len() as u32);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: None,
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let pixels = data
            .chunks_exact(bpr as usize)
            .flat_map(|row| row[..w as usize * 4].iter().copied())
            .collect();
        (w, h, pixels)
    }
}

/// Linear filtering of the independent CPU raster, with transparent padding around the glyph.
fn sample(glyph: &crate::system_font::Glyph, x: f32, y: f32) -> f32 {
    let (x, y) = (x - 0.5, y - 0.5);
    let (left, top) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let mut alpha = 0.0;
    for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
        for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
            let (cx, cy) = (left + dx, top + dy);
            if cx >= 0
                && cy >= 0
                && cx < glyph.metrics.width as i32
                && cy < glyph.metrics.height as i32
            {
                alpha +=
                    glyph.pixels[cy as usize * glyph.metrics.width + cx as usize] as f32 * wx * wy;
            }
        }
    }
    alpha
}

#[test]
fn font_gpu_samples_the_same_glyph_at_every_screen_and_canvas_scale() {
    use crate::system_font;
    let _reset = ResetFont;
    assert!(system_font::select("Abel"));
    let mut gpu = Gpu::new();
    // Switch away and back to exercise reusing GPU atlas slots with fresh uploads too.
    for _ in 0..2 {
        system_font::select("");
        system_font::select("Abel");
        let mut canvas = Canvas::<()>::default();
        canvas.text(20.0, 20.0, system_font::cap(), rgba(0xffffff, 1.0), "Wi12");
        let original = canvas.shapes.clone();
        for canvas_scale in [0.5, 1.0, 1.5] {
            canvas.shapes = original.clone();
            canvas.transform_since(0, [0.0; 2], canvas_scale, 0.0, 0.0);
            for screen_scale in [0.5, 1.0, 1.5, 2.0, 3.0] {
                let (w, h, pixels) = gpu.draw(&canvas.shapes, screen_scale);
                let mut checked = 0;
                let mut wrong = 0;
                for shape in &canvas.shapes {
                    let glyph =
                        system_font::glyph_in_slot((shape.params[3] - system_font::BASE) as usize);
                    let origin = [
                        shape.center[0] - shape.half[0],
                        shape.center[1] - shape.half[1],
                    ];
                    for y in 0..h {
                        for x in 0..w {
                            let gx = ((x as f32 + 0.5) / screen_scale - origin[0]) / canvas_scale;
                            let gy = ((y as f32 + 0.5) / screen_scale - origin[1]) / canvas_scale;
                            if gx < 0.0
                                || gy < 0.0
                                || gx >= glyph.metrics.width as f32
                                || gy >= glyph.metrics.height as f32
                            {
                                continue;
                            }
                            let actual = pixels[((y * w + x) * 4 + 3) as usize] as f32;
                            let expected = sample(&glyph, gx, gy);
                            checked += 1;
                            if (actual - expected).abs() > 3.0 {
                                wrong += 1;
                            }
                        }
                    }
                }
                assert!(checked > 20);
                assert!(
                    wrong * 100 < checked,
                    "canvas scale {canvas_scale}, screen scale {screen_scale}: {wrong}/{checked} pixels differ from the CPU glyph"
                );
                if let Some(dir) = std::env::var_os("TAIKO_GPU_SHOTS") {
                    let path = std::path::Path::new(&dir)
                        .join(format!("font-gpu-{canvas_scale}-{screen_scale}.png"));
                    image::save_buffer(path, &pixels, w, h, image::ColorType::Rgba8).unwrap();
                }
            }
        }
    }
}

#[test]
fn font_gpu_large_titles_use_native_density_rasters() {
    use crate::system_font;
    let _reset = ResetFont;
    assert!(system_font::select("Abel"));
    let mut gpu = Gpu::new();
    for density in [1.0, 2.0, 3.0, 4.0] {
        system_font::set_pixel_scale(density);
        let mut canvas = Canvas::<()>::default();
        canvas.text(10.25, 20.25, 48.0, rgba(0xffffff, 1.0), "SETTINGS");
        let (w, h, pixels) = gpu.draw(&canvas.shapes, density);
        let mut checked = 0;
        let mut wrong = 0;
        for shape in &canvas.shapes {
            let glyph = system_font::glyph_in_slot((shape.params[3] - system_font::BASE) as usize);
            let unit = shape.half[1] * 2.0 / glyph.metrics.height as f32;
            assert!((unit * density - 1.0).abs() < 0.025);
            let origin = [
                shape.center[0] - shape.half[0],
                shape.center[1] - shape.half[1],
            ];
            for y in 0..h {
                for x in 0..w {
                    let gx = ((x as f32 + 0.5) / density - origin[0]) / unit;
                    let gy = ((y as f32 + 0.5) / density - origin[1]) / unit;
                    if gx < 0.0
                        || gy < 0.0
                        || gx >= glyph.metrics.width as f32
                        || gy >= glyph.metrics.height as f32
                    {
                        continue;
                    }
                    let actual = pixels[((y * w + x) * 4 + 3) as usize] as f32;
                    let expected = sample(&glyph, gx, gy);
                    checked += 1;
                    if (actual - expected).abs() > 3.0 {
                        wrong += 1;
                    }
                }
            }
        }
        assert!(checked > 1000);
        assert!(
            wrong * 100 < checked,
            "density {density}: {wrong}/{checked} pixels differ from the native raster"
        );
        if let Some(dir) = std::env::var_os("TAIKO_GPU_SHOTS") {
            let path = std::path::Path::new(&dir).join(format!("font-gpu-title-{density}.png"));
            image::save_buffer(path, &pixels, w, h, image::ColorType::Rgba8).unwrap();
        }
    }
}

#[test]
fn card_pictures_gpu_draw_their_own_strip() {
    let mut gpu = Gpu::new();
    let [w, h] = CARD_CELL;
    let strip = |rgb: [u8; 3]| -> Vec<u8> {
        (0..w * h)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    };
    // Its neighbours in the texture, beside and below it, in other colours.
    let per = CARD_SIDE[0] / w;
    let slot = per + 1;
    for (s, rgb) in [
        (slot - 1, [255, 0, 0]),
        (slot, [0, 255, 0]),
        (slot + 1, [0, 0, 255]),
        (slot - per, [255, 255, 0]),
        (slot + per, [255, 0, 255]),
    ] {
        write_card(&gpu.queue, &gpu.cards, s, &strip(rgb));
    }
    let path = std::path::Path::new("/songs/a/bg.png");
    let mut canvas = Canvas::<()>::default();
    canvas.card_slots = std::sync::Arc::new([(path_picture_key(path), slot)].into());
    // As wide as the target and a third of it tall: its middle, cut to it.
    assert!(canvas.card_picture(path, [8.0, 40.0, 240.0, 48.0], [1.0; 4], 0.0));
    let (width, _, pixels) = gpu.draw(&canvas.shapes, 1.0);
    let at = |x: u32, y: u32| {
        let i = ((y * width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    for (x, y) in [(10, 42), (128, 64), (245, 85), (128, 41), (128, 86)] {
        assert_eq!(at(x, y), [0, 255, 0, 255], "its own strip at {x}, {y}");
    }
    // As tall as the rect (its width is cut by the canvas's clip, which this doesn't apply).
    assert_eq!(at(128, 36)[3], 0, "nothing above it");
    assert_eq!(at(128, 92)[3], 0, "nothing below it");

    // Rounded: its corners are cut, the rest of it is its strip as before.
    let mut canvas = Canvas::<()>::default();
    canvas.card_slots = std::sync::Arc::new([(path_picture_key(path), slot)].into());
    assert!(canvas.card_picture(path, [8.0, 40.0, 240.0, 48.0], [1.0; 4], 8.0));
    let (width, _, pixels) = gpu.draw(&canvas.shapes, 1.0);
    let at = |x: u32, y: u32| {
        let i = ((y * width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    for (x, y) in [(8, 40), (247, 40), (8, 87), (247, 87)] {
        assert_eq!(at(x, y)[3], 0, "its corner at {x}, {y} is cut");
    }
    for (x, y) in [(20, 64), (128, 41), (240, 64)] {
        assert_eq!(at(x, y), [0, 255, 0, 255], "its own strip at {x}, {y}");
    }
}
