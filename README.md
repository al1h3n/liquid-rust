https://github.com/user-attachments/assets/4a06584f-4293-4b74-a245-508b1248128f

**[Try the live demo](https://al1h3n.github.io/liquid-rust-showcase/)** (WebGPU) · [showcase source](https://github.com/al1h3n/liquid-rust-showcase)

# Liquid Rust

Apple-style **Liquid Glass** for [`wgpu`] 30, on native GPUs (Vulkan, Metal, DX12, GL) and in
the browser through WebGPU. It's a library that works in any project: you give it a texture of your content and a list
of glass shapes, and it draws the glass on top.

This is not glassmorphism. The look is calibrated against Apple's own iOS 26/27 Figma kit, and the
lock-screen button and the dock match the kit's render pixel profile for pixel profile:

- **Lensing.** The backdrop is refracted through a squircle bevel (Snell's law, η 1.5) with
  per-channel dispersion. Most of the effect comes from bending light, not from blur.
- **Frost.** A single blur pyramid of the content serves every blur radius on screen. B-spline
  sampling lets each glass use any radius in between levels.
- **Kit tone mapping.** Figma's `LINEAR_DODGE` / `LUMINOSITY` / `LIGHTEN` / `DARKEN` fills are
  reproduced exactly, in sRGB-encoded space.
- **Light.** A crisp lit rim line that takes its colour from what is behind it. Inner rim
  shadows match the kit's `#282828` dodge and `#E6E6E6` burn. iOS 27's darkened hairline edge.
- **Adaptive.** Small regular glass flips between light and dark with the content behind it.
  Large glass gets thicker: more frost, a deeper shadow, a more opaque face. The shadow
  strengthens over busy content and fades over flat light content.
- **Liquid.** Glass in one container is drawn as a single smooth-union distance field, so
  shapes grow a liquid neck and fuse as they approach, then split apart. This comes from the
  geometry, not from collision physics.
- **Motion.** Analytic springs using Apple's `duration` / `bounce` model, so they behave the same
  at any frame rate and can be interrupted mid-flight. Includes press glow from the finger, a
  draggable lens that stretches with speed, materialize in and out, and morphing between frames.
- **Presets.** `regular`, `clear`, `tinted`, `identity`, plus `clear_bar` for dock and search
  chrome. The iOS 27 transparency slider and Reduce Transparency, Increase Contrast and Reduce
  Motion are all supported.
- **Squircles.** `Shape::Rounded` uses Apple's continuous corners: Figma smoothing 60 %, which
  is what every glass element in the kit uses. They are drawn as superellipse corners fitted
  to Figma's geometry within 0.74 % of the radius. `Shape::Circular` gives quarter circles, and
  `Shape::Smooth { radius, smoothing }` takes any Figma smoothing value.
- **Glass on glass.** A higher z-layer refracts the glass that is really under it. The backdrop
  is rebuilt only where layers overlap, and non-overlapping layers cost nothing.

```rust
use liquid_rust::{Color, Frame, Glass, Material, Rect, Renderer, Scene, Shape};

let mut scene = Scene::new();
let toolbar = scene.add_container(14.0, 0);                 // GlassEffectContainer(spacing: 14)
for i in 0..3 {
    scene.add(Glass::new(Rect::new(24.0 + 60.0 * i as f32, 24.0, 52.0, 52.0)).interactive().container(toolbar));
}
scene.add(Glass::new(Rect::new(300.0, 24.0, 110.0, 52.0)).material(Material::tinted(Color::BLUE)).interactive());
scene.add(Glass::new(Rect::new(17.0, 754.0, 368.0, 103.0)).shape(Shape::Rounded(38.0)).material(Material::clear_bar()));

let mut renderer = Renderer::new(&device, surface_format);   // once
// every frame:
let animating = scene.update(dt);                            // false => stop requesting frames
renderer.render(&device, &queue, &mut encoder, &Frame {
    content: &content_view,   // your UI/content, TEXTURE_BINDING, sRGB or float format
    target: &target_view,     // same size; sRGB (or float) view in the renderer's format
    size: (width, height),
    scale: scale_factor,      // pixels per point
    content_changed: true,    // false => blur pyramid reused (static wallpapers cost ~0)
}, &scene);
```

Input: `scene.pointer_down(p)`, `pointer_move(p)`, `pointer_up()` (points).
Animate layout with `scene.set_frame(id, rect)`. Use `scene.remove(id)` to dematerialize glass.

### Merging, physics and morphs

- **Merge distance.** The first argument of `add_container(spacing, z)` is the distance in points
  at which shapes start to bend toward each other. Below half of it they join with a liquid
  neck. Change it later with `scene.set_container_spacing(id, pt)`. Use `0`, or leave the glass
  out of any container, for no merging at all. Rest shapes either under half the spacing
  (joined) or at least the full spacing apart (clean). In between they stay bent toward each
  other without touching, and a neck that breaks just as they stop looks like a glitch,
  especially with Reduce Motion, which has no overshoot to carry them past it.
- **Physics off.** `scene.physics = false` makes every change land on the next `update`: no
  springs, press growth, lens lift or stretch. Use it for static UI, tests or layout work.
  `appearance.reduce_motion` is the gentler option: it keeps the motion and drops the bounce.
- **Grow one glass out of another.** For example, a search bar opening into its results:

```rust
let search = scene.add_container(16.0, 1);
let bar = scene.add(Glass::new(Rect::new(40.0, 24.0, 320.0, 52.0)).interactive().container(search));
// on click:
let results = scene.expand_from(bar, Glass::new(Rect::new(40.0, 96.0, 320.0, 400.0)).shape(Shape::Rounded(28.0)), Spring::new(0.5, 0.2));
// to close:
scene.collapse_into(results, bar, Spring::new(0.35, 0.0));
```

  The new glass starts at the bar's current frame, fused with it. It stretches a liquid neck
  while it grows and pinches off once the gap passes half the spacing. Here the gap ends at
  20 pt, past the 16 pt spacing, so both shapes settle clean. For the two to stay joined, end
  the gap under half the spacing instead. The outline morphs as well: glass grown out of a
  round button starts round and eases into its own corners, and collapses back into a circle.

## Contract

- `content` and `target` are different textures of the same size. `content` must be
  readable as linear colour, so use an sRGB view such as `Rgba8UnormSrgb` / `Bgra8UnormSrgb`, or a
  float format. On the web, configure the canvas with `viewFormats: ["bgra8unorm-srgb"]` and
  render through that view (see `web/`).
- Draw your glyphs and labels on top after `render`. Use vibrancy: additive / plus-lighter
  for light text, multiply / plus-darker for dark text. Don't put glass on glass for that.
- The scene is in points, top-left origin. `Frame::scale` converts points to pixels.

## Presets

| Constructor | Use | Source |
|---|---|---|
| `Material::regular()` | bars, buttons, menus, anything with text | Apple kit Quick Actions menu + compositor defaults |
| `Material::clear()` | controls over photos, video, lock screen | Apple kit "Clear Glass" (refraction 70, depth 30, dispersion 20, frost 6, splay 20, light 40 %) |
| `Material::clear_bar()` | dock / search chrome over media | Apple kit Dock (`#999` luminosity 33 %, frost 3, hairlines) |
| `Material::tinted(color)` | the **one** primary action | `Glass.regular.tint()`: stained glass, never a solid fill |
| `Material::identity()` | transition endpoint | `Glass.identity` |

Every field maps to a knob in Figma's GLASS panel (`refraction`, `depth`, `dispersion`,
`frost`, `splay`, `specular`) or to an Apple kit fill/shadow. You can copy values straight from a design.

## Examples

```bash
cargo run -r --example demo -- path/to/wallpaper.jpg
```

In the demo, drag the lens: it fuses with the blobs. Click `…` or press Space to morph a button into a
menu. Click the search bar or press `F` to open its results, and `Esc` to close them. `P` turns
physics on and off, and `[` / `]` change the blobs' merge distance. Other keys: `1` `2` `3` for the transparency slider, `D` for the dark scheme, `R` / `C` / `M` for
the accessibility settings, `L` to sweep the light, `S` to scroll content under the glass.

```bash
cargo run -r --example snapshot -- wallpaper.png out.png home
```

The snapshot example renders headless. Modes: `lock`, `home` (Apple kit positions),
`corners` (circular vs iOS vs 100 % smoothing), `showcase`, `press`, `search` (results panel
mid-way out of a search bar), `bench`.

### Web (WebGPU)

```bash
cd web
wasm-pack build --target web --release
python -m http.server
```

Then open `http://localhost:8000/index.html?bg=your-image.png`. `web/src/lib.rs` is a thin
`wasm-bindgen` wrapper with the methods `create`, `add`, `addContainer`, `setContainerSpacing`,
`setFrame`, `remove`, `expandFrom`, `collapseInto`, `setPhysics`, `pointerDown`/`Move`/`Up`,
`setAppearance`, `setLightAngle` and `frame(dt) -> animating`.

## Performance

The `bench` snapshot draws 8 glass groups with liquid merging and a glass-on-glass layer at
1206×2622 (iPhone 3x) in **0.6–0.7 ms per frame** on an RTX 30-series GPU. Where the cost goes:

- One full-screen copy of the content.
- A blur pyramid limited to the region the glass can actually sample. It is skipped entirely
  when `content_changed` is false.
- One instanced quad per glass group. Every distance-field evaluation stays inside that quad,
  and the inner rim shadows are skipped deep inside large glass.
- CPU work per frame is O(n) per container plus a buffer upload. Clustering is O(n²) within a
  container, which is fine for tens of shapes.
- `Scene::update` returns `false` when everything is at rest, so the host can stop rendering.
- Glass on glass costs extra only where a higher layer overlaps lower glass.

## Tests

```bash
cargo test
```

Unit tests cover springs, materials, scene interaction and clustering. GPU tests render a frame
and check the compositing contract, and they skip themselves when there is no GPU adapter.

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

[`wgpu`]: https://wgpu.rs

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
