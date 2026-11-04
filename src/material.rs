//! Glass materials: Apple's presets (regular, clear, tinted, identity) and the knobs
//! behind them, named after Figma's GLASS panel so values can be copied from a design.

/// An sRGB colour with straight alpha, components in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Red, sRGB encoded.
    pub r: f32,
    /// Green, sRGB encoded.
    pub g: f32,
    /// Blue, sRGB encoded.
    pub b: f32,
    /// Alpha. For glass tints this is the tint strength.
    pub a: f32,
}

impl Color {
    /// iOS 26 `systemBlue` (`#0088FF`), as used for active toggles in Apple's kit.
    pub const BLUE: Self = Self::hex(0x0088FF);
    /// iOS 26 `systemGreen` (`#34C759`).
    pub const GREEN: Self = Self::hex(0x34C759);
    /// iOS 26 `systemRed` (`#FF383C`).
    pub const RED: Self = Self::hex(0xFF383C);
    /// iOS `systemYellow` (`#FFCC00`).
    pub const YELLOW: Self = Self::hex(0xFFCC00);
    /// Control Center speaker cyan (`#00C0E8`).
    pub const CYAN: Self = Self::hex(0x00C0E8);
    /// Opaque white.
    pub const WHITE: Self = Self::hex(0xFFFFFF);

    /// Opaque colour from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
            g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
            b: (rgb & 0xFF) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// Returns the colour with a different alpha.
    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Converts to OKLab (L, a, b).
    pub(crate) fn to_oklab(self) -> [f32; 3] {
        let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        let (r, g, b) = (lin(self.r), lin(self.g), lin(self.b));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        [0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s, 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s, 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s]
    }
}

/// How the backdrop is tone-mapped under the glass ("Fill + Shadow" fills in Apple's
/// kit). Operations run in order, in sRGB-encoded space, like Figma blend modes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    /// `LINEAR_DODGE` with this grey (e.g. `#101010` = 0.063).
    pub lift: f32,
    /// Target luminance of a `LUMINOSITY` fill (1.0 = white, 0.6 = `#999999`).
    pub luminance: f32,
    /// Opacity of that `LUMINOSITY` fill.
    pub luminance_mix: f32,
    /// Opacity of a white `LIGHTEN` fill.
    pub lighten: f32,
    /// Grey of a `DARKEN` fill.
    pub darken_to: f32,
    /// Opacity of that `DARKEN` fill.
    pub darken: f32,
}

impl Tone {
    /// Leaves the backdrop untouched.
    pub const NONE: Self = Self { lift: 0.0, luminance: 0.0, luminance_mix: 0.0, lighten: 0.0, darken_to: 1.0, darken: 0.0 };
}

/// Drop shadow under the glass, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    /// Vertical offset (positive = down).
    pub offset_y: f32,
    /// Figma blur radius.
    pub blur: f32,
    /// Opacity of black.
    pub opacity: f32,
}

/// Every parameter of one glass surface. Lengths are in points.
///
/// Start from a preset and adjust:
/// ```
/// use liquid_rust::{Color, Material};
/// let primary = Material::tinted(Color::BLUE);
/// let quiet = Material { frost: 10.0, ..Material::clear() };
/// assert!(primary.tint.is_some() && quiet.frost == 10.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Bevel displacement strength, `0..=1` (Figma *Refraction* / 100).
    pub refraction: f32,
    /// Bevel width in points: how far the curved edge reaches inward (Figma *Depth*).
    pub depth: f32,
    /// Chromatic split inside the bevel, `0..=1` (Figma *Dispersion* / 100).
    pub dispersion: f32,
    /// Backdrop blur radius in points (Figma *Frost*).
    pub frost: f32,
    /// How widely the specular light spreads around the rim, `0..=1` (Figma *Splay*).
    pub splay: f32,
    /// Specular rim strength, `0..=1` (Figma *Light* %).
    pub specular: f32,
    /// Bright inner rims at the lit and opposite edges (`#282828` dodge = 0.157).
    pub rim: f32,
    /// Broad soft shade under the lit edge (`#E6E6E6` burn = 0.098).
    pub rim_shade: f32,
    /// Crisp 1.25 pt inner hairlines (dock style), `0..=1`.
    pub hairline: f32,
    /// Darkened outer hairline (iOS 27 edge), burn amount `0..=1`.
    pub edge: f32,
    /// Drop shadow.
    pub shadow: Shadow,
    /// Backdrop tone when the glass is light.
    pub light_tone: Tone,
    /// Backdrop tone when the glass is dark.
    pub dark_tone: Tone,
    /// Regular behaviour: thicker when larger, flips light/dark with the backdrop
    /// when small, follows the colour scheme when large.
    pub adaptive: bool,
    /// Stained-glass tint; alpha is the strength (1.0 still lets 15 % of the backdrop
    /// through). Reserve for the primary action.
    pub tint: Option<Color>,
    /// Localised dimming of the backdrop, `0..=1` (Apple: 0.35 for Clear over bright media).
    pub dim: f32,
}

impl Material {
    /// `Glass.regular`: adaptive glass for bars, buttons, menus and anything with text.
    pub const fn regular() -> Self {
        Self {
            refraction: 0.7,
            depth: 30.0,
            dispersion: 0.2,
            frost: 10.0,
            splay: 0.2,
            specular: 0.3,
            rim: 0.157,
            rim_shade: 0.05,
            hairline: 0.0,
            edge: 0.2,
            shadow: Shadow { offset_y: 8.0, blur: 15.0, opacity: 0.06 },
            light_tone: Tone { lift: 0.0, luminance: 0.92, luminance_mix: 0.3, lighten: 0.2, darken_to: 0.75, darken: 0.1 },
            dark_tone: Tone { lift: 0.03, luminance: 0.25, luminance_mix: 0.2, lighten: 0.0, darken_to: 0.0, darken: 0.25 },
            adaptive: true,
            tint: None,
            dim: 0.0,
        }
    }

    /// `Glass.clear`: Apple kit "Clear Glass" (Lock Screen, Control Center). Use only
    /// over media; add [`Material::dim`] 0.35 when the media is bright.
    pub const fn clear() -> Self {
        let tone = Tone { lift: 0.063, luminance: 1.0, luminance_mix: 0.04, lighten: 0.0, darken_to: 1.0, darken: 0.0 };
        Self {
            refraction: 0.7,
            depth: 30.0,
            dispersion: 0.2,
            frost: 6.0,
            splay: 0.2,
            specular: 0.4,
            rim: 0.157,
            rim_shade: 0.098,
            hairline: 0.0,
            edge: 0.2,
            shadow: Shadow { offset_y: 8.0, blur: 15.0, opacity: 0.02 },
            light_tone: tone,
            dark_tone: tone,
            adaptive: false,
            tint: None,
            dim: 0.0,
        }
    }

    /// Clear chrome from Apple's kit (Dock, Home Screen search pill): luminance pulled
    /// toward mid grey for legibility, light frost, crisp hairlines.
    pub const fn clear_bar() -> Self {
        let tone = Tone { lift: 0.0, luminance: 0.6, luminance_mix: 0.33, lighten: 0.0, darken_to: 1.0, darken: 0.0 };
        Self {
            frost: 3.0,
            specular: 0.2,
            rim_shade: 0.0,
            hairline: 0.157,
            light_tone: tone,
            dark_tone: tone,
            ..Self::clear()
        }
    }

    /// `Glass.regular.tint(color)`: the primary action. `color.a` is the strength.
    pub const fn tinted(color: Color) -> Self {
        Self { tint: Some(color), ..Self::regular() }
    }

    /// `Glass.identity`: renders nothing (use as a transition endpoint).
    pub const fn identity() -> Self {
        Self {
            refraction: 0.0,
            depth: 1.0,
            dispersion: 0.0,
            frost: 0.0,
            splay: 0.0,
            specular: 0.0,
            rim: 0.0,
            rim_shade: 0.0,
            hairline: 0.0,
            edge: 0.0,
            shadow: Shadow { offset_y: 0.0, blur: 0.0, opacity: 0.0 },
            light_tone: Tone::NONE,
            dark_tone: Tone::NONE,
            adaptive: false,
            tint: None,
            dim: 0.0,
        }
    }

    /// Returns a copy with a tint (`Glass.tint(_:)`).
    pub const fn tint(self, color: Color) -> Self {
        Self { tint: Some(color), ..self }
    }

    pub(crate) fn is_identity(&self) -> bool {
        *self == Self::identity()
    }

    /// `self` at `t = 0`, `to` at `t = 1`, every knob in between (`t` is clamped, so a
    /// spring's overshoot never extrapolates). A tint only one side has fades in or out by
    /// its strength, instead of passing through black.
    pub(crate) fn lerp(&self, to: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        // This form is exact at both ends (`a + (b - a) * t` isn't at `t = 1`).
        let f = |a: f32, b: f32| a * (1.0 - t) + b * t;
        let tone = |a: Tone, b: Tone| Tone {
            lift: f(a.lift, b.lift),
            luminance: f(a.luminance, b.luminance),
            luminance_mix: f(a.luminance_mix, b.luminance_mix),
            lighten: f(a.lighten, b.lighten),
            darken_to: f(a.darken_to, b.darken_to),
            darken: f(a.darken, b.darken),
        };
        let tint = match (self.tint, to.tint) {
            (None, None) => None,
            (Some(a), None) => Some(a.with_alpha(a.a * (1.0 - t))),
            (None, Some(b)) => Some(b.with_alpha(b.a * t)),
            (Some(a), Some(b)) => Some(Color { r: f(a.r, b.r), g: f(a.g, b.g), b: f(a.b, b.b), a: f(a.a, b.a) }),
        };
        Self {
            refraction: f(self.refraction, to.refraction),
            depth: f(self.depth, to.depth),
            dispersion: f(self.dispersion, to.dispersion),
            frost: f(self.frost, to.frost),
            splay: f(self.splay, to.splay),
            specular: f(self.specular, to.specular),
            rim: f(self.rim, to.rim),
            rim_shade: f(self.rim_shade, to.rim_shade),
            hairline: f(self.hairline, to.hairline),
            edge: f(self.edge, to.edge),
            shadow: Shadow {
                offset_y: f(self.shadow.offset_y, to.shadow.offset_y),
                blur: f(self.shadow.blur, to.shadow.blur),
                opacity: f(self.shadow.opacity, to.shadow.opacity),
            },
            light_tone: tone(self.light_tone, to.light_tone),
            dark_tone: tone(self.dark_tone, to.dark_tone),
            adaptive: if t < 0.5 { self.adaptive } else { to.adaptive },
            tint,
            dim: f(self.dim, to.dim),
        }
    }
}

/// System-wide appearance and accessibility, shared by every glass in a scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    /// Dark colour scheme. Large regular glass follows it; small glass adapts to content.
    pub dark: bool,
    /// iOS 27 Liquid Glass slider: 0 = ultra clear, 0.5 = default, 1 = fully tinted.
    pub transparency: f32,
    /// Reduce Transparency: frostier, more opaque glass.
    pub reduce_transparency: bool,
    /// Increase Contrast: stronger edges and a contrasting border.
    pub increase_contrast: bool,
    /// Reduce Motion: no bounce, no stretch, no press scale.
    pub reduce_motion: bool,
    /// `true` (Apple's default): small adaptive glass flips light or dark with the content
    /// behind it. `false`: it follows [`Appearance::dark`] like large glass, for interfaces
    /// whose labels follow the colour scheme and would vanish on flipped glass.
    pub adapt_to_content: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            dark: false,
            transparency: 0.5,
            reduce_transparency: false,
            increase_contrast: false,
            reduce_motion: false,
            adapt_to_content: true,
        }
    }
}

/// Material values after size and appearance adaptation, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Resolved {
    pub material: Material,
    /// 0 = follow the colour scheme, 1 = flip with the backdrop.
    pub adapt: f32,
    pub contrast: f32,
}

/// How "thick" adaptive glass is, `0..=1`, from its smaller side in points.
pub(crate) fn thickness(min_side: f32) -> f32 {
    smoothstep(44.0, 320.0, min_side)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn more_opaque(amount: f32, k: f32) -> f32 {
    if k >= 0.0 { amount + (1.0 - amount) * k } else { amount * (1.0 + k) }
}

/// Grey Apple's tinted dark glass settles on: `systemGray6` dark, `#1C1C1E`.
pub(crate) const NEAR_BLACK: f32 = 0x1C as f32 / 255.0;

/// Opacity the slider's "fully tinted" end gives each face, measured from Apple's iOS 27
/// screenshots (menus, dark Home Screen widgets, Clear vs Tinted): dark glass about 70–85 %
/// `#1C1C1E`, light glass about 70–80 % white frost.
const TINTED_DARKEN: f32 = 0.6;
const TINTED_LIGHTEN: f32 = 0.6;
/// Neither face goes fully opaque: a trace of the backdrop's colour always shows through.
const TINTED_MAX: f32 = 0.88;

/// Moves both faces toward Apple's "tinted" glass by `k` (`0..=1`): the dark face darkens
/// toward near-black `#1C1C1E`, the light face whitens. Neither is pulled toward a mid
/// luminance, which is what used to make tinted glass read flat grey: the luminosity fill,
/// the dark face's white and the light face's grey cap stay exactly as they were.
fn tint_faces(m: &mut Material, k: f32) {
    let dark = &mut m.dark_tone;
    // A DARKEN fill with #1C1C1E is Apple's normal-blended near-black for everything brighter
    // than it, and leaves darker content alone instead of lifting it to grey.
    dark.darken_to += (NEAR_BLACK - dark.darken_to) * k;
    dark.darken = more_opaque(dark.darken, TINTED_DARKEN * k).min(TINTED_MAX.max(dark.darken));
    let light = &mut m.light_tone;
    light.lighten = more_opaque(light.lighten, TINTED_LIGHTEN * k).min(TINTED_MAX.max(light.lighten));
}

impl Material {
    /// Adapts the material to an element whose smaller side is `min_side` points.
    pub(crate) fn resolve(&self, min_side: f32, appearance: &Appearance) -> Resolved {
        let mut m = *self;
        let mut adapt = 0.0;
        if m.adaptive {
            // Apple: larger glass is "thicker" — deeper shadow, more lensing, softer
            // scattering, more opaque — and stops flipping light/dark.
            let tau = thickness(min_side);
            m.frost *= 1.0 + 0.6 * tau;
            m.depth *= 1.0 + 0.25 * tau;
            m.specular -= 0.05 * tau;
            m.shadow.blur += 33.0 * tau;
            m.shadow.opacity += 0.19 * tau;
            // Large light glass reads as white frost, so dark text on it stays legible even
            // over a dark, busy backdrop; the luminosity fill grows with it so the face
            // whitens instead of washing toward a flat grey.
            m.light_tone.lighten += 0.55 * tau;
            m.light_tone.luminance_mix += 0.15 * tau;
            m.dark_tone.darken += 0.27 * tau;
            if appearance.adapt_to_content {
                // Only small glass (tab bars, buttons) flips with the content; from about
                // 180 pt (menus, panels, sidebars) glass follows the colour scheme.
                adapt = 1.0 - smoothstep(0.0, 0.5, tau);
            }
        }

        let k = (appearance.transparency.clamp(0.0, 1.0) - 0.5) * 2.0;
        if appearance.reduce_transparency {
            // Frostier and as opaque as fully tinted, without the stronger rims.
            m.frost = (m.frost * 3.0).max(20.0) * 1.1f32.exp2();
            tint_faces(&mut m, 1.0);
        } else if k > 0.0 {
            // Toward "fully tinted" (iOS 26.1: "increases opacity and adds more contrast"): a
            // more opaque face and stronger rims, but frost grows only a little, so the bevel
            // still visibly bends the content behind it.
            m.frost *= (0.5 * k).exp2();
            tint_faces(&mut m, k);
            m.edge = (m.edge + 0.25 * k).min(1.0);
            m.rim += 0.08 * k;
            m.specular += 0.08 * k;
        } else {
            // Toward "ultra clear": less frost and every fill fades.
            m.frost *= (1.1 * k).exp2();
            for tone in [&mut m.light_tone, &mut m.dark_tone] {
                tone.luminance_mix = more_opaque(tone.luminance_mix, 0.45 * k);
                tone.lighten = more_opaque(tone.lighten, 0.35 * k);
                tone.darken = more_opaque(tone.darken, 0.45 * k);
            }
        }

        let contrast = if appearance.increase_contrast {
            m.edge = (m.edge * 2.5).min(1.0);
            1.0
        } else {
            0.0
        };
        Resolved { material: m, adapt, contrast }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_hits_both_ends_and_fades_a_tint() {
        let (plain, blue) = (Material::regular(), Material::tinted(Color::BLUE));
        assert_eq!(plain.lerp(&blue, 1.0), blue);
        assert_eq!(blue.lerp(&plain, 1.0).tint.map(|c| c.a), Some(0.0), "fades out by strength");
        assert_eq!(plain.lerp(&blue, 0.0).tint.map(|c| c.a), Some(0.0), "fades in from zero, not from black");
        let half = plain.lerp(&blue, 0.5).tint.unwrap();
        assert_eq!((half.r, half.g, half.b, half.a), (Color::BLUE.r, Color::BLUE.g, Color::BLUE.b, 0.5), "keeps the hue while it fades");
        assert_eq!(plain.lerp(&Material::clear(), 1.4), Material::clear(), "overshoot is clamped");
        let mid = plain.lerp(&Material::clear(), 0.5);
        assert!((mid.frost - 8.0).abs() < 1e-5 && (mid.specular - 0.35).abs() < 1e-5);
    }

    #[test]
    fn large_regular_glass_is_frostier_than_small() {
        let a = Appearance::default();
        let small = Material::regular().resolve(44.0, &a).material.frost;
        let large = Material::regular().resolve(400.0, &a).material.frost;
        assert!(large > small * 1.5);
    }

    #[test]
    fn clear_glass_does_not_adapt_to_size() {
        let a = Appearance::default();
        assert_eq!(Material::clear().resolve(44.0, &a).material, Material::clear().resolve(400.0, &a).material);
    }

    #[test]
    fn default_transparency_keeps_kit_values() {
        let r = Material::clear().resolve(58.0, &Appearance::default());
        assert!((r.material.frost - 6.0).abs() < 1e-5);
    }

    #[test]
    fn fully_tinted_slider_makes_glass_more_opaque() {
        let tinted = Appearance { transparency: 1.0, ..Appearance::default() };
        let r = Material::regular().resolve(44.0, &tinted).material;
        assert!(r.light_tone.lighten > Material::regular().light_tone.lighten);
    }

    // ------------------------------------------------ tone checks without a GPU

    /// CPU copy of `tone()` in glass.wgsl (sRGB-encoded space): lift, LUMINOSITY fill, white
    /// LIGHTEN fill, grey DARKEN fill. Keep the two in step; `tests/gpu.rs` checks the shader.
    fn tone(c: [f32; 3], t: &Tone) -> [f32; 3] {
        let blend_lum = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
        let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let x = c.map(|v| (v + t.lift).min(1.0));
        // W3C SetLum with ClipColor (Figma's LUMINOSITY blend).
        let d = t.luminance - blend_lum(x);
        let mut s = x.map(|v| v + d);
        let (l, lo, hi) = (blend_lum(s), s[0].min(s[1]).min(s[2]), s[0].max(s[1]).max(s[2]));
        if lo < 0.0 {
            s = s.map(|v| l + (v - l) * l / (l - lo).max(1e-5));
        }
        if hi > 1.0 {
            s = s.map(|v| l + (v - l) * (1.0 - l) / (hi - l).max(1e-5));
        }
        [0, 1, 2].map(|i| {
            let v = mix(x[i], s[i], t.luminance_mix);
            let v = mix(v, 1.0, t.lighten);
            mix(v, v.min(t.darken_to), t.darken)
        })
    }

    /// WCAG relative luminance of an sRGB colour.
    fn luminance(c: [f32; 3]) -> f32 {
        let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
    }

    /// HSV saturation.
    fn saturation(c: [f32; 3]) -> f32 {
        let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        if hi > 0.0 { (hi - lo) / hi } else { 0.0 }
    }

    /// Saturated backdrops (already frosted, so flat): blue, orange, green, magenta, and a
    /// dark busy wallpaper's average.
    const BACKDROPS: [[f32; 3]; 5] = [[0.13, 0.38, 0.85], [0.95, 0.5, 0.1], [0.2, 0.75, 0.35], [0.8, 0.2, 0.6], [0.25, 0.2, 0.35]];

    fn faces(min_side: f32, transparency: f32) -> (Tone, Tone) {
        let m = Material::regular().resolve(min_side, &Appearance { transparency, ..Appearance::default() }).material;
        (m.dark_tone, m.light_tone)
    }

    #[test]
    fn fully_tinted_dark_glass_is_near_black_not_grey() {
        for side in [44.0, 180.0, 400.0] {
            let ((default, _), (tinted, _)) = (faces(side, 0.5), faces(side, 1.0));
            for c in BACKDROPS {
                let (before, after) = (tone(c, &default), tone(c, &tinted));
                // Darker than the default, except over content so dark that the default already
                // went below Apple's #1C1C1E: tinted glass settles on that grey, not on black.
                let floor = 1.5 * luminance([NEAR_BLACK; 3]);
                assert!(luminance(after) < luminance(before).max(floor), "{side} pt over {c:?}: darker than the default");
                // Apple: about 70-85 % #1C1C1E, so at most ~30 % of the backdrop's light.
                assert!(luminance(after) < 0.3 * luminance(c).max(0.05) + luminance([NEAR_BLACK; 3]), "{side} pt over {c:?}: {after:?}");
                let kept = saturation(after) / saturation(c);
                assert!(kept >= 0.4, "{side} pt over {c:?}: keeps only {:.0} % of the colour", kept * 100.0);
            }
        }
    }

    #[test]
    fn fully_tinted_light_glass_is_white_frost() {
        for side in [44.0, 180.0, 400.0] {
            let ((_, default), (_, tinted)) = (faces(side, 0.5), faces(side, 1.0));
            for c in BACKDROPS.into_iter().chain([[0.97; 3]]) {
                let (before, after) = (tone(c, &default), tone(c, &tinted));
                assert!(luminance(after) >= luminance(before), "{side} pt over {c:?}: lighter than the default");
                assert!(after.iter().all(|&v| v > 0.7), "{side} pt over {c:?}: white frost, got {after:?}");
            }
        }
    }

    #[test]
    fn fully_tinted_never_pulls_toward_mid_grey() {
        // The old slider raised the luminosity fill (toward a mid luminance), whitened the dark
        // face and capped the light face at a 75 % grey: all three read as flat grey.
        for side in [44.0, 180.0, 400.0] {
            let ((dark, light), (dark_t, light_t)) = (faces(side, 0.5), faces(side, 1.0));
            assert!(dark_t.luminance_mix <= dark.luminance_mix && light_t.luminance_mix <= light.luminance_mix);
            assert!(dark_t.lighten <= dark.lighten && light_t.darken <= light.darken);
            assert_eq!(dark_t.darken_to, NEAR_BLACK);
        }
    }

    #[test]
    fn large_light_glass_keeps_dark_text_legible_over_dark_busy_content() {
        // Frosted dark wallpaper: deep blue, near-black and a dim red, at the default slider.
        let (_, light) = faces(260.0, 0.5);
        for c in [[0.05, 0.08, 0.2], [0.02, 0.02, 0.02], [0.35, 0.1, 0.1], [0.25, 0.2, 0.35]] {
            let face = tone(c, &light);
            // Black label: WCAG AAA (7:1).
            let contrast = (luminance(face) + 0.05) / 0.05;
            assert!(contrast >= 7.0, "over {c:?}: {face:?} gives {contrast:.1}:1");
        }
    }

    #[test]
    fn clear_glass_stays_clear_at_the_default() {
        for side in [44.0, 400.0] {
            let m = Material::clear().resolve(side, &Appearance::default()).material;
            assert_eq!((m.light_tone, m.dark_tone), (Material::clear().light_tone, Material::clear().dark_tone));
        }
    }

    #[test]
    fn reduce_transparency_is_opaque_without_greying() {
        let a = Appearance { reduce_transparency: true, dark: true, ..Appearance::default() };
        let m = Material::regular().resolve(400.0, &a).material;
        let c = BACKDROPS[0];
        assert!(luminance(tone(c, &m.dark_tone)) < 0.05 && tone(c, &m.light_tone).iter().all(|&v| v > 0.8));
    }

    #[test]
    fn fully_tinted_keeps_the_bevel_visible() {
        // The refraction must still read: frost grows by at most √2 and refraction is untouched,
        // while rims and the outer edge get stronger.
        let tinted = Appearance { transparency: 1.0, ..Appearance::default() };
        let base = Material::regular().resolve(44.0, &Appearance::default()).material;
        let r = Material::regular().resolve(44.0, &tinted).material;
        assert!(r.frost <= base.frost * 1.42);
        assert_eq!(r.refraction, base.refraction);
        assert!(r.edge > base.edge && r.rim > base.rim);
    }

    #[test]
    fn small_glass_follows_the_scheme_when_not_adapting_to_content() {
        let a = Appearance { adapt_to_content: false, ..Appearance::default() };
        assert_eq!(Material::regular().resolve(44.0, &a).adapt, 0.0);
    }

    #[test]
    fn only_small_glass_flips_with_the_content() {
        let a = Appearance::default();
        let adapt = |side: f32| Material::regular().resolve(side, &a).adapt;
        assert_eq!(adapt(44.0), 1.0, "a button flips");
        assert!(adapt(120.0) > 0.0 && adapt(120.0) < 1.0);
        assert_eq!(adapt(190.0), 0.0, "a menu or panel follows the scheme");
    }

    #[test]
    fn white_oklab_lightness_is_one() {
        assert!((Color::WHITE.to_oklab()[0] - 1.0).abs() < 1e-3);
    }
}
