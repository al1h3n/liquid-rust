//! Renders frames on a real adapter: validates the WGSL, the compositing contract and
//! the backdrop caches. Skips when the machine has no GPU adapter.

use liquid_rust::{Color, Frame, Glass, Material, Rect, Renderer, Scene, Shadow, Shape};

const SIZE: u32 = 128;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn stripes() -> Vec<u8> {
    (0..SIZE * SIZE).flat_map(|i| if (i / SIZE / 8).is_multiple_of(2) { [40, 90, 200, 255] } else { [230, 200, 60, 255] }).collect()
}

/// A device with `backdrop` as the content and a target to draw into.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    content: wgpu::TextureView,
    target: wgpu::Texture,
}

impl Gpu {
    fn new(backdrop: &[u8]) -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let content = texture(&device, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let target = texture(&device, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        queue.write_texture(content.as_image_copy(), backdrop, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * SIZE), rows_per_image: None }, content.size());
        Some(Self { content: content.create_view(&Default::default()), target, device, queue })
    }

    /// Draws one frame of `size` pixels with `renderer` and reads the whole target back.
    fn draw(&self, renderer: &mut Renderer, scene: &Scene, size: (u32, u32), scale: f32, content_changed: bool) -> Option<Vec<u8>> {
        let target = self.target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let frame = Frame { content: &self.content, target: &target, size, scale, content_changed };
        renderer.render(&self.device, &self.queue, &mut encoder, &frame, scene);
        self.queue.submit([encoder.finish()]);
        self.read()
    }

    /// Reads the whole target back.
    fn read(&self) -> Option<Vec<u8>> {
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(4 * SIZE * SIZE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * SIZE), rows_per_image: None } },
            self.target.size(),
        );
        self.queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let pixels = readback.get_mapped_range(..).ok()?.to_vec();
        Some(pixels)
    }
}

fn texture(device: &wgpu::Device, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage,
        view_formats: &[],
    })
}

fn render(scene: &Scene) -> Option<Vec<u8>> {
    render_over(scene, &stripes())
}

fn render_over(scene: &Scene, backdrop: &[u8]) -> Option<Vec<u8>> {
    render_at(scene, backdrop, 2.0)
}

/// One frame of `scene` from a fresh renderer, at `scale` pixels per point.
fn render_at(scene: &Scene, backdrop: &[u8], scale: f32) -> Option<Vec<u8>> {
    let gpu = Gpu::new(backdrop)?;
    gpu.draw(&mut Renderer::new(&gpu.device, FORMAT), scene, (SIZE, SIZE), scale, true)
}

fn pixel(data: &[u8], x: u32, y: u32) -> &[u8] {
    let i = (4 * (y * SIZE + x)) as usize;
    &data[i..i + 4]
}

/// Largest difference between two frames in any channel of any pixel.
fn difference(a: &[u8], b: &[u8]) -> u8 {
    a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0)
}

fn settle(scene: &mut Scene) {
    while scene.update(1.0 / 60.0) {}
}

/// Clear glass with no frost or shadow: it reaches only a few pixels past its outline, so
/// layers overlap exactly where a test puts them (drawn at 1 px per point).
fn bare() -> Material {
    Material { frost: 0.0, shadow: Shadow { offset_y: 0.0, blur: 0.0, opacity: 0.0 }, ..Material::clear() }
}

#[test]
fn glass_changes_pixels_inside_and_leaves_far_pixels_untouched() {
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(8.0, 8.0, 40.0, 40.0)).material(Material::clear()));
    settle(&mut scene);
    let Some(out) = render(&scene) else { return };
    let row = |y: u32| if (y / 8).is_multiple_of(2) { [40u8, 90, 200, 255] } else { [230, 200, 60, 255] };
    assert_eq!(pixel(&out, 120, 120), row(120));
    assert_ne!(pixel(&out, 20, 56), row(56), "glass must alter the backdrop inside its bevel");
}

#[test]
fn identity_glass_renders_nothing() {
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(8.0, 8.0, 40.0, 40.0)).material(Material::identity()));
    settle(&mut scene);
    let Some(out) = render(&scene) else { return };
    let row = |y: u32| if (y / 8).is_multiple_of(2) { [40u8, 90, 200, 255] } else { [230, 200, 60, 255] };
    assert!((0..SIZE).all(|y| pixel(&out, 50, y) == row(y)));
}

/// A flat backdrop of one sRGB grey.
fn flat(value: u8) -> Vec<u8> {
    (0..SIZE * SIZE).flat_map(|_| [value, value, value, 255]).collect()
}

#[test]
fn edge_never_turns_a_dark_backdrop_black() {
    // A dark map: the kit's linear burn used to clip the outer hairline to pure black.
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(10.0, 10.0, 40.0, 24.0)).material(Material::regular()));
    settle(&mut scene);
    let Some(out) = render_over(&scene, &flat(30)) else { return };
    // The glass spans x 20..100 px at scale 2; check the 6 px just outside its left and
    // right edges along its middle row, where the side hairlines are strongest.
    let y = 44;
    for x in (14..20).chain(100..106) {
        let p = pixel(&out, x, y);
        assert!(p[..3].iter().all(|&c| c >= 20), "pixel ({x}, {y}) went near-black: {p:?}");
    }
}

#[test]
fn edge_still_darkens_a_light_backdrop() {
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(10.0, 10.0, 40.0, 24.0)).material(Material::regular()));
    settle(&mut scene);
    let Some(out) = render_over(&scene, &flat(235)) else { return };
    let darkest = (14..20).map(|x| pixel(&out, x, 44)[0]).min().unwrap_or(255);
    assert!(darkest < 225, "the iOS 27 hairline must still show over light content: {darkest}");
}

#[test]
fn rim_pixels_are_never_black() {
    // Over a light backdrop no pixel may come out black: NaN refraction at the outermost
    // rim pixels (asin of a ratio rounded above 1) used to leave black specks there.
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(12.0, 12.0, 36.0, 36.0)).material(Material::clear()));
    scene.add(Glass::new(Rect::new(8.0, 52.0, 48.0, 8.0)).material(Material::regular()));
    settle(&mut scene);
    let Some(out) = render_over(&scene, &flat(200)) else { return };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = pixel(&out, x, y);
            assert!(p[..3].iter().any(|&c| c > 60), "black pixel at ({x}, {y}): {p:?}");
        }
    }
}

#[test]
fn without_glass_on_glass_upper_glass_shows_only_the_content() {
    // Apple: glass can't sample glass. A menu over a button must show the backdrop, not a
    // refracted ghost of the button below it.
    let menu = Glass::new(Rect::new(8.0, 8.0, 48.0, 48.0)).shape(Shape::Rounded(12.0)).z(1);
    let mut alone = Scene::new();
    alone.add(menu);
    let mut stacked = Scene::new();
    stacked.glass_on_glass = false;
    stacked.add(Glass::new(Rect::new(20.0, 20.0, 24.0, 24.0)).material(Material::clear()));
    stacked.add(menu);
    for scene in [&mut alone, &mut stacked] {
        settle(scene);
    }
    let (Some(alone), Some(stacked)) = (render(&alone), render(&stacked)) else { return };
    for (x, y) in [(64, 64), (50, 70), (80, 60)] {
        assert_eq!(pixel(&stacked, x, y), pixel(&alone, x, y), "({x}, {y}) shows the glass below");
    }
}

#[test]
fn upper_glass_refracts_every_layer_below_it() {
    // Red glass at the bottom, clear glass two layers up, and a layer in between that
    // touches neither. The backdrop of the top layer used to hold only the layer right
    // below it, so the red glass vanished from under the clear one.
    let red = Glass::new(Rect::new(4.0, 4.0, 40.0, 40.0)).material(Material { tint: Some(Color::RED), ..bare() });
    let top = Glass::new(Rect::new(16.0, 16.0, 40.0, 40.0)).material(bare());
    let mut two = Scene::new();
    two.add(red);
    two.add(top.z(1));
    let mut three = Scene::new();
    three.add(red);
    three.add(Glass::new(Rect::new(100.0, 100.0, 16.0, 16.0)).material(bare()).z(1));
    three.add(top.z(2));
    for scene in [&mut two, &mut three] {
        settle(scene);
    }
    let (Some(two), Some(three)) = (render_at(&two, &stripes(), 1.0), render_at(&three, &stripes(), 1.0)) else { return };
    // Flat centre of the clear glass, over the red one.
    for (x, y) in [(30, 30), (36, 28), (28, 38)] {
        let (a, b) = (pixel(&two, x, y), pixel(&three, x, y));
        assert!(difference(a, b) <= 2, "({x}, {y}) lost the red glass below: {b:?} instead of {a:?}");
    }
}

#[test]
fn layers_that_skip_a_rebuild_still_render() {
    // Layers 1 and 3 overlap glass below them and layer 2 does not. Picking the backdrop
    // texture by layer index made layer 3 read and write the same texture, which wgpu rejects.
    let mut scene = Scene::new();
    for (z, x) in [(0, 4.0), (1, 12.0), (2, 100.0), (3, 8.0)] {
        scene.add(Glass::new(Rect::new(x, x, 16.0, 16.0)).material(bare()).z(z));
    }
    settle(&mut scene);
    let Some(out) = render_at(&scene, &stripes(), 1.0) else { return };
    assert_eq!(pixel(&out, 70, 20), [40, 90, 200, 255], "content away from the glass is untouched");
}

#[test]
fn cached_backdrop_draws_the_same_picture() {
    // A sheet springs across a panel over content that holds still, so every frame after
    // the first reuses the cached blur. Each must match a renderer that caches nothing.
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(6.0, 6.0, 52.0, 28.0)).shape(Shape::Rounded(10.0)));
    let sheet = scene.add(Glass::new(Rect::new(8.0, 30.0, 24.0, 24.0)).material(Material::clear()).z(1));
    settle(&mut scene);
    let Some(gpu) = Gpu::new(&stripes()) else { return };
    let mut cached = Renderer::new(&gpu.device, FORMAT);
    scene.set_frame(sheet, Rect::new(34.0, 4.0, 24.0, 24.0));
    for frame in 0..12 {
        scene.update(1.0 / 30.0);
        let (Some(reused), Some(fresh)) = (
            gpu.draw(&mut cached, &scene, (SIZE, SIZE), 2.0, frame == 0),
            gpu.draw(&mut Renderer::new(&gpu.device, FORMAT), &scene, (SIZE, SIZE), 2.0, true),
        ) else {
            return;
        };
        assert!(difference(&reused, &fresh) <= 1, "frame {frame} differs from an uncached render");
    }
}

#[test]
fn layers_drawn_in_two_passes_match_one() {
    // The lower layer goes into a texture that becomes the content of the upper one, which
    // is how a host puts labels between layers. With no labels the picture must not change.
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(6.0, 6.0, 40.0, 40.0)).material(Material::tinted(Color::RED)));
    scene.add(Glass::new(Rect::new(20.0, 20.0, 36.0, 36.0)).material(Material::clear()).z(1));
    settle(&mut scene);
    let Some(gpu) = Gpu::new(&stripes()) else { return };
    let Some(one) = gpu.draw(&mut Renderer::new(&gpu.device, FORMAT), &scene, (SIZE, SIZE), 2.0, true) else { return };

    let middle = texture(&gpu.device, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
    let target = gpu.target.create_view(&Default::default());
    let frame = |content, target| Frame { content, target, size: (SIZE, SIZE), scale: 2.0, content_changed: true };
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    Renderer::new(&gpu.device, FORMAT).render_layers(&gpu.device, &gpu.queue, &mut encoder, &frame(&gpu.content, &middle), &scene, ..1);
    Renderer::new(&gpu.device, FORMAT).render_layers(&gpu.device, &gpu.queue, &mut encoder, &frame(&middle, &target), &scene, 1..);
    gpu.queue.submit([encoder.finish()]);
    let Some(two) = gpu.read() else { return };
    assert!(difference(&one, &two) <= 2, "two passes differ from one by {}", difference(&one, &two));
    assert_ne!(pixel(&two, 60, 60), pixel(&stripes(), 60, 60), "the upper layer was drawn");
}

#[test]
fn empty_frame_is_skipped() {
    // A minimized window is 0 x 0; there is no texture that small to build a pyramid for.
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(8.0, 8.0, 40.0, 40.0)));
    settle(&mut scene);
    let Some(gpu) = Gpu::new(&stripes()) else { return };
    let mut renderer = Renderer::new(&gpu.device, FORMAT);
    for size in [(0, 0), (SIZE, 0), (0, SIZE)] {
        gpu.draw(&mut renderer, &scene, size, 2.0, true);
    }
}
