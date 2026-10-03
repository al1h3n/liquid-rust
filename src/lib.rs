//! # Liquid Rust
//!
//! Apple-style Liquid Glass for [`wgpu`], on native GPUs and on the web through WebGPU.
//!
//! Not glassmorphism: the backdrop is *refracted* through a squircle bevel (Snell's law,
//! per-channel dispersion), lightly frosted, tone-mapped like Apple's kit, lit by a
//! directional rim light, and outlined by a darkened hairline. Glass in one
//! [`Container`] fuses like liquid through a smooth signed-distance union, and every
//! motion is an analytic spring with Apple's `duration`/`bounce` model.
//!
//! ```no_run
//! # fn demo(device: &wgpu::Device, queue: &wgpu::Queue, content: &wgpu::TextureView, target: &wgpu::TextureView) {
//! use liquid_rust::{Frame, Glass, Material, Rect, Renderer, Scene};
//!
//! let mut scene = Scene::new();
//! let bar = scene.add_container(16.0, 0);
//! scene.add(Glass::new(Rect::new(24.0, 700.0, 58.0, 58.0)).material(Material::clear()).interactive().container(bar));
//!
//! let mut renderer = Renderer::new(device, wgpu::TextureFormat::Bgra8UnormSrgb);
//! let animating = scene.update(1.0 / 120.0);
//! let mut encoder = device.create_command_encoder(&Default::default());
//! renderer.render(device, queue, &mut encoder, &Frame {
//!     content, target, size: (1206, 2622), scale: 3.0, content_changed: false,
//! }, &scene);
//! queue.submit([encoder.finish()]);
//! # let _ = animating;
//! # }
//! ```

mod material;
mod motion;
mod renderer;
mod scene;

pub use glam::Vec2;
pub use material::{Appearance, Color, Material, Shadow, Tone};
pub use motion::{Animated, Spring};
pub use renderer::{Frame, Renderer};
pub use scene::{Container, ContainerId, Glass, GlassId, IOS_SMOOTHING, Rect, Scene, Shape, smooth_corner};
