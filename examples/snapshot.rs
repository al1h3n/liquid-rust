//! Renders a scene offscreen and writes a PNG. No window needed.
//!
//! ```text
//! cargo run --release --example snapshot -- [background.png] [out.png] [lock|home|corners|showcase|press|search|bench]
//! ```
//! An optional fourth argument sets the appearance: `ultra-clear`, `tinted`, `dark`,
//! `reduce-transparency` or `increase-contrast`.
//! `bench` renders the showcase 300 times per case and prints the average frame time: glass
//! at rest and a sheet springing over the panel below it, over static and changing content.
//! `search` catches a results panel mid-way through growing out of a search bar.
//! `lock` and `home` recreate Apple's iOS 26/27 Figma kit controls at their kit
//! positions (402 x 874 pt at 3x) so the result can be compared with the kit render.

use liquid_rust::{Color, Frame, Glass, GlassId, Material, Rect, Renderer, Scene, Shape, Spring, Vec2};

const SCALE: f32 = 3.0;
const SIZE_PT: (f32, f32) = (402.0, 874.0);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Apple kit Lock Screen: two Clear Glass buttons.
fn kit_lock(scene: &mut Scene) {
    for x in [46.0, 298.0] {
        scene.add(Glass::new(Rect::new(x, 766.0, 58.0, 58.0)).material(Material::clear()).interactive());
    }
}

/// Apple kit Home Screen: dock, search pill, Quick Actions menu.
fn kit_home(scene: &mut Scene) {
    scene.add(Glass::new(Rect::new(17.0, 754.0, 368.0, 103.0)).shape(Shape::Rounded(38.0)).material(Material::clear_bar()));
    scene.add(Glass::new(Rect::new(162.5, 704.0, 77.0, 30.0)).material(Material::clear_bar()));
    scene.add(Glass::new(Rect::new(28.0, 120.0, 250.0, 350.0)).shape(Shape::Rounded(30.0)).material(Material::regular()));
}

/// Search bar whose results panel is still growing out of it (`Scene::expand_from`).
fn search(scene: &mut Scene) {
    let bar = scene.add_container(16.0, 0);
    let field = scene.add(Glass::new(Rect::new(40.0, 120.0, 322.0, 52.0)).interactive().container(bar));
    while scene.update(1.0 / 120.0) {}
    scene.expand_from(field, Glass::new(Rect::new(40.0, 192.0, 322.0, 420.0)).shape(Shape::Rounded(30.0)), Spring::new(0.5, 0.2));
    for _ in 0..SEARCH_FRAMES {
        scene.update(1.0 / 120.0);
    }
}

const SEARCH_FRAMES: u32 = 12;

/// The upper sheet of the showcase, at rest over the lower half of the panel.
const SHEET: Rect = Rect::new(150.0, 600.0, 200.0, 160.0);

/// Toolbar, primary action, liquid pair and a panel with a sheet over it. Returns the sheet.
fn showcase(scene: &mut Scene) -> GlassId {
    let bar = scene.add_container(18.0, 0);
    for (i, w) in [(0, 58.0), (1, 58.0), (2, 58.0)] {
        scene.add(Glass::new(Rect::new(40.0 + i as f32 * 66.0, 120.0, w, 58.0)).interactive().container(bar));
    }
    scene.add(Glass::new(Rect::new(250.0, 120.0, 110.0, 58.0)).material(Material::tinted(Color::BLUE)).interactive());
    let blobs = scene.add_container(30.0, 0);
    scene.add(Glass::new(Rect::new(60.0, 300.0, 120.0, 120.0)).material(Material::clear()).container(blobs));
    scene.add(Glass::new(Rect::new(190.0, 330.0, 90.0, 90.0)).material(Material::clear()).container(blobs));
    scene.add(Glass::new(Rect::new(40.0, 520.0, 322.0, 200.0)).shape(Shape::Rounded(34.0)));
    scene.add(Glass::new(SHEET).shape(Shape::Rounded(28.0)).z(1))
}

/// Same tile three ways: circular corners, Apple's continuous corners, Figma 100 %.
fn corners(scene: &mut Scene) {
    let shapes = [Shape::Circular(60.0), Shape::Rounded(60.0), Shape::Smooth { radius: 60.0, smoothing: 1.0 }];
    for (i, shape) in shapes.into_iter().enumerate() {
        scene.add(Glass::new(Rect::new(31.0, 40.0 + i as f32 * 270.0, 340.0, 250.0)).shape(shape).material(Material::clear()));
    }
}

fn procedural(w: u32, h: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(w, h, |x, y| {
        let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
        let stripe = if (y / 18) % 3 == 0 && (x / 9) % 7 != 0 { 0.35 } else { 0.0 };
        let r = 0.5 + 0.5 * (6.0 * u + 1.0).sin();
        let g = 0.5 + 0.5 * (5.0 * v + 2.0).sin();
        let b = 0.5 + 0.5 * (4.0 * (u + v)).cos();
        let px = |c: f32| ((c * (1.0 - stripe)).clamp(0.0, 1.0) * 255.0) as u8;
        image::Rgba([px(r), px(g), px(b), 255])
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (w, h) = ((SIZE_PT.0 * SCALE) as u32, (SIZE_PT.1 * SCALE) as u32);
    let background = match args.first().filter(|a| a.as_str() != "-") {
        Some(path) => image::open(path).expect("readable background image").resize_exact(w, h, image::imageops::FilterType::CatmullRom).to_rgba8(),
        None => procedural(w, h),
    };
    let out = args.get(1).map_or("snapshot.png", String::as_str);
    let which = args.get(2).map_or("home", String::as_str);

    let mut scene = Scene::new();
    let a = &mut scene.appearance;
    match args.get(3).map(String::as_str) {
        Some("ultra-clear") => a.transparency = 0.0,
        Some("tinted") => a.transparency = 1.0,
        Some("dark") => a.dark = true,
        Some("reduce-transparency") => a.reduce_transparency = true,
        Some("increase-contrast") => a.increase_contrast = true,
        _ => {}
    }
    let mut sheet = None;
    match which {
        "showcase" | "press" | "bench" => sheet = Some(showcase(&mut scene)),
        "lock" => kit_lock(&mut scene),
        "corners" => corners(&mut scene),
        "search" => search(&mut scene),
        _ => kit_home(&mut scene),
    }
    if which != "search" {
        while scene.update(1.0 / 120.0) {}
    }
    if which == "press" {
        scene.pointer_down(Vec2::new(69.0, 149.0));
        for _ in 0..12 {
            scene.update(1.0 / 120.0);
        }
    }

    let gpu = pollster::block_on(Offscreen::new(&background));
    let mut renderer = Renderer::new(&gpu.device, FORMAT);
    if let (Some(sheet), "bench") = (sheet, which) {
        bench(&gpu, &mut renderer, &mut scene, sheet);
    }
    gpu.draw(&mut renderer, &scene, true);
    gpu.save(out);
    println!("wrote {out}");
}

const BENCH_FRAMES: u32 = 300;

/// Average frame time with the glass at rest and with the sheet springing up and down over
/// the panel below it, each over content that holds still and content that changes.
fn bench(gpu: &Offscreen, renderer: &mut Renderer, scene: &mut Scene, sheet: GlassId) {
    let (w, h) = gpu.size;
    println!("{}, {w}x{h}, {BENCH_FRAMES} frames per case", gpu.adapter);
    let up = Rect { y: SHEET.y - 130.0, ..SHEET };
    for (label, moving, content_changed) in [
        ("glass at rest, static content", false, false),
        ("glass at rest, changing content", false, true),
        ("glass moving, static content", true, false),
        ("glass moving, changing content", true, true),
    ] {
        gpu.draw(renderer, scene, true);
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU finished");
        let start = std::time::Instant::now();
        for i in 0..BENCH_FRAMES {
            if moving {
                if i % 60 == 0 {
                    scene.set_frame(sheet, if (i / 60) % 2 == 0 { up } else { SHEET });
                }
                scene.update(1.0 / 120.0);
            }
            gpu.draw(renderer, scene, content_changed);
        }
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU finished");
        let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(BENCH_FRAMES);
        println!("{label:>32}: {ms:.3} ms/frame");
        scene.set_frame(sheet, SHEET);
        while scene.update(1.0 / 120.0) {}
    }
}

/// A device with the background in a content texture and a target to draw into.
struct Offscreen {
    adapter: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    content: wgpu::TextureView,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    size: (u32, u32),
}

impl Offscreen {
    async fn new(background: &image::RgbaImage) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.expect("GPU adapter");
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.expect("GPU device");
        let (w, h) = background.dimensions();
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let texture = |usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage,
                view_formats: &[],
            })
        };
        let content = texture(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let target = texture(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        queue.write_texture(
            content.as_image_copy(),
            background.as_raw(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            size,
        );
        Self {
            adapter: adapter.get_info().name,
            content: content.create_view(&Default::default()),
            target_view: target.create_view(&Default::default()),
            target,
            device,
            queue,
            size: (w, h),
        }
    }

    fn draw(&self, renderer: &mut Renderer, scene: &Scene, content_changed: bool) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let frame = Frame { content: &self.content, target: &self.target_view, size: self.size, scale: SCALE, content_changed };
        renderer.render(&self.device, &self.queue, &mut encoder, &frame, scene);
        self.queue.submit([encoder.finish()]);
    }

    /// Reads the target back and writes it to `out` as a PNG.
    fn save(&self, out: &str) {
        let (w, h) = self.size;
        let row = (4 * w).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            self.target.size(),
        );
        self.queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("map readback"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU finished");
        let data = readback.get_mapped_range(..).expect("mapped readback");
        let mut pixels = Vec::with_capacity((4 * w * h) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            pixels.extend_from_slice(&data[start..start + (4 * w) as usize]);
        }
        image::RgbaImage::from_raw(w, h, pixels).expect("pixel buffer").save(out).expect("write png");
    }
}
