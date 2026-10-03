//! wgpu renderer: packs a [`Scene`] into GPU buffers and draws it over your content.
//!
//! Per frame:
//! 1. copy `content` into `target`;
//! 2. build (or reuse) a blur pyramid of the content, only inside the region glass
//!    can sample — Apple samples its backdrop at quarter resolution, we go further;
//! 3. draw every glass group as one instanced quad, z-layer by z-layer;
//! 4. when a higher layer overlaps lower glass, re-render the lower glass into an
//!    offscreen backdrop for just that region and rebuild the pyramid there, so glass
//!    on glass refracts what is really below it. Non-overlapping layers cost nothing.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec2;

use crate::scene::{Geometry, GlassId, Scene};

/// Inputs for one frame.
pub struct Frame<'a> {
    /// What is behind the glass. Needs `TEXTURE_BINDING`; same size as `target`.
    pub content: &'a wgpu::TextureView,
    /// Where content + glass are drawn. Must use the renderer's format. Must not be
    /// the same texture as `content`.
    pub target: &'a wgpu::TextureView,
    /// Size of both textures in pixels.
    pub size: (u32, u32),
    /// Pixels per point (device pixel ratio).
    pub scale: f32,
    /// Set when `content` differs from the previous frame. When `false` and the glass
    /// still covers the same region, the blur pyramid is reused (zero blur cost for
    /// static wallpapers).
    pub content_changed: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 4],
    misc: [f32; 4],
    light: [f32; 4],
    touch: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShapeGpu {
    center_half: [f32; 4],
    params: [f32; 4],
    stretch: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GroupGpu {
    bounds: [f32; 4],
    info: [u32; 4],
    geo: [f32; 4],
    optics: [f32; 4],
    lighting: [f32; 4],
    edge: [f32; 4],
    shadow: [f32; 4],
    light_a: [f32; 4],
    light_b: [f32; 4],
    dark_a: [f32; 4],
    dark_b: [f32; 4],
    tint: [f32; 4],
}

type Bounds = [f32; 4];

struct Layer {
    groups: Range<u32>,
    bounds: Bounds,
    z: i32,
}

#[derive(Default)]
struct Packed {
    shapes: Vec<ShapeGpu>,
    groups: Vec<GroupGpu>,
    layers: Vec<Layer>,
    /// Farthest any group samples beyond its bounds (refraction + blur), px.
    reach: f32,
}

struct Member {
    id: GlassId,
    geo: Geometry,
    z: i32,
    container: u64,
    order: u64,
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn expand(b: Bounds, by: f32) -> Bounds {
    [b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

fn intersects(a: Bounds, b: Bounds) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn contains(outer: Bounds, inner: Bounds) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Splits live glass into render groups: one per container cluster whose members are
/// within `spacing` of each other, one per loose glass.
fn cluster(scene: &Scene) -> Vec<Vec<Member>> {
    let mut members: Vec<Member> = scene
        .nodes
        .iter()
        .filter(|(_, n)| !n.glass.material.is_identity() && n.presence.value > 1e-3)
        .map(|(id, n)| Member {
            id,
            geo: scene.geometry(n),
            z: scene.z_of(n),
            container: n.glass.container.map_or(0, |c| slotmap::Key::data(&c).as_ffi()),
            order: n.order,
        })
        .collect();
    members.sort_by_key(|m| (m.z, m.container, m.order));

    let mut parent: Vec<usize> = (0..members.len()).collect();
    // ponytail: O(n^2) per container; fine for UI-sized clusters (tens of shapes).
    for (i, a) in members.iter().enumerate() {
        let Some(spacing) = scene.nodes[a.id].glass.container.and_then(|c| scene.containers.get(c)).map(|c| c.spacing) else {
            continue;
        };
        for (j, b) in members.iter().enumerate().skip(i + 1) {
            if b.container != a.container {
                break;
            }
            let gap = (a.geo.center - b.geo.center).abs() - (a.geo.reach() + b.geo.reach());
            if gap.max_element() < spacing {
                let (ra, rb) = (find(&mut parent, i), find(&mut parent, j));
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
    }

    let mut groups: Vec<Vec<Member>> = Vec::new();
    let mut slot = vec![usize::MAX; members.len()];
    for (i, m) in members.into_iter().enumerate() {
        let root = find(&mut parent, i);
        if slot[root] == usize::MAX {
            slot[root] = groups.len();
            groups.push(Vec::new());
        }
        groups[slot[root]].push(m);
    }
    groups.sort_by_key(|g| (g[0].z, g[0].order));
    groups
}

fn pack(scene: &Scene, scale: f32, viewport: Vec2, out: &mut Packed) {
    out.shapes.clear();
    out.groups.clear();
    out.layers.clear();
    out.reach = 0.0;
    let s = scale;
    let screen = [0.0, 0.0, viewport.x, viewport.y];

    for members in cluster(scene) {
        let lead = &scene.nodes[members[0].id];
        let spacing = lead.glass.container.and_then(|c| scene.containers.get(c)).map_or(0.0, |c| c.spacing);
        let presence = |m: &Member| scene.nodes[m.id].presence.value.clamp(0.0, 1.0);
        let p = members.iter().map(presence).fold(0.0, f32::max);
        let press = members.iter().map(|m| scene.nodes[m.id].press.value).fold(0.0, f32::max).clamp(0.0, 1.5);
        let min_side = members.iter().map(|m| m.geo.half.min_element() * 2.0).fold(0.0, f32::max);
        let resolved = lead.glass.material.resolve(min_side, &scene.appearance);
        let m = resolved.material;

        let first = out.shapes.len() as u32;
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let mut min_half = f32::MAX;
        for member in &members {
            let g = member.geo;
            let pm = presence(member);
            let others = members.iter().any(|o| o.id != member.id && presence(o) > 0.5);
            let shrink = (1.0 - pm) * g.half.min_element() * if others { 1.0 } else { 0.15 };
            let along = 1.0 + g.stretch;
            out.shapes.push(ShapeGpu {
                center_half: [g.center.x * s, g.center.y * s, g.half.x * s, g.half.y * s],
                params: [g.radius * s, shrink * s, g.axis.x, g.axis.y],
                stretch: [along, 1.0 / along, g.exponent, 0.0],
            });
            let c = g.center * s;
            let r = g.reach() * s;
            bounds = union(bounds, [c.x - r.x, c.y - r.y, c.x + r.x, c.y + r.y]);
            min_half = min_half.min(g.half.min_element() * s);
        }

        let k = spacing * s;
        let shadow_sigma = m.shadow.blur * 0.5 * s;
        let margin = (m.shadow.offset_y.abs() * s + 3.0 * shadow_sigma).max(2.0 * s) + k * 0.25 + 1.0;
        let bounds = clamp_bounds(expand(bounds, margin), screen);
        let depth = (m.depth * s).min(min_half).max(1.0);
        let frost_sigma = m.frost * 0.5 * s * p;
        out.reach = out.reach.max(depth * 1.2 * m.refraction + 8.0 * frost_sigma + 8.0);

        let tone_a = |t: crate::Tone| [t.lift * p, t.luminance, t.luminance_mix * p, t.lighten * p];
        let tone_b = |t: crate::Tone| [t.darken_to, t.darken * p, 0.0, 0.0];
        let tint = m.tint.map_or([0.0; 4], |c| {
            let lab = c.to_oklab();
            // Stained glass, never a solid fill: full alpha keeps 15 % of the backdrop.
            [lab[0], lab[1], lab[2], c.a * 0.85 * p]
        });
        out.groups.push(GroupGpu {
            bounds,
            info: [first, members.len() as u32, 0, 0],
            geo: [k, p, press, resolved.adapt],
            optics: [m.refraction * p, depth, m.dispersion * p, frost_sigma],
            lighting: [m.splay, m.specular * p * (1.0 + 0.6 * press), m.rim * p, m.rim_shade * p],
            edge: [m.hairline * p, m.edge * p, resolved.contrast * p, m.dim * p],
            shadow: [m.shadow.offset_y * s, shadow_sigma, m.shadow.opacity * p, 0.0],
            light_a: tone_a(m.light_tone),
            light_b: tone_b(m.light_tone),
            dark_a: tone_a(m.dark_tone),
            dark_b: tone_b(m.dark_tone),
            tint,
        });

        let index = out.groups.len() as u32 - 1;
        let z = members[0].z;
        match out.layers.last_mut() {
            Some(layer) if layer.z == z => {
                layer.groups.end = index + 1;
                layer.bounds = union(layer.bounds, bounds);
            }
            _ => out.layers.push(Layer { groups: index..index + 1, bounds, z }),
        }
    }
}

fn clamp_bounds(b: Bounds, screen: Bounds) -> Bounds {
    [b[0].max(screen[0]), b[1].max(screen[1]), b[2].min(screen[2]), b[3].min(screen[3])]
}

fn scissor(pass: &mut wgpu::RenderPass<'_>, b: Bounds, size: (u32, u32)) -> bool {
    let x0 = b[0].floor().max(0.0) as u32;
    let y0 = b[1].floor().max(0.0) as u32;
    let x1 = (b[2].ceil().max(0.0) as u32).min(size.0);
    let y1 = (b[3].ceil().max(0.0) as u32).min(size.1);
    if x1 <= x0 || y1 <= y0 {
        return false;
    }
    pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
    true
}

struct Pipelines {
    glass: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    down_first: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

struct Pyramid {
    size: (u32, u32),
    levels: u32,
    full: wgpu::TextureView,
    mips: Vec<wgpu::TextureView>,
    /// `chain[i]` reads mip `i` to produce mip `i + 1`.
    chain: Vec<wgpu::BindGroup>,
}

const PYRAMID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const MAX_LEVELS: u32 = 7;

impl Pyramid {
    fn new(device: &wgpu::Device, pipelines: &Pipelines, size: (u32, u32)) -> Self {
        let min_side = size.0.min(size.1).max(2);
        let levels = (min_side.ilog2().saturating_sub(3)).clamp(1, MAX_LEVELS);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("liquid-rust pyramid"),
            size: wgpu::Extent3d { width: size.0.div_ceil(2), height: size.1.div_ceil(2), depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PYRAMID_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mips: Vec<_> = (0..levels)
            .map(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let chain = mips[..mips.len() - 1]
            .iter()
            .map(|view| down_bind_group(device, &pipelines.down, &pipelines.sampler, view))
            .collect();
        Self { size, levels, full: texture.create_view(&wgpu::TextureViewDescriptor::default()), mips, chain }
    }

    /// Downsamples `src` into every level, only inside `region` (px of level 0).
    fn build(&self, device: &wgpu::Device, pipelines: &Pipelines, encoder: &mut wgpu::CommandEncoder, src: &wgpu::TextureView, region: Bounds) {
        let first = down_bind_group(device, &pipelines.down_first, &pipelines.sampler, src);
        for (i, mip) in self.mips.iter().enumerate() {
            let f = (1u32 << (i + 1)) as f32;
            let mip_size = ((self.size.0.div_ceil(2) >> i).max(1), (self.size.1.div_ceil(2) >> i).max(1));
            let b = expand([region[0] / f, region[1] / f, region[2] / f, region[3] / f], 2.0);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("liquid-rust downsample"),
                color_attachments: &[Some(attachment(mip))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !scissor(&mut pass, b, mip_size) {
                continue;
            }
            if i == 0 {
                pass.set_pipeline(&pipelines.down_first);
                pass.set_bind_group(0, &first, &[]);
            } else {
                pass.set_pipeline(&pipelines.down);
                pass.set_bind_group(0, &self.chain[i - 1], &[]);
            }
            pass.draw(0..3, 0..1);
        }
    }
}

fn attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
    }
}

fn down_bind_group(device: &wgpu::Device, pipeline: &wgpu::RenderPipeline, sampler: &wgpu::Sampler, src: &wgpu::TextureView) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("liquid-rust downsample"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
        ],
    })
}

fn blit(device: &wgpu::Device, pipelines: &Pipelines, encoder: &mut wgpu::CommandEncoder, src: &wgpu::TextureView, dst: &wgpu::TextureView, region: Option<Bounds>, size: (u32, u32)) {
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("liquid-rust blit"),
        layout: &pipelines.blit.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) }],
    });
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("liquid-rust blit"),
        color_attachments: &[Some(attachment(dst))],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some(region) = region
        && !scissor(&mut pass, region, size)
    {
        return;
    }
    pass.set_pipeline(&pipelines.blit);
    pass.set_bind_group(0, &group, &[]);
    pass.draw(0..3, 0..1);
}

/// Draws [`Scene`]s with Liquid Glass. Create one per target format and reuse it.
pub struct Renderer {
    format: wgpu::TextureFormat,
    pipelines: Pipelines,
    globals: wgpu::Buffer,
    shapes: wgpu::Buffer,
    groups: wgpu::Buffer,
    pyramid: Option<Pyramid>,
    layer_textures: Vec<wgpu::TextureView>,
    /// Region of the pyramid that currently holds a valid blur of the content.
    cached: Option<Bounds>,
    packed: Packed,
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn pipeline(device: &wgpu::Device, module: &wgpu::ShaderModule, vs: &str, fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, topology: wgpu::PrimitiveTopology) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fs),
        layout: None,
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState { topology, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Renderer {
    /// Creates pipelines for drawing into textures of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let glass = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("liquid-rust glass"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/glass.wgsl").into()),
        });
        let pyramid = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("liquid-rust pyramid"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/pyramid.wgsl").into()),
        });
        let strip = wgpu::PrimitiveTopology::TriangleStrip;
        let list = wgpu::PrimitiveTopology::TriangleList;
        let pipelines = Pipelines {
            glass: pipeline(device, &glass, "vs_glass", "fs_glass", format, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), strip),
            blit: pipeline(device, &pyramid, "vs_full", "fs_blit", format, None, list),
            down_first: pipeline(device, &pyramid, "vs_full", "fs_down_first", PYRAMID_FORMAT, None, list),
            down: pipeline(device, &pyramid, "vs_full", "fs_down", PYRAMID_FORMAT, None, list),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("liquid-rust linear clamp"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        };
        Self {
            format,
            globals: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("liquid-rust globals"),
                size: size_of::<Globals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            shapes: storage(device, "liquid-rust shapes", size_of::<ShapeGpu>() as u64 * 16),
            groups: storage(device, "liquid-rust groups", size_of::<GroupGpu>() as u64 * 16),
            pipelines,
            pyramid: None,
            layer_textures: Vec::new(),
            cached: None,
            packed: Packed::default(),
        }
    }

    /// Texture format this renderer draws into.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Records commands that draw `frame.content` plus all glass of `scene` into
    /// `frame.target`.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &Frame<'_>, scene: &Scene) {
        let size = frame.size;
        pack(scene, frame.scale, Vec2::new(size.0 as f32, size.1 as f32), &mut self.packed);
        if self.pyramid.as_ref().is_none_or(|p| p.size != size) {
            self.pyramid = Some(Pyramid::new(device, &self.pipelines, size));
            self.layer_textures.clear();
            self.cached = None;
        }
        self.upload(device, queue, frame, scene);
        blit(device, &self.pipelines, encoder, frame.content, frame.target, None, size);
        if self.packed.layers.is_empty() {
            return;
        }

        // Which layers need a backdrop that includes the glass below them.
        let screen = [0.0, 0.0, size.0 as f32, size.1 as f32];
        let reach = self.packed.reach;
        let layers = &self.packed.layers;
        let tail = |i: usize| clamp_bounds(expand(layers[i..].iter().fold(layers[i].bounds, |a, l| union(a, l.bounds)), reach), screen);
        let mut rebuild = vec![false; layers.len()];
        let mut below = layers[0].bounds;
        for (i, layer) in layers.iter().enumerate().skip(1) {
            rebuild[i] = intersects(expand(layer.bounds, reach), below);
            below = union(below, layer.bounds);
        }
        if rebuild.iter().any(|&r| r) && self.layer_textures.is_empty() {
            self.layer_textures = (0..2).map(|_| layer_texture(device, self.format, size)).collect();
        }

        let Some(pyramid) = &self.pyramid else { return };
        let region = tail(0);
        let reuse = !frame.content_changed && self.cached.is_some_and(|c| contains(c, region));
        if !reuse {
            pyramid.build(device, &self.pipelines, encoder, frame.content, region);
            self.cached = Some(region);
        }

        let mut src = frame.content;
        for (i, layer) in layers.iter().enumerate() {
            if rebuild[i] {
                let region = tail(i);
                let dst = &self.layer_textures[i % 2];
                blit(device, &self.pipelines, encoder, src, dst, Some(region), size);
                self.draw_layer(device, encoder, &layers[i - 1], src, dst, Some(region), size);
                pyramid.build(device, &self.pipelines, encoder, dst, region);
                self.cached = None;
                src = dst;
            }
            self.draw_layer(device, encoder, layer, src, frame.target, None, size);
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame<'_>, scene: &Scene) {
        let (w, h) = (frame.size.0 as f32, frame.size.1 as f32);
        let angle = scene.light_angle.value.to_radians();
        let touch = &scene.touch;
        let globals = Globals {
            viewport: [w, h, 1.0 / w, 1.0 / h],
            misc: [
                frame.scale,
                self.pyramid.as_ref().map_or(1, |p| p.levels) as f32,
                if scene.appearance.dark { 0.0 } else { 1.0 },
                0.0,
            ],
            light: [angle.sin(), -angle.cos(), 0.0, 0.0],
            touch: [
                touch.pos.x * frame.scale,
                touch.pos.y * frame.scale,
                touch.radius.value.max(0.0) * frame.scale,
                touch.intensity.value.max(0.0),
            ],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        write_storage(device, queue, &mut self.shapes, "liquid-rust shapes", &self.packed.shapes);
        write_storage(device, queue, &mut self.groups, "liquid-rust groups", &self.packed.groups);
    }

    #[expect(clippy::too_many_arguments, reason = "plain pass recording, no state to bundle")]
    fn draw_layer(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, layer: &Layer, src: &wgpu::TextureView, dst: &wgpu::TextureView, region: Option<Bounds>, size: (u32, u32)) {
        let Some(pyramid) = &self.pyramid else { return };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("liquid-rust glass"),
            layout: &self.pipelines.glass.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.shapes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.groups.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.pipelines.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&pyramid.full) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("liquid-rust glass"),
            color_attachments: &[Some(attachment(dst))],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(region) = region
            && !scissor(&mut pass, region, size)
        {
            return;
        }
        pass.set_pipeline(&self.pipelines.glass);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..4, layer.groups.clone());
    }
}

fn layer_texture(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32)) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("liquid-rust layer backdrop"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn write_storage<T: Pod>(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &mut wgpu::Buffer, label: &str, data: &[T]) {
    let bytes: &[u8] = bytemuck::cast_slice(data);
    if bytes.len() as u64 > buffer.size() {
        *buffer = storage(device, label, (bytes.len() as u64).next_power_of_two());
    }
    if !bytes.is_empty() {
        queue.write_buffer(buffer, 0, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Glass, Rect};

    fn groups_for_gap(gap: f32) -> usize {
        let mut scene = Scene::new();
        let c = scene.add_container(20.0, 0);
        scene.add(Glass::new(Rect::new(0.0, 0.0, 50.0, 50.0)).container(c));
        scene.add(Glass::new(Rect::new(50.0 + gap, 0.0, 50.0, 50.0)).container(c));
        while scene.update(1.0 / 60.0) {}
        cluster(&scene).len()
    }

    #[test]
    fn glass_within_spacing_shares_one_group() {
        assert_eq!(groups_for_gap(10.0), 1);
    }

    #[test]
    fn glass_beyond_spacing_renders_separately() {
        assert_eq!(groups_for_gap(30.0), 2);
    }

    #[test]
    fn higher_z_starts_a_new_layer() {
        let mut scene = Scene::new();
        scene.add(Glass::new(Rect::new(0.0, 0.0, 50.0, 50.0)));
        scene.add(Glass::new(Rect::new(10.0, 10.0, 50.0, 50.0)).z(1));
        while scene.update(1.0 / 60.0) {}
        let mut packed = Packed::default();
        pack(&scene, 2.0, Vec2::new(200.0, 200.0), &mut packed);
        assert_eq!(packed.layers.len(), 2);
    }
}
