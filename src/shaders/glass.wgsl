// Liquid Glass surface. One instanced quad per glass group; the group's shapes are
// smooth-unioned into one signed distance field, so neighbours fuse like liquid.
//
// Per pixel, in Apple's kit order:
//   backdrop -> refraction (squircle bevel, Snell) + dispersion + frost (pyramid)
//   -> tone mapping (Figma fills, sRGB-encoded space) -> stained-glass tint
//   -> specular light, inner rim shadows, touch glow
//   outside the shape: adaptive drop shadow + darkened hairline edge (iOS 27).
// Output is premultiplied alpha over the content already in the target.

struct Globals {
    viewport: vec4f, // width, height, 1/width, 1/height (px)
    misc: vec4f,     // px per pt, pyramid levels, scheme (1 light, 0 dark), unused
    light: vec4f,    // direction toward the light (x, y), unused, unused
    touch: vec4f,    // position (px), radius (px), intensity
};

struct Shape {
    center_half: vec4f, // center (px), half size (px)
    params: vec4f,      // corner radius, inward shrink, stretch axis (x, y)
    stretch: vec4f,     // scale along axis, scale across axis, corner exponent, unused
    extra: vec4f,       // bevel depth (px), thickness (0 = group's thin entry, 1 = thick), unused, unused
};

struct Group {
    bounds: vec4f,      // quad: min (px), max (px)
    info: vec4u,        // first shape, shape count, unused, unused
    geo: vec4f,         // union radius k (px), presence, press, light/dark adaptivity
    optics: vec4f,      // refraction, unused (depth is per shape), dispersion, frost sigma (px)
    lighting: vec4f,    // splay, specular, rim, rim shade
    edge: vec4f,        // inner hairline, outer edge burn, contrast border, dim
    shadow: vec4f,      // offset y (px), sigma (px), opacity, unused
    light_a: vec4f,     // light tone: lift, luminance, luminance mix, lighten
    light_b: vec4f,     // light tone: darken to, darken, unused, unused
    dark_a: vec4f,      // dark tone
    dark_b: vec4f,
    tint: vec4f,        // OKLab L, a, b, strength
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> shapes: array<Shape>;
// Two entries per group: material for its thinnest member, then for its thickest.
@group(0) @binding(2) var<storage, read> groups: array<Group>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var src: texture_2d<f32>;
@group(0) @binding(5) var pyr: texture_2d<f32>;

const ETA: f32 = 1.5;               // index of refraction of the virtual glass
const KIT_OFFSET: f32 = 40.0;       // Figma inner-shadow offset/spread in the kit (pt)
// Blur sigma of pyramid level 1 in level-0 px: 13-tap kernel + bilinear (var 1.75)
// plus the B-spline reconstruction (var 1.33), so level n is ~0.96 * 2^n px.
const PYRAMID_SIGMA0: f32 = 1.9;

struct Varyings {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(flat) group: u32,
};

@vertex
fn vs_glass(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Varyings {
    let b = groups[2u * ii].bounds;
    let corner = vec2f(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let px = mix(b.xy, b.zw, corner);
    var out: Varyings;
    out.pos = vec4f(px.x * globals.viewport.z * 2.0 - 1.0, 1.0 - px.y * globals.viewport.w * 2.0, 0.0, 1.0);
    out.group = ii;
    return out;
}

// ---------------------------------------------------------------- distance fields

// Rounded box distance and outward normal. Corners are superellipses with exponent `n`
// (2 = circular, ~2.79 = Apple's continuous/squircle corner, see smooth_corner() in
// scene.rs); `r` is the corner's extent along each edge. The superellipse value is
// divided by its gradient length, which keeps it a true distance near the edge where
// antialiasing and the bevel need it. Inside the straight part the normal is softened
// across the diagonal (`soft` px) so the bevel has no crease at corners, the same job
// as CASDFElementLayer's "gradientOvalization".
fn sdg_rrect(p: vec2f, half: vec2f, r: f32, n: f32, soft: f32) -> vec3f {
    let sg = select(vec2f(-1.0), vec2f(1.0), p >= vec2f(0.0));
    let w = abs(p) - (half - vec2f(r));
    let g = max(w.x, w.y);
    if g > 0.0 {
        let q = max(w, vec2f(1e-7));
        if n == 2.0 {
            let l = length(q);
            return vec3f(l - r, sg * q / l);
        }
        let l = pow(pow(q.x, n) + pow(q.y, n), 1.0 / n);
        let grad = pow(q / l, vec2f(n - 1.0));
        let gl = length(grad);
        return vec3f((l - r) / gl, sg * grad / gl);
    }
    let e = exp((w - vec2f(g)) / max(soft, 1e-3));
    return vec3f(g - r, sg * normalize(e));
}

fn sdg_shape(p: vec2f, s: Shape) -> vec3f {
    let soft = s.extra.x * 0.35;
    let axis = s.params.zw;
    let across = vec2f(-axis.y, axis.x);
    let rel = p - s.center_half.xy;
    let u = dot(rel, axis) / s.stretch.x;
    let v = dot(rel, across) / s.stretch.y;
    let dg = sdg_rrect(u * axis + v * across, s.center_half.zw, s.params.x, s.stretch.z, soft);
    let m = min(s.stretch.x, s.stretch.y);
    let grad = (dot(dg.yz, axis) / s.stretch.x * axis + dot(dg.yz, across) / s.stretch.y * across) * m;
    return vec3f(dg.x * m + s.params.y, grad);
}

// Quadratic smooth minimum (Inigo Quilez): the blended distance, and how much of `b`
// to mix into anything carried along with it.
fn smin_w(a: f32, b: f32, k: f32) -> vec2f {
    if k <= 0.0 {
        return vec2f(min(a, b), select(1.0, 0.0, a < b));
    }
    let h = max(k - abs(a - b), 0.0);
    let n = 0.5 * h / k;
    return vec2f(min(a, b) - 0.25 * h * h / k, select(1.0 - n, n, a < b));
}

fn sdg_group(p: vec2f, grp: Group) -> vec3f {
    var acc = sdg_shape(p, shapes[grp.info.x]);
    for (var i = 1u; i < grp.info.y; i++) {
        let b = sdg_shape(p, shapes[grp.info.x + i]);
        let w = smin_w(acc.x, b.x, grp.geo.x);
        acc = vec3f(w.x, mix(acc.yz, b.yz, w.y));
    }
    return acc;
}

struct Field {
    dg: vec3f,   // distance and normal, as sdg_group
    attrs: vec4f, // per-shape bevel depth (px), thickness, centre (px), blended as the shapes fuse
};

fn field_group(p: vec2f, grp: Group) -> Field {
    let first = shapes[grp.info.x];
    var dg = sdg_shape(p, first);
    var attrs = vec4f(first.extra.xy, first.center_half.xy);
    for (var i = 1u; i < grp.info.y; i++) {
        let s = shapes[grp.info.x + i];
        let b = sdg_shape(p, s);
        let w = smin_w(dg.x, b.x, grp.geo.x);
        dg = vec3f(w.x, mix(dg.yz, b.yz, w.y));
        attrs = mix(attrs, vec4f(s.extra.xy, s.center_half.xy), w.y);
    }
    return Field(dg, attrs);
}

fn mix_group(a: Group, b: Group, t: f32) -> Group {
    return Group(a.bounds, a.info, mix(a.geo, b.geo, t), mix(a.optics, b.optics, t), mix(a.lighting, b.lighting, t),
        mix(a.edge, b.edge, t), mix(a.shadow, b.shadow, t), mix(a.light_a, b.light_a, t), mix(a.light_b, b.light_b, t),
        mix(a.dark_a, b.dark_a, t), mix(a.dark_b, b.dark_b, t), mix(a.tint, b.tint, t));
}

// ---------------------------------------------------------------- maths helpers

fn erf_approx(x: f32) -> f32 {
    let c = clamp(x, -4.0, 4.0);
    return tanh(c * (1.1283792 + 0.1003 * c * c));
}

// Fraction of a Gaussian-blurred half-plane that lies "outside" at signed distance x.
fn phi(x: f32, sigma: f32) -> f32 {
    return 0.5 + 0.5 * erf_approx(x / (max(sigma, 1e-3) * 1.4142135));
}

fn coverage(d: f32) -> f32 {
    return clamp(0.5 - d, 0.0, 1.0);
}

fn encode(c: vec3f) -> vec3f {
    return pow(max(c, vec3f(0.0)), vec3f(1.0 / 2.2));
}

fn decode(c: vec3f) -> vec3f {
    return pow(max(c, vec3f(0.0)), vec3f(2.2));
}

fn blend_lum(c: vec3f) -> f32 {
    return dot(c, vec3f(0.3, 0.59, 0.11));
}

fn luminance(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

// W3C SetLum with ClipColor: Figma's LUMINOSITY blend.
fn set_lum(c: vec3f, l: f32) -> vec3f {
    var x = c + vec3f(l - blend_lum(c));
    let lx = blend_lum(x);
    let lo = min(min(x.r, x.g), x.b);
    let hi = max(max(x.r, x.g), x.b);
    if lo < 0.0 {
        x = vec3f(lx) + (x - vec3f(lx)) * lx / max(lx - lo, 1e-5);
    }
    if hi > 1.0 {
        x = vec3f(lx) + (x - vec3f(lx)) * (1.0 - lx) / max(hi - lx, 1e-5);
    }
    return x;
}

fn tone(c: vec3f, a: vec4f, b: vec4f) -> vec3f {
    var x = min(c + vec3f(a.x), vec3f(1.0));
    x = mix(x, set_lum(x, a.y), a.z);
    x = mix(x, vec3f(1.0), a.w);
    x = mix(x, min(x, vec3f(b.x)), b.y);
    return x;
}

fn oklab(c: vec3f) -> vec3f {
    let lms = pow(max(vec3f(
        dot(c, vec3f(0.4122214708, 0.5363325363, 0.0514459929)),
        dot(c, vec3f(0.2119034982, 0.6806995451, 0.1073969566)),
        dot(c, vec3f(0.0883024619, 0.2817188376, 0.6299787005)),
    ), vec3f(0.0)), vec3f(1.0 / 3.0));
    return vec3f(
        dot(lms, vec3f(0.2104542553, 0.7936177850, -0.0040720468)),
        dot(lms, vec3f(1.9779984951, -2.4285922050, 0.4505937099)),
        dot(lms, vec3f(0.0259040371, 0.7827717662, -0.8086757660)),
    );
}

fn oklab_to_linear(lab: vec3f) -> vec3f {
    let l = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    let m = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    let s = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    let lms = vec3f(l * l * l, m * m * m, s * s * s);
    return vec3f(
        dot(lms, vec3f(4.0767416621, -3.3077115913, 0.2309699292)),
        dot(lms, vec3f(-1.2684380046, 2.6097574011, -0.3413193965)),
        dot(lms, vec3f(-0.0041960863, -0.7034186147, 1.7076147010)),
    );
}

// ---------------------------------------------------------------- backdrop sampling

// Cubic B-spline filtered fetch from one pyramid mip using 4 bilinear taps.
fn bspline(uv: vec2f, mip: f32) -> vec3f {
    let size = vec2f(textureDimensions(pyr, u32(mip)));
    let t = uv * size - 0.5;
    let i = floor(t);
    let f = t - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - f) * (1.0 - f) * (1.0 - f) / 6.0;
    let w1 = (3.0 * f3 - 6.0 * f2 + 4.0) / 6.0;
    let w2 = (-3.0 * f3 + 3.0 * f2 + 3.0 * f + 1.0) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let h0 = (i - 0.5 + w1 / g0) / size;
    let h1 = (i + 1.5 + w3 / g1) / size;
    let a = textureSampleLevel(pyr, samp, vec2f(h0.x, h0.y), mip).rgb;
    let b = textureSampleLevel(pyr, samp, vec2f(h1.x, h0.y), mip).rgb;
    let c = textureSampleLevel(pyr, samp, vec2f(h0.x, h1.y), mip).rgb;
    let d = textureSampleLevel(pyr, samp, vec2f(h1.x, h1.y), mip).rgb;
    return g0.y * (g0.x * a + g1.x * b) + g1.y * (g0.x * c + g1.x * d);
}

// Backdrop at `q` (px) blurred by `sigma` (px): sharp source, then pyramid levels with
// continuous interpolation between them.
fn frost(q: vec2f, sigma: f32) -> vec3f {
    let uv = q * globals.viewport.zw;
    let level = log2(max(sigma, 1e-4) / PYRAMID_SIGMA0) + 1.0;
    if level <= 0.0 {
        return textureSampleLevel(src, samp, uv, 0.0).rgb;
    }
    let top = globals.misc.y;
    if level < 1.0 {
        return mix(textureSampleLevel(src, samp, uv, 0.0).rgb, bspline(uv, 0.0), level);
    }
    let l = min(level, top);
    let lo = min(floor(l), top - 1.0);
    return mix(bspline(uv, lo - 1.0), bspline(uv, lo), clamp(l - lo, 0.0, 1.0));
}

// Squircle bevel h(t) = (1 - (1 - t)^4)^(1/4): Apple's soft flat-to-curve profile.
fn bevel_slope(t: f32) -> f32 {
    let u = 1.0 - t;
    let u3 = u * u * u;
    return u3 * pow(max(1.0 - u3 * u, 1e-5), -0.75);
}

// Lateral travel (in units of glass thickness) of a vertical ray refracted by a
// surface tilted by atan(slope).
fn refract_offset(slope: f32, eta: f32) -> f32 {
    let s = slope / sqrt(1.0 + slope * slope);
    let theta = asin(s);
    return tan(theta - asin(s / eta));
}

// Inner shadow in Figma's sense: present where the shape is not covered by itself
// shifted by `offset` and grown by `grow`, blurred by `sigma`.
fn inner_shadow(p: vec2f, grp: Group, offset: vec2f, grow: f32, sigma: f32) -> f32 {
    let hole = sdg_group(p - offset, grp).x - grow;
    return phi(hole, sigma);
}

// ---------------------------------------------------------------- fragment

@fragment
fn fs_glass(v: Varyings) -> @location(0) vec4f {
    let p = v.pos.xy;
    let pt = globals.misc.x;
    let thin = groups[2u * v.group];
    let field = field_group(p, thin);
    let attrs = field.attrs;
    let grp = mix_group(thin, groups[2u * v.group + 1u], attrs.y);
    let depth = max(attrs.x, 1.0);

    let dg = field.dg;
    let glen = max(length(dg.yz), 1e-3);
    let d = dg.x / glen;
    let n = dg.yz / glen;
    let cov = coverage(d);
    let center_uv = attrs.zw * globals.viewport.zw;

    // Outside: drop shadow and the darkened hairline edge.
    var dark = 0.0;
    if cov < 1.0 {
        let stats = textureSampleLevel(pyr, samp, center_uv, max(globals.misc.y - 4.0, 0.0));
        let mean = luminance(stats.rgb);
        let detail = sqrt(max(stats.a - mean * mean, 0.0));
        // Apple: shadows grow over busy content (text), fade over flat light content.
        let adapt = mix(0.7, 1.6, smoothstep(0.02, 0.12, detail)) * mix(1.0, 0.65, smoothstep(0.5, 0.85, mean));
        var shadow = 0.0;
        if grp.shadow.z > 0.0 {
            let shadow_d = sdg_group(p - vec2f(0.0, grp.shadow.x), grp).x;
            shadow = grp.shadow.z * adapt * (1.0 - phi(shadow_d, grp.shadow.y));
        }

        var edge_dark = 0.0;
        if grp.edge.y > 0.0 && d < 2.0 * pt {
            let ring = clamp(coverage(d - 0.5 * pt) - cov, 0.0, 1.0);
            let side_l = coverage(sdg_group(p + vec2f(1.25 * pt, 0.0), grp).x + 0.75 * pt);
            let side_r = coverage(sdg_group(p - vec2f(1.25 * pt, 0.0), grp).x + 0.75 * pt);
            let burn = grp.edge.y * (ring + 0.92 * max(side_l, side_r) * (1.0 - cov));
            let under = encode(textureLoad(src, vec2i(p), 0).rgb);
            let burnt = max(under - vec3f(burn), vec3f(0.0));
            edge_dark = 1.0 - luminance(decode(burnt)) / max(luminance(decode(under)), 1e-4);
        }
        dark = 1.0 - (1.0 - clamp(shadow, 0.0, 1.0)) * (1.0 - clamp(edge_dark, 0.0, 1.0));
        dark *= 1.0 - cov;
    }
    if cov <= 0.0 {
        return vec4f(0.0, 0.0, 0.0, dark);
    }

    // Refraction through the squircle bevel, per channel for dispersion.
    let t = clamp(-d / depth, 0.0, 1.0);
    let sigma = grp.optics.w;
    var col: vec3f;
    if t < 1.0 && grp.optics.x > 0.0 {
        let slope = bevel_slope(t);
        let reach = grp.optics.x * depth;
        let disp = grp.optics.z * 0.25;
        let off_g = reach * refract_offset(slope, ETA);
        if disp > 0.0 {
            let off_r = reach * refract_offset(slope, ETA - disp);
            let off_b = reach * refract_offset(slope, ETA + disp);
            col = vec3f(frost(p - n * off_r, sigma).r, frost(p - n * off_g, sigma).g, frost(p - n * off_b, sigma).b);
        } else {
            col = frost(p - n * off_g, sigma);
        }
    } else {
        col = frost(p, sigma);
    }
    col *= 1.0 - grp.edge.w;

    // Tone mapping: light and dark faces, chosen by scheme or by the backdrop.
    var g = encode(col);
    let top = max(globals.misc.y - 1.0, 0.0);
    let bg = luminance(encode(textureSampleLevel(pyr, samp, center_uv, top).rgb));
    // Biased toward light: small glass only turns dark over genuinely dark content.
    let lightness = mix(globals.misc.z, smoothstep(0.26, 0.46, bg), grp.geo.w);
    g = mix(tone(g, grp.dark_a, grp.dark_b), tone(g, grp.light_a, grp.light_b), lightness);

    // Stained-glass tint: keep hue and chroma, let lightness follow the backdrop.
    if grp.tint.w > 0.0 {
        let lab = oklab(decode(g));
        let l = clamp(grp.tint.x + (lab.x - 0.65) * 0.6, 0.15, 0.97);
        g = mix(g, encode(oklab_to_linear(vec3f(l, grp.tint.y, grp.tint.z))), grp.tint.w);
    }

    // Light: a crisp line just inside the edge, brightest facing the light, weaker on
    // the opposite edge (Apple lights glass from two opposite sides); splay spreads it
    // around to the flanks. The line is a brightened, saturated copy of what is behind
    // (CoreAnimation's vibrantColorMatrix), so it picks up the backdrop's colour.
    let to_light = globals.light.xy;
    let facing = dot(n, to_light);
    let splay = grp.lighting.x;
    let lit = splay + (1.0 - splay) * pow(max(facing, 0.0), 2.0);
    let back = splay + (0.4 - splay) * pow(max(-facing, 0.0), 2.0);
    let dir = select(back, lit, facing > 0.0);
    let line = exp(d / (0.25 * pt * (1.0 + splay)));
    let vivid = vec3f(blend_lum(g)) + (g - vec3f(blend_lum(g))) * 2.2;
    let highlight = vivid * 1.45 + vec3f(0.05);
    g = mix(g, highlight, clamp(line * dir * grp.lighting.y * 2.5, 0.0, 1.0));

    // Inner shadows from the kit, skipped deep inside where they vanish.
    let rim_sigma = 5.0 * pt;
    if -d < 45.0 * pt {
        let o = KIT_OFFSET * pt;
        let away = -to_light * o; // shadow offset that lights the edge facing the light
        if grp.lighting.z > 0.0 {
            let rims = inner_shadow(p, grp, away, o, rim_sigma) + inner_shadow(p, grp, -away, o, rim_sigma);
            g += vec3f(grp.lighting.z * rims);
        }
        if grp.lighting.w > 0.0 {
            g -= vec3f(grp.lighting.w * inner_shadow(p, grp, away, o, 15.0 * pt));
        }
        if grp.edge.x > 0.0 {
            let h = -to_light * 1.25 * pt;
            let lines = inner_shadow(p, grp, h, 0.0, 0.125 * pt) + inner_shadow(p, grp, -h, 0.0, 0.125 * pt);
            g += vec3f(grp.edge.x * lines);
        }
    }

    // Touch: the glass lights up from the finger; the light spills onto nearby glass.
    let tp = p - globals.touch.xy;
    let glow = globals.touch.w * exp(-dot(tp, tp) / max(globals.touch.z * globals.touch.z, 1.0));
    // Screen blend: light only fills the headroom that is left, so over light content
    // the glass brightens a little instead of clipping to an opaque white disc, while
    // over dark content it keeps nearly the full lift.
    g += vec3f(0.22 * glow + 0.03 * grp.geo.z) * (1.0 - clamp(g, vec3f(0.0), vec3f(1.0)));

    // Increase Contrast: a border in the label colour.
    if grp.edge.z > 0.0 {
        let border = clamp(1.0 - (-d) / pt, 0.0, 1.0);
        g = mix(g, vec3f(1.0 - step(0.5, lightness)), border * grp.edge.z);
    }

    // Materialize needs no alpha: the CPU scales every effect by presence, so at 0 the
    // glass is exactly the untouched backdrop.
    return vec4f(decode(g) * cov, cov + dark);
}
