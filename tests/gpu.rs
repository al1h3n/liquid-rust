//! Renders one frame on a real adapter: validates the WGSL and the compositing
//! contract. Skips when the machine has no GPU adapter.

use liquid_rust::{Frame, Glass, Material, Rect, Renderer, Scene};

const SIZE: u32 = 128;

fn render(scene: &Scene) -> Option<Vec<u8>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let size = wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 };
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
    let stripes: Vec<u8> = (0..SIZE * SIZE).flat_map(|i| if (i / SIZE / 8).is_multiple_of(2) { [40, 90, 200, 255] } else { [230, 200, 60, 255] }).collect();
    queue.write_texture(content.as_image_copy(), &stripes, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * SIZE), rows_per_image: None }, size);

    let mut renderer = Renderer::new(&device, format);
    let (content_view, target_view) = (content.create_view(&Default::default()), target.create_view(&Default::default()));
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(4 * SIZE * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let frame = Frame { content: &content_view, target: &target_view, size: (SIZE, SIZE), scale: 2.0, content_changed: true };
    renderer.render(&device, &queue, &mut encoder, &frame, scene);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * SIZE), rows_per_image: None } },
        size,
    );
    queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
    let pixels = readback.get_mapped_range(..).ok()?.to_vec();
    Some(pixels)
}

fn pixel(data: &[u8], x: u32, y: u32) -> &[u8] {
    let i = (4 * (y * SIZE + x)) as usize;
    &data[i..i + 4]
}

#[test]
fn glass_changes_pixels_inside_and_leaves_far_pixels_untouched() {
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(8.0, 8.0, 40.0, 40.0)).material(Material::clear()));
    while scene.update(1.0 / 60.0) {}
    let Some(out) = render(&scene) else { return };
    let row = |y: u32| if (y / 8).is_multiple_of(2) { [40u8, 90, 200, 255] } else { [230, 200, 60, 255] };
    assert_eq!(pixel(&out, 120, 120), row(120));
    assert_ne!(pixel(&out, 20, 56), row(56), "glass must alter the backdrop inside its bevel");
}

#[test]
fn identity_glass_renders_nothing() {
    let mut scene = Scene::new();
    scene.add(Glass::new(Rect::new(8.0, 8.0, 40.0, 40.0)).material(Material::identity()));
    while scene.update(1.0 / 60.0) {}
    let Some(out) = render(&scene) else { return };
    let row = |y: u32| if (y / 8).is_multiple_of(2) { [40u8, 90, 200, 255] } else { [230, 200, 60, 255] };
    assert!((0..SIZE).all(|y| pixel(&out, 50, y) == row(y)));
}
