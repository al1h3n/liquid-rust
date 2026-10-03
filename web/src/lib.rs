//! Liquid Rust in the browser: WebGPU through wgpu, driven from JavaScript.
#![cfg(target_arch = "wasm32")]

use liquid_rust::{Color, ContainerId, Frame, Glass, GlassId, Material, Rect, Renderer, Scene, Shape, Vec2};
use wasm_bindgen::prelude::*;

/// Glass rendered over an image on a `<canvas>`.
#[wasm_bindgen]
pub struct LiquidGlass {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    renderer: Renderer,
    content: wgpu::Texture,
    content_changed: bool,
    scene: Scene,
    glass: Vec<GlassId>,
    containers: Vec<ContainerId>,
    scale: f32,
}

#[wasm_bindgen]
impl LiquidGlass {
    /// Creates the renderer for `canvas` (sized in device pixels) with `background`
    /// drawn behind the glass. `scale` is `devicePixelRatio`.
    pub async fn create(canvas: web_sys::HtmlCanvasElement, background: web_sys::ImageBitmap, scale: f32) -> Result<LiquidGlass, JsValue> {
        let (width, height) = (canvas.width(), canvas.height());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(|e| e.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() })
            .await
            .map_err(|e| e.to_string())?;
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.map_err(|e| e.to_string())?;
        let mut config = surface.get_default_config(&adapter, width, height).ok_or("canvas not supported")?;
        // Canvases are bgra8unorm; draw through an sRGB view so shader output gets encoded.
        let view_format = config.format.add_srgb_suffix();
        config.view_formats = vec![view_format];
        config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        surface.configure(&device, &config);

        let content = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("content"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        queue.copy_external_image_to_texture(
            &wgpu::CopyExternalImageSourceInfo {
                source: wgpu::ExternalImageSource::ImageBitmap(background.clone()),
                origin: wgpu::Origin2d::ZERO,
                flip_y: false,
            },
            wgpu::CopyExternalImageDestInfo {
                texture: &content,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
                color_space: wgpu::PredefinedColorSpace::Srgb,
                premultiplied_alpha: false,
            },
            wgpu::Extent3d { width: background.width().min(width), height: background.height().min(height), depth_or_array_layers: 1 },
        );
        Ok(LiquidGlass {
            renderer: Renderer::new(&device, view_format),
            surface,
            device,
            queue,
            config,
            view_format,
            content,
            content_changed: true,
            scene: Scene::new(),
            glass: Vec::new(),
            containers: Vec::new(),
            scale,
        })
    }

    /// `GlassEffectContainer(spacing:)`; returns a handle.
    #[wasm_bindgen(js_name = addContainer)]
    pub fn add_container(&mut self, spacing: f32, z: i32) -> u32 {
        self.containers.push(self.scene.add_container(spacing, z));
        self.containers.len() as u32 - 1
    }

    /// Adds glass in CSS pixels. `material`: "regular", "clear", "clear-bar" or "tinted"
    /// (with `tint` as 0xRRGGBB). `radius < 0` makes a capsule; `container < 0` means none.
    #[expect(clippy::too_many_arguments, reason = "flat JS binding")]
    pub fn add(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, material: &str, tint: u32, interactive: bool, lens: bool, container: i32, z: i32) -> u32 {
        let material = match material {
            "clear" => Material::clear(),
            "clear-bar" => Material::clear_bar(),
            "tinted" => Material::tinted(Color::hex(tint)),
            _ => Material::regular(),
        };
        let mut glass = Glass::new(Rect::new(x, y, w, h)).material(material).z(z);
        if radius >= 0.0 {
            glass = glass.shape(Shape::Rounded(radius));
        }
        if interactive {
            glass = glass.interactive();
        }
        if lens {
            glass = glass.lens();
        }
        if let Some(&c) = usize::try_from(container).ok().and_then(|c| self.containers.get(c)) {
            glass = glass.container(c);
        }
        self.glass.push(self.scene.add(glass));
        self.glass.len() as u32 - 1
    }

    /// Morphs glass `id` to a new frame (CSS pixels).
    #[wasm_bindgen(js_name = setFrame)]
    pub fn set_frame(&mut self, id: u32, x: f32, y: f32, w: f32, h: f32) {
        if let Some(&id) = self.glass.get(id as usize) {
            self.scene.set_frame(id, Rect::new(x, y, w, h));
        }
    }

    /// Dematerializes glass `id`.
    pub fn remove(&mut self, id: u32) {
        if let Some(&id) = self.glass.get(id as usize) {
            self.scene.remove(id);
        }
    }

    /// Pointer pressed at CSS pixels; returns the glass handle or -1.
    #[wasm_bindgen(js_name = pointerDown)]
    pub fn pointer_down(&mut self, x: f32, y: f32) -> i32 {
        let hit = self.scene.pointer_down(Vec2::new(x, y));
        hit.and_then(|id| self.glass.iter().position(|&g| g == id)).map_or(-1, |i| i as i32)
    }

    /// Pointer moved (CSS pixels).
    #[wasm_bindgen(js_name = pointerMove)]
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.scene.pointer_move(Vec2::new(x, y));
    }

    /// Pointer released.
    #[wasm_bindgen(js_name = pointerUp)]
    pub fn pointer_up(&mut self) {
        self.scene.pointer_up();
    }

    /// Transparency slider (0 ultra clear .. 1 fully tinted), dark scheme, accessibility.
    #[wasm_bindgen(js_name = setAppearance)]
    pub fn set_appearance(&mut self, transparency: f32, dark: bool, reduce_transparency: bool, increase_contrast: bool, reduce_motion: bool) {
        let a = &mut self.scene.appearance;
        a.transparency = transparency;
        a.dark = dark;
        a.reduce_transparency = reduce_transparency;
        a.increase_contrast = increase_contrast;
        a.reduce_motion = reduce_motion;
    }

    /// Light direction in degrees (0 = from the top).
    #[wasm_bindgen(js_name = setLightAngle)]
    pub fn set_light_angle(&mut self, degrees: f32) {
        self.scene.set_light_angle(degrees);
    }

    /// Advances `dt` seconds and draws. Returns `true` while animating: keep calling
    /// `requestAnimationFrame` only while it does.
    pub fn frame(&mut self, dt: f32) -> bool {
        let animating = self.scene.update(dt);
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return animating,
        };
        let target = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.view_format), ..Default::default() });
        let content = self.content.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let frame = Frame {
            content: &content,
            target: &target,
            size: (self.config.width, self.config.height),
            scale: self.scale,
            content_changed: self.content_changed,
        };
        self.renderer.render(&self.device, &self.queue, &mut encoder, &frame, &self.scene);
        self.queue.submit([encoder.finish()]);
        self.queue.present(surface_texture);
        self.content_changed = false;
        animating
    }
}
