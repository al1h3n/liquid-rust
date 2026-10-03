//! Interactive playground.
//!
//! ```text
//! cargo run --release --example demo -- [background.jpg]
//! ```
//! Drag the clear lens (it fuses with the blobs), press any control, click `…` to morph
//! it into a menu. Keys: `1` `2` `3` transparency slider (ultra clear, default, fully
//! tinted), `D` dark scheme, `R` Reduce Transparency, `C` Increase Contrast, `M` Reduce
//! Motion, `L` sweep the light, `S` scroll the content under the glass.

use std::sync::Arc;
use std::time::Instant;

use liquid_rust::{Color, ContainerId, Frame, Glass, GlassId, Material, Rect, Renderer, Scene, Shape, Spring, Vec2};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const BACKGROUND_WGSL: &str = r"
struct U { offset: vec4f };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var img: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(xy * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(img));
    let uv = (p.xy * u.offset.z + vec2f(0.0, u.offset.y)) / size;
    return vec4f(textureSampleLevel(img, samp, uv, 0.0).rgb, 1.0);
}
";

struct Ui {
    menu_container: ContainerId,
    more: GlassId,
    menu: Option<GlassId>,
    pressed: Option<(GlassId, Vec2)>,
}

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    background: wgpu::RenderPipeline,
    background_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    image_size: (u32, u32),
    content: wgpu::Texture,
}

struct App {
    image: image::RgbaImage,
    gpu: Option<Gpu>,
    scene: Scene,
    ui: Option<Ui>,
    cursor: Vec2,
    last: Instant,
    scrolling: bool,
    scroll: f32,
    light: f32,
    content_dirty: bool,
    frames: u32,
    fps_clock: Instant,
}

fn build_scene(scene: &mut Scene, size: Vec2) -> Ui {
    let toolbar = scene.add_container(14.0, 0);
    for i in 0..3 {
        scene.add(Glass::new(Rect::new(24.0 + i as f32 * 60.0, 24.0, 52.0, 52.0)).interactive().container(toolbar));
    }
    let menu_container = scene.add_container(20.0, 2);
    let more = scene.add(Glass::new(Rect::new(220.0, 24.0, 52.0, 52.0)).interactive().container(menu_container));
    scene.add(Glass::new(Rect::new(size.x - 134.0, 24.0, 110.0, 52.0)).material(Material::tinted(Color::BLUE)).interactive());

    let liquid = scene.add_container(28.0, 0);
    scene.add(Glass::new(Rect::new(80.0, 220.0, 140.0, 140.0)).material(Material::clear()).container(liquid));
    scene.add(Glass::new(Rect::new(236.0, 260.0, 96.0, 96.0)).material(Material::clear()).container(liquid));
    scene.add(Glass::new(Rect::new(120.0, 420.0, 96.0, 96.0)).material(Material::clear()).lens().container(liquid));

    let tiles = scene.add_container(0.0, 0);
    let x0 = size.x - 340.0;
    scene.add(Glass::new(Rect::new(x0, 160.0, 155.0, 155.0)).shape(Shape::Rounded(30.0)).material(Material::clear()).interactive().container(tiles));
    for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let r = Rect::new(x0 + 170.0 + i as f32 * 85.0, 160.0 + j as f32 * 85.0, 70.0, 70.0);
        scene.add(Glass::new(r).material(Material::clear()).interactive().container(tiles));
    }
    scene.add(Glass::new(Rect::new(x0, 330.0, 155.0, 70.0)).material(Material::clear()).interactive().container(tiles));

    let tab_bar = Rect::new(size.x * 0.5 - 190.0, size.y - 96.0, 380.0, 64.0);
    scene.add(Glass::new(tab_bar).shape(Shape::Capsule));
    scene.add(Glass::new(Rect::new(tab_bar.x + 6.0, tab_bar.y + 6.0, 92.0, 52.0)).material(Material::clear()).lens().z(1));
    Ui { menu_container, more, menu: None, pressed: None }
}

impl App {
    fn toggle_menu(&mut self) {
        let Some(ui) = &mut self.ui else { return };
        let button = self.scene.glass(ui.more).map_or(Rect::default(), |g| g.frame);
        match ui.menu.take() {
            Some(menu) => {
                self.scene.set_frame_with(menu, button, Spring::new(0.4, 0.0));
                self.scene.remove(menu);
            }
            None => {
                let menu = self.scene.add(Glass::new(button).shape(Shape::Rounded(26.0)).container(ui.menu_container));
                self.scene.set_frame_with(menu, Rect::new(button.x, button.y + 64.0, 250.0, 300.0), Spring::new(0.45, 0.25));
                ui.menu = Some(menu);
            }
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32();
        self.last = now;
        let animating = self.scene.update(dt);
        if self.scrolling {
            self.scroll += dt * 120.0;
            self.content_dirty = true;
        }
        let Some(gpu) = &mut self.gpu else { return };

        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                gpu.window.request_redraw();
                return;
            }
            _ => return,
        };
        let target = frame.texture.create_view(&Default::default());
        let content = gpu.content.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        if self.content_dirty {
            let cover = (gpu.image_size.0 as f32 / gpu.config.width as f32).min(gpu.image_size.1 as f32 / gpu.config.height as f32);
            gpu.queue.write_buffer(&gpu.uniforms, 0, bytemuck::cast_slice(&[0.0f32, self.scroll, cover, 0.0]));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("demo content"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &content,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.background);
            pass.set_bind_group(0, &gpu.background_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let size = (gpu.config.width, gpu.config.height);
        let scale = gpu.window.scale_factor() as f32;
        let frame_info = Frame { content: &content, target: &target, size, scale, content_changed: self.content_dirty };
        gpu.renderer.render(&gpu.device, &gpu.queue, &mut encoder, &frame_info, &self.scene);
        gpu.queue.submit([encoder.finish()]);
        gpu.window.pre_present_notify();
        gpu.queue.present(frame);
        self.content_dirty = false;

        self.frames += 1;
        let elapsed = self.fps_clock.elapsed().as_secs_f32();
        if elapsed > 0.5 {
            gpu.window.set_title(&format!("Liquid Rust — {:.0} fps", self.frames as f32 / elapsed));
            self.frames = 0;
            self.fps_clock = Instant::now();
        }
        if animating || self.scrolling {
            gpu.window.request_redraw();
        }
    }

    fn key(&mut self, key: &Key) {
        let a = &mut self.scene.appearance;
        match key.as_ref() {
            Key::Character("1") => a.transparency = 0.0,
            Key::Character("2") => a.transparency = 0.5,
            Key::Character("3") => a.transparency = 1.0,
            Key::Character("d") => a.dark = !a.dark,
            Key::Character("r") => a.reduce_transparency = !a.reduce_transparency,
            Key::Character("c") => a.increase_contrast = !a.increase_contrast,
            Key::Character("m") => a.reduce_motion = !a.reduce_motion,
            Key::Character("s") => self.scrolling = !self.scrolling,
            Key::Character("l") => {
                self.light += 90.0;
                self.scene.set_light_angle(self.light);
            }
            Key::Named(NamedKey::Space) => self.toggle_menu(),
            _ => return,
        }
        println!("{:?}", self.scene.appearance);
    }
}

fn init_gpu(window: Arc<Window>, image: &image::RgbaImage) -> Gpu {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let surface = instance.create_surface(window.clone()).expect("surface");
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .expect("GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
    let size = window.inner_size();
    let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
    config.format = config.format.add_srgb_suffix();
    config.present_mode = wgpu::PresentMode::AutoVsync;
    surface.configure(&device, &config);

    let image_size = image.dimensions();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("demo image"),
        size: wgpu::Extent3d { width: image_size.0, height: image_size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        image.as_raw(),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * image_size.0), rows_per_image: Some(image_size.1) },
        texture.size(),
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("demo background"),
        source: wgpu::ShaderSource::Wgsl(BACKGROUND_WGSL.into()),
    });
    let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("demo background"),
        layout: None,
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(config.format.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("demo uniforms"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::MirrorRepeat,
        address_mode_v: wgpu::AddressMode::MirrorRepeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let view = texture.create_view(&Default::default());
    let background_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("demo background"),
        layout: &background.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
        ],
    });
    let content = content_texture(&device, &config);
    Gpu {
        renderer: Renderer::new(&device, config.format),
        window,
        surface,
        device,
        queue,
        config,
        background,
        background_group,
        uniforms,
        image_size,
        content,
    }
}

fn content_texture(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("demo content"),
        size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attributes = Window::default_attributes().with_title("Liquid Rust").with_inner_size(LogicalSize::new(1100.0, 760.0));
        let window = Arc::new(event_loop.create_window(attributes).expect("window"));
        let logical = window.inner_size().to_logical::<f32>(window.scale_factor());
        self.ui = Some(build_scene(&mut self.scene, Vec2::new(logical.width, logical.height)));
        self.gpu = Some(init_gpu(window.clone(), &self.image));
        self.last = Instant::now();
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(gpu) = &mut self.gpu else { return };
        let scale = gpu.window.scale_factor() as f32;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.config.width = size.width.max(1);
                gpu.config.height = size.height.max(1);
                gpu.surface.configure(&gpu.device, &gpu.config);
                gpu.content = content_texture(&gpu.device, &gpu.config);
                self.content_dirty = true;
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                return;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Vec2::new(position.x as f32, position.y as f32) / scale;
                self.scene.pointer_move(self.cursor);
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let hit = self.scene.pointer_down(self.cursor);
                if let Some(ui) = &mut self.ui {
                    ui.pressed = hit.map(|id| (id, self.cursor));
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.scene.pointer_up();
                let clicked_more = self
                    .ui
                    .as_mut()
                    .and_then(|ui| ui.pressed.take().map(|(id, at)| id == ui.more && at.distance(self.cursor) < 8.0))
                    .unwrap_or(false);
                if clicked_more {
                    self.toggle_menu();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => self.key(&event.logical_key),
            _ => return,
        }
        if let Some(gpu) = &self.gpu {
            gpu.window.request_redraw();
        }
    }
}

fn main() {
    let image = match std::env::args().nth(1) {
        Some(path) => image::open(path).expect("readable background image").to_rgba8(),
        None => image::RgbaImage::from_fn(1600, 1200, |x, y| {
            let (u, v) = (x as f32 / 1600.0, y as f32 / 1200.0);
            let stripe = if (y / 22) % 3 == 0 && (x / 11) % 6 != 0 { 0.4 } else { 0.0 };
            let c = |f: f32| ((f * (1.0 - stripe)).clamp(0.0, 1.0) * 255.0) as u8;
            image::Rgba([c(0.5 + 0.5 * (7.0 * u).sin()), c(0.5 + 0.5 * (5.0 * v + 1.0).sin()), c(0.6 + 0.4 * (4.0 * (u - v)).cos()), 255])
        }),
    };
    println!("Drag the lenses, press controls, click … (or Space) for the menu.");
    println!("Keys: 1/2/3 transparency, D dark, R reduce transparency, C contrast, M reduce motion, L light, S scroll");
    let mut app = App {
        image,
        gpu: None,
        scene: Scene::new(),
        ui: None,
        cursor: Vec2::ZERO,
        last: Instant::now(),
        scrolling: false,
        scroll: 0.0,
        light: 0.0,
        content_dirty: true,
        frames: 0,
        fps_clock: Instant::now(),
    };
    EventLoop::new().expect("event loop").run_app(&mut app).expect("run");
}
