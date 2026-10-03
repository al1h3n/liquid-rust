// Backdrop pyramid: each level is a 2x downsample with Jimenez's 13-tap filter
// (Call of Duty: Advanced Warfare). Frost samples it with a B-spline at a continuous
// level, so one pyramid serves every blur radius on screen.
//
// Alpha carries E[luminance^2] so glass can read local contrast (variance) for its
// adaptive shadow; colour alpha of the backdrop is ignored (backdrops are opaque).

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var src: texture_2d<f32>;

struct Varyings {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> Varyings {
    let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Varyings;
    out.pos = vec4f(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2f(xy.x, 1.0 - xy.y);
    return out;
}

fn luminance(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

fn tap(uv: vec2f, first: bool) -> vec4f {
    let c = textureSampleLevel(src, samp, uv, 0.0);
    if first {
        let l = luminance(c.rgb);
        return vec4f(c.rgb, l * l);
    }
    return c;
}

fn down13(uv: vec2f, first: bool) -> vec4f {
    let t = 1.0 / vec2f(textureDimensions(src));
    let a = tap(uv + t * vec2f(-2.0, -2.0), first);
    let b = tap(uv + t * vec2f(0.0, -2.0), first);
    let c = tap(uv + t * vec2f(2.0, -2.0), first);
    let d = tap(uv + t * vec2f(-1.0, -1.0), first);
    let e = tap(uv + t * vec2f(1.0, -1.0), first);
    let f = tap(uv + t * vec2f(-2.0, 0.0), first);
    let g = tap(uv, first);
    let h = tap(uv + t * vec2f(2.0, 0.0), first);
    let i = tap(uv + t * vec2f(-1.0, 1.0), first);
    let j = tap(uv + t * vec2f(1.0, 1.0), first);
    let k = tap(uv + t * vec2f(-2.0, 2.0), first);
    let l = tap(uv + t * vec2f(0.0, 2.0), first);
    let m = tap(uv + t * vec2f(2.0, 2.0), first);
    return (d + e + i + j) * 0.125 + g * 0.125 + (b + f + h + l) * 0.0625 + (a + c + k + m) * 0.03125;
}

@fragment
fn fs_down_first(v: Varyings) -> @location(0) vec4f {
    return down13(v.uv, true);
}

@fragment
fn fs_down(v: Varyings) -> @location(0) vec4f {
    return down13(v.uv, false);
}

// Exact pixel copy, used to put the content under the glass and to seed layer backdrops.
@fragment
fn fs_blit(v: Varyings) -> @location(0) vec4f {
    return textureLoad(src, vec2i(v.pos.xy), 0);
}
