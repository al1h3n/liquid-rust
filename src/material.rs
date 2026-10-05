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
        [
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        ]
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
        Self { frost: 3.0, specular: 0.2, rim_shade: 0.0, hairline: 0.157, light_tone: tone, dark_tone: tone, ..Self::clear() }
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
}

impl Default for Appearance {
    fn default() -> Self {
        Self { dark: false, transparency: 0.5, reduce_transparency: false, increase_contrast: false, reduce_motion: false }
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
            m.light_tone.lighten += 0.5 * tau;
            m.dark_tone.darken += 0.27 * tau;
            adapt = 1.0 - tau;
        }

        let mut k = (appearance.transparency.clamp(0.0, 1.0) - 0.5) * 2.0;
        if appearance.reduce_transparency {
            k = 1.0;
            m.frost = (m.frost * 3.0).max(20.0);
        }
        m.frost *= (1.1 * k).exp2();
        for tone in [&mut m.light_tone, &mut m.dark_tone] {
            tone.luminance_mix = more_opaque(tone.luminance_mix, 0.45 * k);
            tone.lighten = more_opaque(tone.lighten, 0.35 * k).min(0.9);
            tone.darken = more_opaque(tone.darken, 0.45 * k).min(0.9);
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

    #[test]
    fn white_oklab_lightness_is_one() {
        assert!((Color::WHITE.to_oklab()[0] - 1.0).abs() < 1e-3);
    }
}
