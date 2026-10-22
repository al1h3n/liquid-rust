//! Analytic damped springs with Apple's `duration` / `bounce` parameterisation.
//!
//! Springs are solved in closed form, so any frame delta (60 Hz, 120 Hz, a dropped
//! frame) lands on the same curve, and retargeting mid-flight keeps velocity — the
//! behaviour of SwiftUI's `spring(duration:bounce:)`.

use std::f32::consts::TAU;

/// Spring timing in Apple's terms: `duration` is the perceptual settle time in
/// seconds, `bounce` is 0 for no overshoot, positive for bounce, negative for a
/// flatter, over-damped curve.
///
/// ```
/// use liquid_rust::Spring;
/// assert_eq!(Spring::SNAPPY.bounce, 0.15);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    /// Perceptual duration in seconds.
    pub duration: f32,
    /// Bounce in `-1.0..1.0`. Solved within `-0.95..=0.95`, where a spring still settles.
    pub bounce: f32,
}

impl Spring {
    /// SwiftUI `.smooth`: no overshoot.
    pub const SMOOTH: Self = Self::new(0.5, 0.0);
    /// SwiftUI `.snappy`: small overshoot, used for layout and morphs.
    pub const SNAPPY: Self = Self::new(0.5, 0.15);
    /// SwiftUI `.bouncy`: visible overshoot, used after release.
    pub const BOUNCY: Self = Self::new(0.5, 0.3);
    /// SwiftUI `.interactiveSpring`: tight follow of a finger.
    pub const INTERACTIVE: Self = Self::new(0.15, 0.14);

    /// Creates a spring from a duration (seconds) and bounce.
    pub const fn new(duration: f32, bounce: f32) -> Self {
        Self { duration, bounce }
    }

    /// The critically damped version of this spring, used for Reduce Motion.
    pub fn without_bounce(self) -> Self {
        Self::new(self.duration * 0.8, self.bounce.min(0.0))
    }

    fn angular_frequency(self) -> f32 {
        TAU / self.duration.max(1e-3)
    }

    fn damping_ratio(self) -> f32 {
        // At a bounce of 1 the spring rings forever and the host never stops rendering;
        // at -1 the ratio is infinite, and past either end the value runs away.
        let bounce = self.bounce.clamp(-0.95, 0.95);
        if bounce >= 0.0 { 1.0 - bounce } else { 1.0 / (1.0 + bounce) }
    }

    /// Advances displacement `x0` and velocity `v0` by `t` seconds, returning the new
    /// displacement and velocity.
    pub fn solve(self, x0: f32, v0: f32, t: f32) -> (f32, f32) {
        let w0 = self.angular_frequency();
        let zeta = self.damping_ratio();
        if zeta < 1.0 {
            let a = zeta * w0;
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            let (s, c) = (wd * t).sin_cos();
            let e = (-a * t).exp();
            let b = (v0 + a * x0) / wd;
            (e * (x0 * c + b * s), e * (v0 * c - (x0 * wd + a * b) * s))
        } else if zeta == 1.0 {
            let e = (-w0 * t).exp();
            let b = v0 + w0 * x0;
            (e * (x0 + b * t), e * (v0 - w0 * b * t))
        } else {
            let q = (zeta * zeta - 1.0).sqrt();
            let (r1, r2) = (-w0 * (zeta - q), -w0 * (zeta + q));
            let c2 = (v0 - r1 * x0) / (r2 - r1);
            let c1 = x0 - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        }
    }
}

/// A scalar driven by a [`Spring`] toward a target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animated {
    /// Current value.
    pub value: f32,
    /// Current velocity in units per second.
    pub velocity: f32,
    /// Value the spring settles at.
    pub target: f32,
    /// Timing used for the next steps.
    pub spring: Spring,
}

const REST_EPSILON: f32 = 1e-3;

/// Far from zero an `f32` can't hold a value within [`REST_EPSILON`] of its target and
/// stops a few float steps short: 1/8192 apart at 1024, 1/16 at a million. A spring whose
/// remaining swing fits in this many steps has nothing left that the value can show.
const REST_STEPS: f32 = 16.0;

impl Animated {
    /// A value at rest.
    pub const fn new(value: f32, spring: Spring) -> Self {
        Self { value, velocity: 0.0, target: value, spring }
    }

    /// Retargets, keeping the current value and velocity (interruptible animation).
    pub fn animate_to(&mut self, target: f32, spring: Spring) {
        self.target = target;
        self.spring = spring;
    }

    /// Jumps to `value` with no animation.
    pub fn snap(&mut self, value: f32) {
        *self = Self::new(value, self.spring);
    }

    /// Advances by `dt` seconds. Returns `true` while still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        if self.is_at_rest() {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        let (x, v) = self.spring.solve(self.value - self.target, self.velocity, dt);
        if !(x.is_finite() && v.is_finite()) {
            // A NaN or infinite target, step or spring (a layout that divided by zero) lands
            // at once; left alone it would never come to rest.
            if !self.target.is_finite() {
                self.target = if self.value.is_finite() { self.value } else { 0.0 };
            }
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        let value = self.target + x;
        if dt > 0.0 && value == self.value && x.abs() <= self.stall_distance(dt) {
            // The step rounded back to the same value, close to the target: the spring is
            // stalled there for good, so it lands instead of reporting motion forever.
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        self.value = value;
        self.velocity = v;
        true
    }

    /// `true` when the value sits at its target with negligible velocity, or as close to
    /// it as an `f32` gets (within 16 float steps, at large values).
    pub fn is_at_rest(&self) -> bool {
        let offset = self.value - self.target;
        if offset.abs() < REST_EPSILON && self.velocity.abs() < REST_EPSILON {
            return true;
        }
        // How far the spring can still swing: its offset plus what its speed carries it.
        // The offset alone rules out almost every moving frame, before the costlier part.
        let limit = REST_STEPS * self.float_step();
        offset.abs() <= limit && offset.hypot(self.velocity / self.spring.angular_frequency()) <= limit
    }

    /// Gap between the target and the next larger `f32`: the finest move the value can make.
    fn float_step(&self) -> f32 {
        self.target.abs().next_up() - self.target.abs()
    }

    /// Offset within which a step of `dt` can't move the value at all. Held at offset `x`,
    /// a spring creeps toward the target at about `w0·x / 2ζ`; once that covers under half
    /// a float step per frame, every step rounds back to the same value (the limit allows
    /// a whole step, for margin). Slow and
    /// over-damped springs stall there well beyond [`REST_STEPS`]. Frames under a
    /// millisecond count as one, so a burst of tiny steps can't land a spring early.
    fn stall_distance(&self, dt: f32) -> f32 {
        let creep_per_offset = self.spring.angular_frequency() / (2.0 * self.spring.damping_ratio());
        self.float_step() / (creep_per_offset * dt.max(1e-3))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(spring: Spring, steps: usize, dt: f32) -> Animated {
        let mut a = Animated::new(0.0, spring);
        a.animate_to(1.0, spring);
        for _ in 0..steps {
            a.step(dt);
        }
        a
    }

    #[test]
    fn spring_is_frame_rate_independent() {
        let at_60 = settle(Spring::BOUNCY, 18, 1.0 / 60.0).value;
        let at_120 = settle(Spring::BOUNCY, 36, 1.0 / 120.0).value;
        assert!((at_60 - at_120).abs() < 1e-4, "{at_60} vs {at_120}");
    }

    #[test]
    fn bouncy_spring_overshoots() {
        let mut a = Animated::new(0.0, Spring::BOUNCY);
        a.animate_to(1.0, Spring::BOUNCY);
        let peak = (0..120).map(|_| { a.step(1.0 / 120.0); a.value }).fold(0.0, f32::max);
        assert!(peak > 1.02, "peak {peak}");
    }

    #[test]
    fn smooth_spring_never_overshoots() {
        let mut a = Animated::new(0.0, Spring::SMOOTH);
        a.animate_to(1.0, Spring::SMOOTH);
        let peak = (0..240).map(|_| { a.step(1.0 / 120.0); a.value }).fold(0.0, f32::max);
        assert!(peak <= 1.0 + 1e-4, "peak {peak}");
    }

    #[test]
    fn spring_settles_within_two_durations() {
        assert!(settle(Spring::SNAPPY, 120, 1.0 / 120.0).is_at_rest());
    }

    #[test]
    fn bounce_outside_apples_range_still_settles() {
        for bounce in [1.0, 3.0, -1.0, -3.0] {
            let a = settle(Spring::new(0.2, bounce), 40_000, 1.0 / 120.0);
            assert!(a.is_at_rest(), "bounce {bounce} left the value at {}", a.value);
        }
    }

    /// Steps until `step` reports rest, or `None` if it is still moving after `limit`.
    fn steps_to_rest(a: &mut Animated, dt: f32, limit: usize) -> Option<usize> {
        (1..=limit).find(|_| !a.step(dt))
    }

    #[test]
    fn spring_to_a_large_target_comes_to_rest() {
        // Past 1024 the value used to stop a float step short of the target while its speed
        // stayed above REST_EPSILON, so it reported motion forever.
        for spring in [Spring::SMOOTH, Spring::SNAPPY, Spring::BOUNCY, Spring::INTERACTIVE] {
            for target in [1100.0, 2048.0, 5000.0, 1e5, 1e6, -3000.0] {
                for dt in [1.0 / 240.0, 1.0 / 120.0, 1.0 / 60.0] {
                    let mut a = Animated::new(0.0, spring);
                    a.animate_to(target, spring);
                    let steps = steps_to_rest(&mut a, dt, (3.0 / dt) as usize);
                    assert!(steps.is_some(), "{spring:?} to {target} at {dt}: still moving at {} ({} short)", a.value, target - a.value);
                    assert_eq!(a.value, target);
                    assert!(a.is_at_rest());
                }
            }
        }
    }

    #[test]
    fn extreme_springs_at_large_values_come_to_rest() {
        // A ringing spring jitters a float step either side of its target; a slow,
        // over-damped one stalls tens of steps short at a high frame rate.
        for spring in [Spring::new(0.05, 0.95), Spring::new(2.0, -0.95), Spring::new(1.0, 0.5)] {
            for dt in [1.0 / 1000.0, 1.0 / 60.0] {
                let mut a = Animated::new(0.0, spring);
                a.animate_to(1e5, spring);
                assert!(steps_to_rest(&mut a, dt, 200_000).is_some(), "{spring:?} at {dt} stuck at {}", a.value);
                assert_eq!(a.value, 1e5);
            }
        }
    }

    #[test]
    fn fast_spring_at_a_large_target_keeps_moving() {
        // Passing through the target at speed is motion, not rest.
        let mut a = Animated { value: 1e6, velocity: 500.0, target: 1e6, spring: Spring::SNAPPY };
        assert!(!a.is_at_rest());
        assert!(a.step(1.0 / 120.0));
        assert!(a.value > 1e6);
    }

    #[test]
    fn tiny_steps_do_not_land_a_spring_early() {
        // Steps too small to move the value must not read as a stall.
        let mut a = Animated::new(0.0, Spring::SMOOTH);
        a.animate_to(1000.0, Spring::SMOOTH);
        while (a.target - a.value).abs() > 0.5 {
            a.step(1.0 / 120.0);
        }
        let before = a.value;
        assert!(a.step(0.0));
        assert!(a.step(1e-7));
        assert_eq!(a.value, before);
    }

    #[test]
    fn non_finite_input_lands_instead_of_animating_forever() {
        let mut a = Animated::new(5.0, Spring::SNAPPY);
        a.animate_to(f32::NAN, Spring::SNAPPY);
        assert!(!a.step(1.0 / 120.0));
        assert_eq!((a.value, a.target), (5.0, 5.0), "a NaN target is dropped");

        a.animate_to(9.0, Spring::new(f32::NAN, f32::NAN));
        assert!(!a.step(1.0 / 120.0));
        assert_eq!(a.value, 9.0, "a NaN spring lands on the target");

        a.animate_to(2.0, Spring::SNAPPY);
        assert!(!a.step(f32::NAN));
        assert_eq!(a.value, 2.0, "a NaN step lands on the target");
    }

    #[test]
    fn overdamped_velocity_matches_finite_difference() {
        let s = Spring::new(0.4, -0.5);
        let (x1, v1) = s.solve(1.0, 0.5, 0.1);
        let (x2, _) = s.solve(1.0, 0.5, 0.1 + 1e-3);
        assert!(((x2 - x1) / 1e-3 - v1).abs() < 1e-2);
    }

    #[test]
    fn underdamped_velocity_matches_finite_difference() {
        let (x1, v1) = Spring::BOUNCY.solve(1.0, -2.0, 0.07);
        let (x2, _) = Spring::BOUNCY.solve(1.0, -2.0, 0.07 + 1e-3);
        assert!(((x2 - x1) / 1e-3 - v1).abs() < 2e-2);
    }
}
