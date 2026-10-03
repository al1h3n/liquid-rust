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
    /// Bounce in `-1.0..1.0`.
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

    fn damping_ratio(self) -> f32 {
        if self.bounce >= 0.0 {
            1.0 - self.bounce
        } else {
            1.0 / (1.0 + self.bounce)
        }
    }

    /// Advances displacement `x0` and velocity `v0` by `t` seconds, returning the new
    /// displacement and velocity.
    pub fn solve(self, x0: f32, v0: f32, t: f32) -> (f32, f32) {
        let w0 = TAU / self.duration.max(1e-3);
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
        self.value = self.target + x;
        self.velocity = v;
        true
    }

    /// `true` when the value sits at its target with negligible velocity.
    pub fn is_at_rest(&self) -> bool {
        (self.value - self.target).abs() < REST_EPSILON && self.velocity.abs() < REST_EPSILON
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
