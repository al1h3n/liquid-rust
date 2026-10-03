//! Renders a scene offscreen and writes a PNG. No window needed.
//!
//! ```text
//! cargo run --release --example snapshot -- [background.png] [out.png] [lock|home|corners|showcase|press|bench]
//! ```
//! An optional fourth argument sets the appearance: `ultra-clear`, `tinted`, `dark`,
//! `reduce-transparency` or `increase-contrast`.
//! `bench` renders the showcase 300 times with a cached and with a changing backdrop
//! and prints the average GPU frame time.
//! `lock` and `home` recreate Apple's iOS 26/27 Figma kit controls at their kit
//! positions (402 x 874 pt at 3x) so the result can be compared with the kit render.

use liquid_rust::{Color, Frame, Glass, Material, Rect, Renderer, Scene, Shape, Vec2};

const SCALE: f32 = 3.0;
const SIZE_PT: (f32, f32) = (402.0, 874.0);

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

fn showcase(scene: &mut Scene) {
    let bar = scene.add_container(18.0, 0);
    for (i, w) in [(0, 58.0), (1, 58.0), (2, 58.0)] {
        scene.add(Glass::new(Rect::new(40.0 + i as f32 * 66.0, 120.0, w, 58.0)).interactive().container(bar));
    }
    scene.add(Glass::new(Rect::new(250.0, 120.0, 110.0, 58.0)).material(Material::tinted(Color::BLUE)).interactive());
    let blobs = scene.add_container(30.0, 0);
    scene.add(Glass::new(Rect::new(60.0, 300.0, 120.0, 120.0)).material(Material::clear()).container(blobs));
    scene.add(Glass::new(Rect::new(190.0, 330.0, 90.0, 90.0)).material(Material::clear()).container(blobs));
    scene.add(Glass::new(Rect::new(40.0, 520.0, 322.0, 200.0)).shape(Shape::Rounded(34.0)));
    scene.add(Glass::new(Rect::new(150.0, 600.0, 200.0, 160.0)).shape(Shape::Rounded(28.0)).z(1));
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
    match which {
        "showcase" | "press" | "bench" => showcase(&mut scene),
        "lock" => kit_lock(&mut scene),
        "corners" => corners(&mut scene),
        _ => kit_home(&mut scene),
    }
    while scene.update(1.0 / 120.0) {}
    if which == "press" {
        scene.pointer_down(Vec2::new(69.0, 149.0));
        for _ in 0..12 {
            scene.update(1.0 / 120.0);
        }
    }

    let frames = if which == "bench" { 300 } else { 1 };
    pollster::block_on(render(&scene, &background, out, frames));
    println!("wrote {out}");
}

async fn render(scene: &Scene, background: &image::RgbaImage, out: &str, frames: u32) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.expect("GPU adapter");
    let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.expect("GPU device");
    let (w, h) = background.dimensions();
    let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = |usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
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

    let mut renderer = Renderer::new(&device, format);
    let content_view = content.create_view(&Default::default());
    let target_view = target.create_view(&Default::default());
    for content_changed in [false, true] {
        if frames < 2 {
            break;
        }
        let start = std::time::Instant::now();
        for _ in 0..frames {
            let mut encoder = device.create_command_encoder(&Default::default());
            let frame = Frame { content: &content_view, target: &target_view, size: (w, h), scale: SCALE, content_changed };
            renderer.render(&device, &queue, &mut encoder, &frame, scene);
            queue.submit([encoder.finish()]);
        }
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU finished");
        let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        println!("{w}x{h}, content_changed={content_changed}: {ms:.3} ms/frame");
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    let frame = Frame { content: &content_view, target: &target_view, size: (w, h), scale: SCALE, content_changed: true };
    renderer.render(&device, &queue, &mut encoder, &frame, scene);

    let row = (4 * w).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
        },
        size,
    );
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("map readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU finished");
    let data = readback.get_mapped_range(..).expect("mapped readback");
    let mut pixels = Vec::with_capacity((4 * w * h) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        pixels.extend_from_slice(&data[start..start + (4 * w) as usize]);
    }
    image::RgbaImage::from_raw(w, h, pixels).expect("pixel buffer").save(out).expect("write png");
}
