//! Scene graph of glass elements: geometry, containers, interaction and animation.
//!
//! Units are points; the renderer multiplies by the frame's scale factor.

use glam::Vec2;
use slotmap::{SlotMap, new_key_type};

use crate::material::{Appearance, Material};
use crate::motion::{Animated, Spring};

new_key_type! {
    /// Handle to a glass element in a [`Scene`].
    pub struct GlassId;
    /// Handle to a [`Container`] in a [`Scene`].
    pub struct ContainerId;
}

/// Axis-aligned rectangle in points (top-left origin, y down).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Rect {
    /// Creates a rectangle.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    /// Rectangle of `size` centred on `center`.
    pub fn centered(center: Vec2, size: Vec2) -> Self {
        Self::new(center.x - size.x * 0.5, center.y - size.y * 0.5, size.x, size.y)
    }

    /// Centre point.
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }
}

/// Outline of a glass element.
///
/// Corners follow Apple: `Rounded` is a *continuous* (squircle) corner like SwiftUI's
/// `.rect(cornerRadius:)` / `RoundedCornerStyle.continuous`, which is what every glass
/// element in Apple's iOS 26/27 kit uses (Figma corner smoothing 60 %).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Fully rounded ends; a circle when square. Apple's default for glass.
    Capsule,
    /// Continuous-curvature (squircle) corners with this radius in points: Figma corner
    /// smoothing 60 %, the iOS standard.
    Rounded(f32),
    /// Quarter-circle corners (`RoundedCornerStyle.circular`, CSS `border-radius`).
    Circular(f32),
    /// Figma-style corners: `radius` in points and `smoothing` in `0..=1`
    /// (0 = circular, 0.6 = iOS).
    Smooth {
        /// Corner radius in points.
        radius: f32,
        /// Figma corner smoothing.
        smoothing: f32,
    },
}

/// iOS corner smoothing (Figma 60 %).
pub const IOS_SMOOTHING: f32 = 0.6;

/// Maps a Figma smoothed corner to a superellipse corner: returns the corner's extent
/// along each edge and its exponent. Figma's geometry (arc + two Béziers) is matched to
/// within 0.74 % of the radius for any smoothing; like Figma, smoothing shrinks when the
/// corner would not fit in `budget` (half the shorter side).
pub fn smooth_corner(radius: f32, smoothing: f32, budget: f32) -> (f32, f32) {
    let r = radius.min(budget).max(0.0);
    if r <= 0.0 {
        return (0.0, 2.0);
    }
    let s = smoothing.clamp(0.0, 1.0).min((budget / r - 1.0).max(0.0));
    (r * (1.0 + 0.42 * s + 0.18 * s * s), 2.0 + 0.95 * s + 0.6 * s * s)
}

/// Groups glass that renders together and liquid-merges, like `GlassEffectContainer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Container {
    /// Distance in points at which member shapes start to bend toward each other. They
    /// join with a liquid neck below half of it. Rest them either under half (joined) or
    /// at least this far apart (clean): in between they stay bent without touching, and a
    /// neck that breaks just before they stop looks like a glitch.
    pub spacing: f32,
    /// Stacking order. Higher containers sample the rendered glass below them.
    pub z: i32,
}

/// Description of one glass element.
///
/// ```
/// use liquid_rust::{Glass, Material, Rect, Shape};
/// let button = Glass::new(Rect::new(20.0, 40.0, 58.0, 58.0))
///     .material(Material::clear())
///     .interactive();
/// assert_eq!(button.shape, Shape::Capsule);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    /// Target frame in points.
    pub frame: Rect,
    /// Outline.
    pub shape: Shape,
    /// Optical material.
    pub material: Material,
    /// Reacts to presses: grows, lights up from the touch point, bounces back.
    pub interactive: bool,
    /// Draggable liquid lens: follows the pointer, lifts and stretches with velocity.
    pub lens: bool,
    /// Container this glass merges within.
    pub container: Option<ContainerId>,
    /// Stacking order when not in a container.
    pub z: i32,
}

impl Glass {
    /// Regular capsule glass at `frame`.
    pub fn new(frame: Rect) -> Self {
        Self {
            frame,
            shape: Shape::Capsule,
            material: Material::regular(),
            interactive: false,
            lens: false,
            container: None,
            z: 0,
        }
    }

    /// Sets the outline.
    pub fn shape(mut self, shape: Shape) -> Self {
        self.shape = shape;
        self
    }

    /// Sets the material.
    pub fn material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    /// Enables press feedback (`.interactive()`).
    pub fn interactive(mut self) -> Self {
        self.interactive = true;
        self
    }

    /// Makes the glass a draggable liquid lens.
    pub fn lens(mut self) -> Self {
        self.lens = true;
        self
    }

    /// Places the glass in a container.
    pub fn container(mut self, container: ContainerId) -> Self {
        self.container = Some(container);
        self
    }

    /// Sets the stacking order (ignored inside a container).
    pub fn z(mut self, z: i32) -> Self {
        self.z = z;
        self
    }
}

pub(crate) struct Node {
    pub glass: Glass,
    pub x: Animated,
    pub y: Animated,
    pub w: Animated,
    pub h: Animated,
    pub presence: Animated,
    pub press: Animated,
    pub lift: Animated,
    pub stretch: Animated,
    pub stretch_dir: Vec2,
    pub removing: bool,
    pub order: u64,
}

/// Current, animated outline of a node in points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    pub center: Vec2,
    pub half: Vec2,
    /// Corner extent (points) and superellipse exponent (2 = circular).
    pub radius: f32,
    pub exponent: f32,
    pub axis: Vec2,
    pub stretch: f32,
}

impl Geometry {
    /// Conservative half-extent including stretch.
    pub fn reach(&self) -> Vec2 {
        self.half * (1.0 + self.stretch)
    }

    fn contains(&self, p: Vec2) -> bool {
        let q = (p - self.center).abs() - self.half + Vec2::splat(self.radius);
        let c = q.max(Vec2::ZERO);
        let n = self.exponent;
        (c.x.powf(n) + c.y.powf(n)).powf(1.0 / n) + q.x.max(q.y).min(0.0) - self.radius <= 0.0
    }
}

pub(crate) struct Touch {
    pub pos: Vec2,
    pub intensity: Animated,
    pub radius: Animated,
}

struct Drag {
    id: GlassId,
    grab: Vec2,
    pointer: Vec2,
    moved: Vec2,
    velocity: Vec2,
}

const MATERIALIZE_IN: Spring = Spring::new(0.4, 0.0);
const MATERIALIZE_OUT: Spring = Spring::new(0.3, 0.0);
const PRESS_DOWN: Spring = Spring::new(0.25, 0.0);
const PRESS_RELEASE: Spring = Spring::new(0.5, 0.35);
const LIFT: Spring = Spring::new(0.3, 0.15);
const JELLY: Spring = Spring::new(0.35, 0.4);
const GLOW_IN: Spring = Spring::new(0.2, 0.0);
const GLOW_SPREAD: Spring = Spring::new(0.45, 0.0);
const GLOW_OUT: Spring = Spring::new(0.6, 0.0);
const LIGHT_SWEEP: Spring = Spring::new(0.8, 0.0);
/// Drag speed (pt/s) that produces the full stretch.
const STRETCH_SPEED: f32 = 2500.0;
const MAX_STRETCH: f32 = 0.25;
const LENS_LIFT: f32 = 0.3;

/// All glass on screen plus the shared appearance, light and touch state.
///
/// ```
/// use liquid_rust::{Glass, Rect, Scene};
/// let mut scene = Scene::new();
/// let id = scene.add(Glass::new(Rect::new(0.0, 0.0, 120.0, 44.0)));
/// while scene.update(1.0 / 120.0) {} // materializes, then rests
/// assert!(scene.glass(id).is_some());
/// ```
pub struct Scene {
    pub(crate) nodes: SlotMap<GlassId, Node>,
    pub(crate) containers: SlotMap<ContainerId, Container>,
    /// Colour scheme, transparency slider and accessibility settings.
    pub appearance: Appearance,
    /// `false` turns every spring off: frames, materialize, press and lens motion land
    /// instantly, and press growth, lens lift and stretch are skipped. Liquid merging is
    /// geometry, not physics; control it with container spacing.
    pub physics: bool,
    pub(crate) light_angle: Animated,
    pub(crate) touch: Touch,
    pressed: Option<GlassId>,
    drag: Option<Drag>,
    next_order: u64,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    /// Empty scene, light from the top.
    pub fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
            containers: SlotMap::with_key(),
            appearance: Appearance::default(),
            physics: true,
            light_angle: Animated::new(0.0, LIGHT_SWEEP),
            touch: Touch {
                pos: Vec2::ZERO,
                intensity: Animated::new(0.0, GLOW_OUT),
                radius: Animated::new(0.0, GLOW_SPREAD),
            },
            pressed: None,
            drag: None,
            next_order: 0,
        }
    }

    fn spring(&self, spring: Spring) -> Spring {
        if self.appearance.reduce_motion { spring.without_bounce() } else { spring }
    }

    /// No decorative motion: Reduce Motion or physics off.
    fn still(&self) -> bool {
        self.appearance.reduce_motion || !self.physics
    }

    /// Adds a container (`GlassEffectContainer(spacing:)`).
    pub fn add_container(&mut self, spacing: f32, z: i32) -> ContainerId {
        self.containers.insert(Container { spacing, z })
    }

    /// Changes a container's [`Container::spacing`] (points). `0` turns liquid merging
    /// off for that container.
    pub fn set_container_spacing(&mut self, id: ContainerId, spacing: f32) {
        if let Some(container) = self.containers.get_mut(id) {
            container.spacing = spacing.max(0.0);
        }
    }

    /// Adds glass; it materializes by ramping its lensing in.
    pub fn add(&mut self, glass: Glass) -> GlassId {
        let at = |v| Animated::new(v, Spring::SNAPPY);
        let mut presence = Animated::new(0.0, MATERIALIZE_IN);
        presence.animate_to(1.0, self.spring(MATERIALIZE_IN));
        let f = glass.frame;
        self.next_order += 1;
        self.nodes.insert(Node {
            glass,
            x: at(f.x),
            y: at(f.y),
            w: at(f.width),
            h: at(f.height),
            presence,
            press: at(0.0),
            lift: at(0.0),
            stretch: at(0.0),
            stretch_dir: Vec2::X,
            removing: false,
            order: self.next_order,
        })
    }

    /// Dematerializes the glass, then drops it.
    pub fn remove(&mut self, id: GlassId) {
        let spring = self.spring(MATERIALIZE_OUT);
        if let Some(node) = self.nodes.get_mut(id) {
            node.removing = true;
            node.presence.animate_to(0.0, spring);
        }
    }

    /// Description of a glass element (its target frame, not the animated one).
    pub fn glass(&self, id: GlassId) -> Option<&Glass> {
        self.nodes.get(id).map(|n| &n.glass)
    }

    /// Frame on screen right now, mid-animation (without press or lens scaling).
    pub fn current_frame(&self, id: GlassId) -> Option<Rect> {
        self.nodes.get(id).map(|n| Rect::new(n.x.value, n.y.value, n.w.value, n.h.value))
    }

    /// Grows new glass out of `source`: it starts at `source`'s current frame and springs
    /// to `glass.frame`, like a search bar opening into its results. Without a container
    /// of its own, `glass` joins `source`'s; sharing a container, it emerges fused to
    /// `source`, stretches a liquid neck and pinches off once the gap passes half the
    /// container's spacing; end at least the full spacing away for clean shapes (see
    /// [`Container::spacing`]). Outside a container it materializes instead.
    pub fn expand_from(&mut self, source: GlassId, mut glass: Glass, spring: Spring) -> GlassId {
        let Some(start) = self.current_frame(source) else { return self.add(glass) };
        glass.container = glass.container.or(self.nodes[source].glass.container);
        let fused = self.fuses(source, glass.container);
        let target = glass.frame;
        let id = self.add(Glass { frame: start, ..glass });
        if fused {
            self.nodes[id].presence.snap(1.0);
        }
        self.set_frame_with(id, target, spring);
        id
    }

    /// Reverse of [`Scene::expand_from`]: springs `id` back into `target`'s frame. Fused
    /// glass is dropped once it has merged back in; otherwise it dematerializes.
    pub fn collapse_into(&mut self, id: GlassId, target: GlassId, spring: Spring) {
        let Some(frame) = self.glass(target).map(|g| g.frame) else { return self.remove(id) };
        self.set_frame_with(id, frame, spring);
        let container = self.glass(id).and_then(|g| g.container);
        if self.fuses(target, container) {
            self.nodes[id].removing = true;
        } else {
            self.remove(id);
        }
    }

    /// `true` when glass in `container` liquid-merges with `other`.
    fn fuses(&self, other: GlassId, container: Option<ContainerId>) -> bool {
        container.is_some_and(|c| self.glass(other).is_some_and(|g| g.container == Some(c)) && self.containers[c].spacing > 0.0)
    }

    /// Moves/resizes with the default morph spring (`.snappy`).
    pub fn set_frame(&mut self, id: GlassId, frame: Rect) {
        self.set_frame_with(id, frame, Spring::SNAPPY);
    }

    /// Moves/resizes with a custom spring. Shapes in a container fuse and split on
    /// their own as they pass within `spacing` of each other.
    pub fn set_frame_with(&mut self, id: GlassId, frame: Rect, spring: Spring) {
        let spring = self.spring(spring);
        let Some(node) = self.nodes.get_mut(id) else { return };
        node.glass.frame = frame;
        node.x.animate_to(frame.x, spring);
        node.y.animate_to(frame.y, spring);
        node.w.animate_to(frame.width, spring);
        node.h.animate_to(frame.height, spring);
    }

    /// Replaces the material (instantly).
    pub fn set_material(&mut self, id: GlassId, material: Material) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.glass.material = material;
        }
    }

    /// Replaces the outline.
    pub fn set_shape(&mut self, id: GlassId, shape: Shape) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.glass.shape = shape;
        }
    }

    /// Points the specular light: degrees, 0 = from the top, clockwise. Drive it from
    /// device motion or sweep it on focus changes.
    pub fn set_light_angle(&mut self, degrees: f32) {
        let spring = self.spring(LIGHT_SWEEP);
        self.light_angle.animate_to(degrees, spring);
    }

    /// Topmost interactive or lens glass under `p`.
    pub fn hit_test(&self, p: Vec2) -> Option<GlassId> {
        self.nodes
            .iter()
            .filter(|(_, n)| !n.removing && (n.glass.interactive || n.glass.lens))
            .filter(|(_, n)| self.geometry(n).contains(p))
            .max_by_key(|(_, n)| (self.z_of(n), n.order))
            .map(|(id, _)| id)
    }

    /// Press at `p` (points). Returns the glass that took the press.
    pub fn pointer_down(&mut self, p: Vec2) -> Option<GlassId> {
        let id = self.hit_test(p)?;
        let still = self.still();
        let lift = self.spring(LIFT);
        let node = &mut self.nodes[id];
        let size = Vec2::new(node.w.target, node.h.target);
        if node.glass.interactive {
            node.press.animate_to(1.0, PRESS_DOWN);
        }
        if node.glass.lens {
            if !still {
                node.lift.animate_to(1.0, lift);
            }
            let center = Vec2::new(node.x.target, node.y.target) + size * 0.5;
            self.drag = Some(Drag { id, grab: center - p, pointer: p, moved: Vec2::ZERO, velocity: Vec2::ZERO });
        }
        self.pressed = Some(id);
        self.touch.pos = p;
        self.touch.intensity.animate_to(1.0, GLOW_IN);
        self.touch.radius.snap(0.0);
        self.touch.radius.animate_to(size.max_element() * 1.2, GLOW_SPREAD);
        Some(id)
    }

    /// Pointer moved to `p` (points).
    pub fn pointer_move(&mut self, p: Vec2) {
        if self.pressed.is_some() {
            self.touch.pos = p;
        }
        let Some(drag) = &mut self.drag else { return };
        drag.moved += p - drag.pointer;
        drag.pointer = p;
        let center = p + drag.grab;
        let id = drag.id;
        if let Some(node) = self.nodes.get_mut(id) {
            let size = Vec2::new(node.w.target, node.h.target);
            node.glass.frame = Rect::centered(center, size);
            node.x.animate_to(center.x - size.x * 0.5, Spring::INTERACTIVE);
            node.y.animate_to(center.y - size.y * 0.5, Spring::INTERACTIVE);
        }
    }

    /// Releases the press: bounces back, the glow fades, a lens drops and jiggles.
    pub fn pointer_up(&mut self) {
        let release = self.spring(PRESS_RELEASE);
        let lift = self.spring(Spring::SNAPPY);
        let jelly = self.spring(JELLY);
        if let Some(node) = self.pressed.take().and_then(|id| self.nodes.get_mut(id)) {
            node.press.animate_to(0.0, release);
            node.lift.animate_to(0.0, lift);
            node.stretch.animate_to(0.0, jelly);
        }
        self.drag = None;
        self.touch.intensity.animate_to(0.0, GLOW_OUT);
    }

    /// Advances all animation by `dt` seconds. Returns `true` while anything moves;
    /// when `false`, stop requesting frames until the next input or change.
    pub fn update(&mut self, dt: f32) -> bool {
        if !self.physics {
            self.settle();
            return false;
        }
        let dt = dt.clamp(0.0, 1.0 / 30.0);
        self.update_drag_stretch(dt);

        let mut moving = false;
        for node in self.nodes.values_mut() {
            for a in [
                &mut node.x,
                &mut node.y,
                &mut node.w,
                &mut node.h,
                &mut node.presence,
                &mut node.press,
                &mut node.lift,
                &mut node.stretch,
            ] {
                moving |= a.step(dt);
            }
        }
        moving |= self.touch.intensity.step(dt);
        moving |= self.touch.radius.step(dt);
        moving |= self.light_angle.step(dt);
        self.nodes.retain(|_, n| !(n.removing && [&n.presence, &n.x, &n.y, &n.w, &n.h].iter().all(|a| a.is_at_rest())));
        moving || self.drag.as_ref().is_some_and(|d| d.velocity != Vec2::ZERO)
    }

    /// Physics off: jumps every animation to its target.
    fn settle(&mut self) {
        for node in self.nodes.values_mut() {
            node.stretch.target = 0.0;
            for a in [&mut node.x, &mut node.y, &mut node.w, &mut node.h, &mut node.presence, &mut node.press, &mut node.lift, &mut node.stretch] {
                a.snap(a.target);
            }
        }
        for a in [&mut self.touch.intensity, &mut self.touch.radius, &mut self.light_angle] {
            a.snap(a.target);
        }
        self.nodes.retain(|_, n| !n.removing);
    }

    fn update_drag_stretch(&mut self, dt: f32) {
        let still = self.still();
        let Some(drag) = &mut self.drag else { return };
        if dt > 0.0 {
            let v = drag.moved / dt;
            drag.moved = Vec2::ZERO;
            drag.velocity = drag.velocity.lerp(v, 0.5);
            if drag.velocity.length_squared() < 1.0 {
                drag.velocity = Vec2::ZERO;
            }
        }
        let Some(node) = self.nodes.get_mut(drag.id) else { return };
        let speed = drag.velocity.length();
        if speed > 1.0 {
            node.stretch_dir = drag.velocity / speed;
        }
        let target = if still { 0.0 } else { (speed / STRETCH_SPEED).min(1.0) * MAX_STRETCH };
        node.stretch.animate_to(target, JELLY);
    }

    pub(crate) fn z_of(&self, node: &Node) -> i32 {
        node.glass.container.and_then(|c| self.containers.get(c)).map_or(node.glass.z, |c| c.z)
    }

    pub(crate) fn geometry(&self, node: &Node) -> Geometry {
        let size = Vec2::new(node.w.value.max(0.0), node.h.value.max(0.0));
        let grow = if self.still() { 0.0 } else { (10.0 / size.min_element().max(1.0)).min(0.15) };
        let scale = 1.0 + node.press.value * grow + node.lift.value * LENS_LIFT;
        let half = size * 0.5 * scale;
        let budget = half.min_element();
        let (radius, exponent) = match node.glass.shape {
            Shape::Capsule => (budget, 2.0),
            Shape::Circular(r) => ((r * scale).min(budget), 2.0),
            Shape::Rounded(r) => smooth_corner(r * scale, IOS_SMOOTHING, budget),
            Shape::Smooth { radius, smoothing } => smooth_corner(radius * scale, smoothing, budget),
        };
        Geometry {
            center: Vec2::new(node.x.value, node.y.value) + size * 0.5,
            half,
            radius,
            exponent,
            axis: node.stretch_dir,
            stretch: node.stretch.value.max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(scene: &mut Scene) {
        for _ in 0..600 {
            if !scene.update(1.0 / 120.0) {
                break;
            }
        }
    }

    #[test]
    fn removed_glass_disappears_after_dematerializing() {
        let mut scene = Scene::new();
        let id = scene.add(Glass::new(Rect::new(0.0, 0.0, 50.0, 50.0)));
        settle(&mut scene);
        scene.remove(id);
        settle(&mut scene);
        assert!(scene.glass(id).is_none());
    }

    #[test]
    fn hit_test_respects_rounded_corners() {
        let mut scene = Scene::new();
        scene.add(Glass::new(Rect::new(0.0, 0.0, 100.0, 100.0)).interactive());
        settle(&mut scene);
        assert!(scene.hit_test(Vec2::new(2.0, 2.0)).is_none());
    }

    #[test]
    fn ios_corner_reaches_further_along_the_edge_than_its_radius() {
        let (extent, exponent) = smooth_corner(30.0, IOS_SMOOTHING, 77.5);
        assert!((extent - 39.5).abs() < 0.1 && (exponent - 2.79).abs() < 0.01, "{extent} {exponent}");
    }

    #[test]
    fn capsule_budget_removes_smoothing() {
        assert_eq!(smooth_corner(100.0, IOS_SMOOTHING, 29.0), (29.0, 2.0));
    }

    #[test]
    fn squircle_and_circle_meet_at_the_diagonal() {
        // Figma keeps the 45° point of a smoothed corner close to the plain arc's.
        let (extent, n) = smooth_corner(30.0, IOS_SMOOTHING, 100.0);
        let squircle = extent - extent * 2f32.powf(-1.0 / n);
        let circle = 30.0 - 30.0 / 2f32.sqrt();
        assert!((squircle - circle).abs() < 0.3, "{squircle} vs {circle}");
    }

    #[test]
    fn hit_test_picks_topmost() {
        let mut scene = Scene::new();
        scene.add(Glass::new(Rect::new(0.0, 0.0, 100.0, 100.0)).interactive());
        let top = scene.add(Glass::new(Rect::new(0.0, 0.0, 100.0, 100.0)).interactive().z(1));
        assert_eq!(scene.hit_test(Vec2::new(50.0, 50.0)), Some(top));
    }

    #[test]
    fn lens_follows_pointer_and_settles() {
        let mut scene = Scene::new();
        let id = scene.add(Glass::new(Rect::new(0.0, 0.0, 40.0, 40.0)).lens());
        settle(&mut scene);
        scene.pointer_down(Vec2::new(20.0, 20.0));
        scene.pointer_move(Vec2::new(220.0, 20.0));
        scene.pointer_up();
        settle(&mut scene);
        assert!((scene.nodes[id].x.value - 200.0).abs() < 0.01);
    }

    #[test]
    fn fast_drag_stretches_lens() {
        let mut scene = Scene::new();
        let id = scene.add(Glass::new(Rect::new(0.0, 0.0, 40.0, 40.0)).lens());
        settle(&mut scene);
        scene.pointer_down(Vec2::new(20.0, 20.0));
        for i in 1..20 {
            scene.pointer_move(Vec2::new(20.0 + i as f32 * 30.0, 20.0));
            scene.update(1.0 / 120.0);
        }
        assert!(scene.nodes[id].stretch.value > 0.1);
    }

    #[test]
    fn physics_off_lands_frames_instantly() {
        let mut scene = Scene::new();
        scene.physics = false;
        let id = scene.add(Glass::new(Rect::new(0.0, 0.0, 40.0, 40.0)));
        scene.set_frame(id, Rect::new(100.0, 0.0, 80.0, 40.0));
        assert!(!scene.update(1.0 / 120.0));
        assert_eq!(scene.current_frame(id), Some(Rect::new(100.0, 0.0, 80.0, 40.0)));
        assert_eq!(scene.nodes[id].presence.value, 1.0);
        scene.remove(id);
        scene.update(1.0 / 120.0);
        assert!(scene.glass(id).is_none());
    }

    #[test]
    fn expand_starts_at_source_and_collapses_back() {
        let mut scene = Scene::new();
        let c = scene.add_container(20.0, 0);
        let bar = scene.add(Glass::new(Rect::new(10.0, 10.0, 200.0, 44.0)).container(c));
        settle(&mut scene);
        let panel = scene.expand_from(bar, Glass::new(Rect::new(10.0, 70.0, 200.0, 300.0)), Spring::SNAPPY);
        assert_eq!(scene.current_frame(panel), Some(Rect::new(10.0, 10.0, 200.0, 44.0)));
        assert_eq!(scene.glass(panel).map(|g| g.container), Some(Some(c)));
        assert_eq!(scene.nodes[panel].presence.value, 1.0, "emerges fused, not materializing");
        settle(&mut scene);
        assert!((scene.nodes[panel].h.value - 300.0).abs() < 0.01);
        scene.collapse_into(panel, bar, Spring::SNAPPY);
        settle(&mut scene);
        assert!(scene.glass(panel).is_none());
    }

    #[test]
    fn reduce_motion_disables_stretch() {
        let mut scene = Scene::new();
        scene.appearance.reduce_motion = true;
        let id = scene.add(Glass::new(Rect::new(0.0, 0.0, 40.0, 40.0)).lens());
        settle(&mut scene);
        scene.pointer_down(Vec2::new(20.0, 20.0));
        for i in 1..20 {
            scene.pointer_move(Vec2::new(20.0 + i as f32 * 30.0, 20.0));
            scene.update(1.0 / 120.0);
        }
        assert!(scene.nodes[id].stretch.value.abs() < 1e-3);
    }
}
