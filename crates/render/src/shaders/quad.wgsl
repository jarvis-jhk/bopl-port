// Instanced quads for every item: sprites, the original's custom shaders
// (reimplemented from their behaviour, see docs/platform.md "Shaders"),
// analytic shapes and SDF glyphs.
//
// Gamma-space pipeline: texels and colours are used as stored; every kind
// returns premultiplied colour except GRADIENT (straight colour for the
// inverse-alpha blend).
//
// Texture coordinates: `in.uv` is in PNG convention (v = 0 top row).
// `unity_uv` flips v so that v = 0 is the bottom row, like Unity's texture
// space; render targets use the same convention (`sample_u`).

struct Globals {
    view_proj: mat4x4<f32>,
    // Unity _Time: (t/20, t, 2t, 3t), _SinTime: sin(t/8, t/4, t/2, t), _CosTime.
    time: vec4<f32>,
    sin_time: vec4<f32>,
    cos_time: vec4<f32>,
    // x: _Timestop01, y: _ProjectionParams.x (1), z/w: target size in pixels.
    misc: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var tex0: texture_2d<f32>;
@group(1) @binding(1) var samp0: sampler;
@group(2) @binding(0) var tex1: texture_2d<f32>;
@group(2) @binding(1) var samp1: sampler;
@group(2) @binding(2) var tex2: texture_2d<f32>;
@group(2) @binding(3) var samp2: sampler;
@group(2) @binding(4) var tex3: texture_2d<f32>;
@group(2) @binding(5) var samp3: sampler;

const K_SPRITE: u32 = 0u;
const K_ALPHA_TINT: u32 = 1u;
const K_CHANNEL: u32 = 2u;
const K_GRADIENT: u32 = 3u;
const K_SLIME: u32 = 4u;
const K_WATER: u32 = 5u;
const K_DISTORT: u32 = 6u;
const K_AURORA: u32 = 7u;
const K_ELLIPSE: u32 = 8u;
const K_RING: u32 = 9u;
const K_RRECT: u32 = 10u;
const K_SDF: u32 = 11u;
const K_MASK: u32 = 12u;
const K_COPY: u32 = 13u;
const K_BLACKHOLE: u32 = 14u;
const K_GRAB_INVERT: u32 = 15u;
const K_DISTORT_SPRITE: u32 = 16u;
const K_SMEAR: u32 = 17u;
const K_ONLY_WHITE: u32 = 18u;
const K_EXPANDING: u32 = 19u;
const K_WHITE_TO_COL: u32 = 20u;
const K_WALL_MASKED: u32 = 21u;
const K_SINGLE_CHANNEL: u32 = 22u;
const K_CHANNELS3: u32 = 23u;
const K_RPLATFORM: u32 = 24u;
const K_LIGHTNING: u32 = 25u;
const K_BEAM: u32 = 26u;
const K_CLONE: u32 = 27u;
const K_PARTICLE_PREMUL: u32 = 28u;
const K_PARTICLE_BLEND: u32 = 29u;
const K_INDICATOR: u32 = 30u;
const TAU: f32 = 6.28318530718;

struct Instance {
    // World position of local corner (0,0); w unused.
    @location(0) origin: vec4<f32>,
    // World vectors of the quad's local x and y edges; w = local size.
    @location(1) axis_x: vec4<f32>,
    @location(2) axis_y: vec4<f32>,
    // u0, v0, u1, v1: PNG-convention texture coords at local (0,0), (1,1).
    @location(3) uv: vec4<f32>,
    // Vertex colour, straight alpha.
    @location(4) color: vec4<f32>,
    @location(5) p0: vec4<f32>,
    @location(6) p1: vec4<f32>,
    @location(7) p2: vec4<f32>,
    @location(8) p3: vec4<f32>,
    @location(9) p4: vec4<f32>,
    @location(10) p5: vec4<f32>,
    @location(11) kind: vec4<u32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Position in the quad, local units, centred.
    @location(1) local: vec2<f32>,
    @location(2) color: vec4<f32>,
    // Water: screen position (0..1, y up) of the point _ReflH above.
    @location(3) screen: vec2<f32>,
    @location(4) @interpolate(flat) p0: vec4<f32>,
    @location(5) @interpolate(flat) p1: vec4<f32>,
    @location(6) @interpolate(flat) p2: vec4<f32>,
    @location(7) @interpolate(flat) p3: vec4<f32>,
    @location(8) @interpolate(flat) p4: vec4<f32>,
    @location(9) @interpolate(flat) p5: vec4<f32>,
    @location(10) @interpolate(flat) size: vec2<f32>,
    @location(11) @interpolate(flat) kind: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    let c = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    var world = inst.origin.xyz + inst.axis_x.xyz * c.x + inst.axis_y.xyz * c.y;
    if (inst.kind.y == 1u) {
        // Free-form quad: bilinear between the four corners.
        let c00 = inst.origin.xyz;
        let c10 = c00 + inst.axis_x.xyz;
        let c01 = c00 + inst.axis_y.xyz;
        let c11 = inst.p5.xyz;
        world = mix(mix(c00, c10, c.x), mix(c01, c11, c.x), c.y);
    }
    var out: VsOut;
    out.pos = g.view_proj * vec4<f32>(world, 1.0);
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    let size = vec2<f32>(inst.axis_x.w, inst.axis_y.w);
    out.local = (c - vec2<f32>(0.5)) * size;
    out.color = inst.color;
    var lift = 0.0;
    if (inst.kind.x == K_WATER) {
        lift = inst.p1.w;
    }
    // Like the original: no perspective divide (orthographic cameras).
    let s = g.view_proj * vec4<f32>(world.x, world.y + lift, world.z, 1.0);
    out.screen = (s.xy + vec2<f32>(1.0)) * 0.5;
    out.p0 = inst.p0;
    out.p1 = inst.p1;
    out.p2 = inst.p2;
    out.p3 = inst.p3;
    out.p4 = inst.p4;
    out.p5 = inst.p5;
    out.size = size;
    out.kind = inst.kind.x;
    return out;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

// Samples with Unity-convention coordinates (v = 0 at the bottom row).
fn sample_u(t: texture_2d<f32>, s: sampler, uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t, s, vec2<f32>(uv.x, 1.0 - uv.y), 0.0);
}

fn coverage(d: f32, aa: f32) -> f32 {
    return clamp(0.5 - d / aa, 0.0, 1.0);
}

fn premul(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

// Unlit/SlimeTrailedMovable: the brightest of the trail texel and its four
// diagonal neighbours (trail edges look solid), tinted by the platform's
// green channel, painted over the sprite by the centre texel's alpha.
fn slime(in: VsOut, uvu: vec2<f32>, main: vec4<f32>) -> vec4<f32> {
    let flip = in.p0.xy;
    let texel = in.p0.zw;
    let tuv = uvu + flip - 2.0 * flip * uvu;
    let centre = sample_u(tex1, samp1, tuv);
    let pp = sample_u(tex1, samp1, tuv + texel);
    let mm = sample_u(tex1, samp1, tuv - texel);
    let mp = sample_u(tex1, samp1, tuv + vec2<f32>(-texel.x, texel.y));
    let pm = sample_u(tex1, samp1, tuv + vec2<f32>(texel.x, -texel.y));
    let l_pp = luma(pp.rgb);
    let l_mm = luma(mm.rgb);
    var a = pp.rgb;
    if (l_mm >= l_pp) {
        a = mm.rgb;
    }
    let la = max(l_pp, l_mm);
    let l_pm = luma(pm.rgb);
    let l_mp = luma(mp.rgb);
    var b = mp.rgb;
    if (l_pm >= l_mp) {
        b = pm.rgb;
    }
    let lb = max(l_pm, l_mp);
    var m = a;
    if (la < lb) {
        m = b;
    }
    var t = centre.rgb;
    if (luma(m) >= luma(centre.rgb)) {
        t = m;
    }
    t = t * sqrt(main.g);
    let rgb = (t * centre.a + main.rgb * (1.0 - centre.a)) * in.color.rgb;
    let alpha = main.a * in.color.a;
    return vec4<f32>(rgb * alpha, alpha);
}

// Custom/ReflectiveWaterWithDistortion. Below the horizon (screen height of
// _WorldHorizonHeight) it shows the mirrored reflection render texture over
// the water colour, above it the background render texture, both distorted
// by the wave render texture, mixed with a vertical gradient and wave noise.
fn water(in: VsOut, uv0: vec2<f32>) -> vec4<f32> {
    let alpha = in.p0.x;
    let distortion = in.p0.y;
    let horizon = in.p0.z;
    let surface_gradient = in.p0.w;
    let extra_noise = in.p1.x;
    let gradient_height = in.p1.y;
    let horizon_gradient_height = in.p1.z;
    let hc = g.view_proj[1].yw * horizon + g.view_proj[3].yw;
    let hn = (hc.x / hc.y + 1.0) * 0.5;
    var ry = in.screen.y;
    var hb = hn;
    if (g.misc.y > 0.0) {
        ry = 1.0 - in.screen.y;
        hb = 1.0 - hn;
    }
    let k = clamp((ry - hb) * surface_gradient, 0.0, 1.0);
    let below = k / max(k, 1e-9);
    let w = sample_u(tex1, samp1, uv0);
    let d = w.yx * distortion;
    let f = 1.0 - uv0.y * uv0.y;
    let edge = clamp(1.0 - (in.screen.x - 0.9) * 10.0, 0.0, 1.0);
    let st = vec2<f32>(d.y * f * edge + in.screen.x, d.x * f + ry);
    let r = sample_u(tex2, samp2, st);
    let bg = sample_u(tex3, samp3, st);
    let ra = below * r.a;
    let ia = 1.0 - r.a * below;
    let wc = k * r.rgb + (1.0 - k) * in.p4.rgb;
    let gt = clamp(uv0.y + gradient_height, 0.0, 1.0);
    let grad = gt * in.p2.rgb + (1.0 - gt) * in.p3.rgb;
    let base = (1.0 - alpha) * grad;
    let noise = w.x * grad * extra_noise;
    let c1 = wc * alpha + base;
    let c2 = bg.rgb * alpha + base;
    let col = c2 * ia + c1 * ra;
    let hz = uv0.y + horizon_gradient_height;
    let hz3 = hz * hz * hz;
    let q = 1.0 - hz3 * hz3;
    return vec4<f32>(noise * q + col, 1.0);
}

// Custom/WaterDistortion: the wave texture, tiled `_SubDivide` times and
// offset by animated Perlin noise, faded out towards the top.
fn distortion(in: VsOut, uv: vec2<f32>) -> vec4<f32> {
    let strength = in.p0.x;
    let subdivide = in.p0.y;
    let sdn = in.p0.z;
    let ns = in.p1.xyz;
    let s3 = in.p2.xyz;
    let uvc = in.p3;
    let t = g.time.x * g.misc.x;
    let ts = vec4<f32>(ns.x, ns.y, ns.y, ns.z) * t;
    let n_a = sample_u(tex1, samp1, vec2<f32>(uv.x * sdn - ts.y, uv.y * sdn - ts.z));
    let n_b = sample_u(tex1, samp1, vec2<f32>(uv.x * sdn + ts.x, uv.y * sdn - ts.x));
    let n_c = sample_u(tex1, samp1, vec2<f32>(uv.x * sdn + ts.w, uv.y * sdn - ts.w));
    let a1 = n_a.y * strength * s3.y;
    let x3 = n_b.x * strength * s3.x - a1;
    let off = -(n_c.z * strength) * s3.z + x3;
    let sn = g.sin_time.x * g.misc.x;
    let cs = g.cos_time.x * g.misc.x;
    let cuv = uv - uvc.xy;
    let ruv = vec2<f32>(cs * cuv.x + sn * cuv.y, -sn * cuv.x + cs * cuv.y);
    let n_d = sample_u(tex1, samp1, ruv);
    let o = n_d.xy * uvc.z + n_d.xy * uvc.w + vec2<f32>(off);
    let m = sample_u(tex0, samp0, uv * subdivide + o);
    let a = m.a * in.color.a * (1.0 - uv.y);
    return vec4<f32>(vec3<f32>(a * m.r), a);
}

// Unlit/aurora: bands from the sprite's own texture scrolled along the mask's
// green channel, wobbled by noise; blue in the mask cuts it out.
fn aurora(in: VsOut, uv: vec2<f32>) -> vec4<f32> {
    let height = in.p0.x;
    let blue = in.p0.y;
    let wobble = in.p0.z;
    let nz = sample_u(tex1, samp1, vec2<f32>(g.time.x * 0.5) + uv);
    let mk = sample_u(tex2, samp2, uv + vec2<f32>(0.0, nz.x * wobble));
    let s1 = sample_u(tex0, samp0, vec2<f32>(g.time.x * 0.2 + mk.y, 0.3));
    let s2 = sample_u(tex0, samp0, vec2<f32>(-g.time.x * 0.19 + mk.y, 0.7));
    let v = s1.x * s2.x * height;
    let rgb = vec3<f32>(v * mk.x + mk.x) * in.color.rgb;
    let cut = -blue * mk.z + (v * mk.x + mk.x);
    let a = cut * in.color.a * in.color.a;
    return vec4<f32>(rgb * a, a);
}


// Samples a render texture at a screen position (0..1, y up).
fn sample_screen(t: texture_2d<f32>, s: sampler, p: vec2<f32>) -> vec4<f32> {
    return sample_u(t, s, p);
}

// Samples the main texture at Unity texture coordinates (v = 0 bottom).
fn main_u(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(tex0, samp0, vec2<f32>(uv.x, 1.0 - uv.y), 0.0);
}

// Custom/BlackHole2: the grabbed screen pulled inwards by `k²`, a glow in
// the vertex colour and the anti-aliased hole (8 steps over a few pixels).
fn black_hole(in: VsOut, uvu: vec2<f32>) -> vec4<f32> {
    let distortion = in.p0.x;
    let prox = in.p0.y;
    let scale = max(in.p0.z, 1e-4);
    let hb = in.p0.w;
    let d = (uvu - vec2<f32>(0.5)) * prox;
    let r = length(d);
    let k = max(1.0 - r, 0.0);
    let k2 = k * k;
    var dir = vec2<f32>(0.0);
    if (r > 1e-6) {
        dir = -d / r;
    }
    let off = dir * k2 * distortion;
    let grab = sample_screen(tex1, samp1, in.screen + vec2<f32>(-off.x, off.y));
    var c = grab * (1.0 - k2) + in.color * k2;
    c = vec4<f32>(c.rgb + 3.0 * k2 * k2 * in.color.rgb, c.a);
    let dist = length(vec2<f32>(0.5) - uvu);
    let s = 50.0 / (max(g.misc.z, 1.0) * scale);
    let base = 0.05 - 7.0 * s;
    var h = select(0.0, 0.125, dist < 0.05) + select(0.0, 1.0, dist < base);
    for (var i = 1; i <= 6; i = i + 1) {
        h = h + select(0.0, 0.125, dist < base + f32(i) * s);
    }
    let w = min(h, 1.0);
    let o = c * (1.0 - w) + vec4<f32>(vec3<f32>(w * hb), w);
    return vec4<f32>(o.rgb * o.a, o.a);
}

// Custom/Distortion: the sprite sampled through two scrolling, rotating
// noise lookups, strongest near `uv_center`.
fn distort_sprite(in: VsOut, uvu: vec2<f32>) -> vec4<f32> {
    let tt = g.time.y * in.p0.x;
    let uvc = in.p0.yz;
    let radius = in.p0.w;
    let cc = in.p1.x;
    let cw = in.p1.y;
    let strength = in.p1.z;
    let c1 = cos(tt);
    let s1 = sin(tt);
    let sw = sin(tt * 0.5) * 0.5 + 1.5;
    let q = uvu - uvc;
    let a = vec2<f32>(dot(vec2<f32>(-s1, s1), q), dot(vec2<f32>(-s1, -c1), q)) * sw * cw
        + vec2<f32>(0.3, -0.5);
    let n1 = (textureSampleLevel(tex1, samp1, a, 0.0).xy - vec2<f32>(0.5)) * strength;
    let dc = uvc - uvu;
    var fall = 1.0 - (dot(dc, dc) + 0.15) * radius;
    fall = fall * fall;
    fall = fall * fall;
    let uv2 = n1 * fall * 2.0 + uvu;
    let sw2 = g.cos_time.y * 0.5 + 1.5;
    let b = vec2<f32>(dot(vec2<f32>(c1, s1), q), dot(vec2<f32>(-s1, c1), q)) * sw2 * cc;
    let n2 = (textureSampleLevel(tex1, samp1, b, 0.0).xy - vec2<f32>(0.5)) * strength * fall;
    let m = main_u(n2 * 2.0 + uv2);
    let am = 2.0 * m.a * fall;
    let k = am * in.color.a;
    return vec4<f32>(in.color.rgb * k, am * k);
}

// Custom/SmearSprite.
fn smear_wrap(y: f32, o: vec4<f32>) -> f32 {
    return y - floor((y - o.y) / o.w) * o.y;
}

fn smear(in: VsOut, uvu: vec2<f32>) -> vec4<f32> {
    let o = in.p2;
    let sc = in.p3.xy;
    let p = uvu * sc;
    let t0 = main_u(vec2<f32>(p.x, smear_wrap(p.y, o)));
    let t1 = main_u(vec2<f32>(p.x, smear_wrap(p.y + o.w * 0.33, o)));
    let t2 = main_u(vec2<f32>(p.x, smear_wrap(p.y + o.w * 0.66, o)));
    let t = max(max(t0, t1), t2);
    let gb = clamp(t.g - t.b, 0.0, 1.0);
    let rb = clamp(t.r - t.b, 0.0, 1.0);
    var rgb = rb * in.p0.rgb + gb * in.p1.rgb + vec3<f32>(t.b);
    let s = (in.color.r + in.color.g + in.color.b) / 3.0;
    rgb = rgb * s + (1.0 - s) * in.color.rgb;
    let a = t.a * in.color.a;
    let q = (uvu - o.xy) / o.zw;
    var r = length(q - vec2<f32>(0.5)) * 2.0;
    r = r * r;
    r = min(r * r, 1.0);
    let ends = clamp(q.y * (1.0 - q.y) * 16.0, 0.0, 1.0);
    let f = (1.0 - r) * ends;
    return vec4<f32>(rgb * a, a) * f;
}

// Custom/ExpandingCircle (sprites: no custom vertex stream, age 0).
fn expanding(in: VsOut, uvu: vec2<f32>) -> vec4<f32> {
    let progress = in.p0.x;
    let dist = in.p1.xyz;
    let radius = in.p2.xyz;
    let pr = clamp(progress - 0.3, 0.0, 1.0);
    let rr = radius.xy - vec2<f32>(pr * 14.0);
    let n = textureSampleLevel(tex1, samp1, vec2<f32>(uvu.x, 1.0 - uvu.y), 0.0).xy;
    let pp = n * dist.x + uvu;
    let cc = dist.x * 0.5 + 0.5;
    let dd = vec2<f32>(cc) - pp;
    let d2 = dot(dd, dd);
    var w = vec2<f32>(1.0) - vec2<f32>(d2 + 0.15) * (rr + vec2<f32>(6.0));
    w = w * w;
    w = min(w * w, vec2<f32>(1.0));
    w = vec2<f32>(1.0) - w;
    let ring = max(w.x - w.y, 0.0);
    var col = in.color * ring;
    let e0 = vec2<f32>(0.5) - uvu;
    var e = 1.0 - (dot(e0, e0) + 0.15) * radius.z;
    e = e * e;
    e = min(e * e, 1.0);
    col = col * (1.0 - e);
    col.a = col.a * sample_u(tex2, samp2, uvu).r;
    col = col * dist.y;
    let a2 = col.a * col.a;
    return vec4<f32>(col.rgb * a2, a2);
}

// Unlit/ResizablePlatform: rounded rectangle in the vertex colour with a
// black outline, anti-aliased (quad-local units).
fn rplatform(in: VsOut, aa: f32) -> vec4<f32> {
    let half = in.p0.xy;
    let rad = in.p0.z;
    let border = in.p0.w;
    let q = abs(in.local) - half;
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - rad;
    let outer = coverage(d, aa);
    let inner = coverage(d + border, aa);
    let a = min(in.color.a * inner + outer, 1.0) * in.color.a;
    return vec4<f32>(in.color.rgb * inner * in.color.a, a);
}

// Custom/LightningA8: the strip repeats `scale` times along u, every other
// copy mirrored between uv_min and uv_max; bilinear alpha plus a radial
// blur glow in the vertex colour.
fn lightning_uv(q: vec2<f32>, scale: f32, vmin: f32, vmax: f32) -> vec2<f32> {
    let x = q.x * scale - scale * 0.5;
    let m = (x + 1000.0) - 2.0 * floor((x + 1000.0) * 0.5);
    var y = q.y;
    if (m >= 1.0) {
        y = vmax - (q.y - vmin);
    }
    return vec2<f32>(fract(x), y);
}

fn lightning(in: VsOut, uvu: vec2<f32>) -> vec4<f32> {
    let scale = max(in.p0.x, 1e-4);
    let vmin = in.p0.y;
    let vmax = in.p0.z;
    let blur = in.p0.w;
    let dirs = max(in.p1.x, 1.0);
    let quality = max(in.p1.y, 1.0);
    let dims = vec2<f32>(textureDimensions(tex0));
    let texel = vec2<f32>(1.0) / max(dims, vec2<f32>(1.0));
    let p0 = uvu + texel;
    let sa = main_u(lightning_uv(p0 + vec2<f32>(0.0, texel.y), scale, vmin, vmax)).a;
    let sb = main_u(lightning_uv(p0 + vec2<f32>(texel.x / scale, 0.0), scale, vmin, vmax)).a;
    let sc = main_u(lightning_uv(p0 + vec2<f32>(texel.x / scale, texel.y), scale, vmin, vmax)).a;
    let sd = main_u(lightning_uv(p0, scale, vmin, vmax)).a;
    let f = fract(vec2<f32>(scale * dims.x * p0.x, p0.y * dims.y));
    let lo = mix(sd, sb, f.x);
    let hi = mix(sa, sc, f.x);
    let v = mix(lo, hi, f.y);
    var col = v * (vec4<f32>(1.0) - in.color) + in.color;
    col.a = v * col.a;
    let bt = texel * blur;
    var sum = sd;
    let step_q = 1.0 / quality;
    for (var a = 0.0; a < TAU; a = a + TAU / dirs) {
        let o = bt * vec2<f32>(cos(a) / scale, sin(a));
        for (var i = step_q; i <= 1.0; i = i + step_q) {
            sum = sum + main_u(lightning_uv(o * i + p0, scale, vmin, vmax)).a;
        }
    }
    let glow = sum / max(quality * dirs - 15.0, 1.0);
    let o = glow * in.color + col;
    return vec4<f32>(o.rgb * o.a, o.a);
}

// 2D simplex noise (the classic skewed-grid formulation with a polynomial
// permutation mod 289), in [-1, 1].
fn mod289v3(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn mod289v2(x: vec2<f32>) -> vec2<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn permute3(x: vec3<f32>) -> vec3<f32> {
    return mod289v3(((x * 34.0) + 1.0) * x);
}

fn simplex2(v: vec2<f32>) -> f32 {
    let cx = 0.211324865405187;
    let cy = 0.366025403784439;
    let cz = -0.577350269189626;
    let cw = 0.024390243902439;
    var i = floor(v + vec2<f32>(dot(v, vec2<f32>(cy))));
    let x0 = v - i + vec2<f32>(dot(i, vec2<f32>(cx)));
    var i1 = vec2<f32>(0.0, 1.0);
    if (x0.x > x0.y) {
        i1 = vec2<f32>(1.0, 0.0);
    }
    let x12a = x0 + vec2<f32>(cx) - i1;
    let x12b = x0 + vec2<f32>(cz);
    i = mod289v2(i);
    let pm = permute3(permute3(i.y + vec3<f32>(0.0, i1.y, 1.0)) + i.x + vec3<f32>(0.0, i1.x, 1.0));
    var m = max(vec3<f32>(0.5) - vec3<f32>(dot(x0, x0), dot(x12a, x12a), dot(x12b, x12b)), vec3<f32>(0.0));
    m = m * m;
    m = m * m;
    let x = 2.0 * fract(pm * cw) - 1.0;
    let h = abs(x) - 0.5;
    let ox = floor(x + 0.5);
    let a0 = x - ox;
    m = m * (1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h));
    let gx = a0.x * x0.x + h.x * x0.y;
    let gy = a0.y * x12a.x + h.y * x12a.y;
    let gz = a0.z * x12b.x + h.z * x12b.y;
    return 130.0 * dot(m, vec3<f32>(gx, gy, gz));
}

// The beam shaders (u along the beam, v across; `_platformRT` hides the
// beam inside walls).
fn beam(in: VsOut, uvu: vec2<f32>) -> vec4<f32> {
    let kind = u32(in.p0.x + 0.5);
    let line_width = max(in.p0.y, 1e-4);
    let noise_freq = in.p0.z;
    let scale = in.p0.w;
    let aura = in.p1.x;
    let time_scale = in.p1.y;
    let dist_str = in.p1.z;
    let dist_str2 = in.p1.w;
    let ratio = in.p2.x;
    let ratio2 = in.p2.y;
    let wall = 1.0 - sample_screen(tex2, samp2, in.screen).a;
    let u = uvu.x;
    let v = uvu.y;
    var a = 0.0;
    switch kind {
        case 0u: {
            let t = g.time.w * time_scale;
            let n = simplex2(vec2<f32>(scale * ratio * u * noise_freq - t, v * noise_freq));
            let qx = scale * ratio2 * u;
            let cs = g.cos_time.w;
            let sn = g.sin_time.w;
            let r = vec2<f32>(dot(vec2<f32>(qx, v), vec2<f32>(cs, -sn)), dot(vec2<f32>(qx, v), vec2<f32>(sn, cs)));
            let tex = textureSampleLevel(tex1, samp1, r, 0.0);
            let k = (clamp(1.0 - tex.x, 0.0, 1.0) - 0.5) * dist_str + vec2<f32>(0.2, 0.8);
            let e = clamp((vec2<f32>(v) - k) / line_width + vec2<f32>(n), vec2<f32>(0.0), vec2<f32>(1.0));
            let band = e.x * (1.0 - e.y);
            var fall = clamp((qx + 0.18) * aura * (v * aura) * ((1.0 - v) * aura), 0.0, 1.0);
            fall = fall * fall;
            fall = fall * fall;
            fall = fall * fall;
            a = band * fall;
        }
        case 1u: {
            let qx = scale * ratio2 * u;
            let cs = g.cos_time.w;
            let sn = g.sin_time.w;
            let r = vec2<f32>(dot(vec2<f32>(qx, v), vec2<f32>(cs, -sn)), dot(vec2<f32>(qx, v), vec2<f32>(sn, cs)));
            let start = clamp(aura * qx * 2.0, 0.0, 1.0);
            let tex = textureSampleLevel(tex1, samp1, r, 0.0);
            let k = (clamp(1.0 - tex.x, 0.0, 1.0) - 0.5) * dist_str2 + vec2<f32>(0.2, 0.8);
            let e = clamp(((vec2<f32>(v) - k) / line_width + vec2<f32>(1.0)) * 0.4, vec2<f32>(0.0), vec2<f32>(1.0));
            let band = (1.0 - e.y) * e.x;
            let endf = clamp((1.0 - u) * aura, 0.0, 1.0) * start;
            let an = aura * noise_freq;
            let lo = an * (v - 0.08);
            let hi = an * (1.0 - (v + 0.08));
            let side = clamp(hi * (lo * endf) * 0.25, 0.0, 1.0);
            a = band * side;
        }
        case 2u: {
            var w = clamp(scale * ratio * u * aura, 0.0, 1.0);
            w = w * (v * aura) * ((1.0 - v) * aura) * 0.5;
            a = min(w * w, 1.0);
        }
        default: {
            let y1 = clamp(v + 0.5, 0.0, 1.0);
            let y2 = clamp(1.5 - v, 0.0, 1.0);
            var b = y1 * y2 - 0.5;
            b = b * b * min(abs(v - 0.5), 1.0);
            let end_w = 6.0 / max(scale, 1e-4);
            let x = clamp(u / end_w, 0.0, 1.0) * clamp((1.0 - u) / end_w, 0.0, 1.0);
            let c = in.color * b * x * 30.0 * in.color.a;
            return c * wall;
        }
    }
    return in.color * a * wall;
}

// Custom/CloneEffect: four copies of the sprite pushed out along
// directions around `direction` as `progress` grows (the quad is drawn 4x
// larger), the brightest (r + b) copy wins.
fn clone_fx(in: VsOut, uvu: vec2<f32>) -> vec4<f32> {
    let progress = in.p0.x;
    let noise_str = in.p0.y;
    let noise_freq = in.p0.z;
    let uv_off = in.p0.w;
    let dir = in.p1.xy;
    let ang = atan2(dir.y, dir.x);
    let st = sin(g.time.y);
    let ct = cos(g.time.y);
    let nq = uvu * noise_freq - vec2<f32>(0.5);
    let nr = vec2<f32>(dot(vec2<f32>(ct, st), nq), dot(vec2<f32>(-st, ct), nq));
    let n = textureSampleLevel(tex1, samp1, nr, 0.0).x * noise_str;
    let p2 = progress * progress;
    let wob = clamp((p2 - 2.0 * progress * progress + progress) * n, 0.0, 1.0);
    let spreads = array<f32, 4>(
        min(p2, 1.0) + wob,
        1.0 - min((1.0 - progress) * (1.0 - progress), 1.0) + wob,
        clamp(progress, 0.0, 1.0) + wob,
        clamp(progress * n, 0.0, 1.0) + wob
    );
    let offs = array<f32, 4>(-0.471238911, -0.314159274, 0.314159274, 0.471238911);
    let base = uvu * 4.0 - vec2<f32>(1.5);
    var best = vec4<f32>(0.0);
    var best_l = -1.0;
    var alpha = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        let s = spreads[i];
        let a = ang + offs[i] * s;
        let len = s * (uv_off - s) + s;
        let c = main_u(base - len * vec2<f32>(cos(a), sin(a)));
        let pm = c * c.a;
        let l = pm.r + pm.b;
        if (l > best_l) {
            best_l = l;
            best = pm;
        }
        alpha = max(alpha, pm.a);
    }
    let o = vec4<f32>(best.rgb, alpha) * in.color;
    return vec4<f32>(o.rgb * o.a, o.a);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Derivatives in uniform control flow.
    let aa = max(length(dpdx(in.local)) + length(dpdy(in.local)), 1e-6) * 0.5;
    let duv_dx = dpdx(in.uv);
    let duv_dy = dpdy(in.uv);
    let t = textureSampleLevel(tex0, samp0, in.uv, 0.0);
    let uvu = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    let half = in.size * 0.5;
    switch in.kind {
        case K_SPRITE: {
            return premul(t * in.color);
        }
        case K_ALPHA_TINT: {
            let a = t.a * in.color.a;
            return vec4<f32>(in.color.rgb * a, a);
        }
        case K_CHANNEL: {
            let x = clamp(t.g - t.b, 0.0, 1.0);
            let y = clamp(t.r - t.b, 0.0, 1.0);
            var c = y * in.p0.rgb + x * in.p1.rgb + vec3<f32>(t.b);
            let s = (in.color.r + in.color.g + in.color.b) / 3.0;
            c = c * s + (1.0 - s) * in.color.rgb;
            let a = t.a * in.color.a;
            return vec4<f32>(c * a, a);
        }
        case K_GRADIENT: {
            let h1 = in.p2.z + 1.0;
            let bw = 1.0 - uvu.y * h1;
            let tw = h1 * uvu.y;
            let rgb = in.p0.rgb * tw + in.p1.rgb * bw;
            return vec4<f32>(rgb, tw * in.p2.x + bw * in.p2.y);
        }
        case K_SLIME: {
            return slime(in, uvu, t);
        }
        case K_WATER: {
            return water(in, uvu);
        }
        case K_DISTORT: {
            return distortion(in, uvu);
        }
        case K_AURORA: {
            return aurora(in, uvu);
        }
        case K_ELLIPSE: {
            let r = min(half.x, half.y);
            let d = (length(in.local / max(half, vec2<f32>(1e-6))) - 1.0) * r;
            return premul(in.color) * coverage(d, aa);
        }
        case K_RING: {
            let r = length(in.local);
            let outer = half.x;
            let inner = outer - in.p0.x;
            var d = max(r - outer, inner - r);
            let sweep = in.p0.z;
            if (sweep < TAU - 1e-4) {
                let mid = in.p0.y + sweep * 0.5;
                let a = atan2(in.local.y, in.local.x) - mid;
                let dev = abs(a - TAU * round(a / TAU));
                d = max(d, (dev - sweep * 0.5) * r);
            }
            return premul(in.color) * coverage(d, aa);
        }
        case K_INDICATOR: {
            // Custom/AbilityIndicatorWithCircle.
            let r = length(in.local);
            let radius = in.p3.x;
            let cov = coverage(r - radius, aa);
            let rimf = 1.0 - coverage(r - (radius - in.p3.y), aa);
            let base = mix(in.p0, in.p1, rimf);
            let b = vec4<f32>(base.rgb * base.a, base.a) * cov;
            let inside = in.uv.x >= in.p4.x && in.uv.x <= in.p4.z && in.uv.y >= in.p4.y && in.uv.y <= in.p4.w;
            var ic = premul(t);
            if (!inside) {
                ic = vec4<f32>(0.0);
            }
            var c = ic + b * (1.0 - ic.a);
            let progress = in.p3.z;
            if (progress < 1.0) {
                // Uncharged sector: clockwise from 12 o'clock.
                let sweep = (1.0 - clamp(progress, 0.0, 1.0)) * TAU;
                let a = atan2(in.local.x, in.local.y) - sweep * 0.5;
                let dev = abs(a - TAU * round(a / TAU));
                let f = coverage((dev - sweep * 0.5) * max(r, 1e-4), aa);
                let filled = vec4<f32>(c.rgb * in.p2.rgb, c.a) * in.p2.a;
                c = mix(c, filled, f);
            }
            return vec4<f32>(c.rgb * in.color.rgb, c.a) * in.color.a;
        }
        case K_RRECT: {
            let rad = min(in.p0.x, min(half.x, half.y));
            let q = abs(in.local) - half + vec2<f32>(rad);
            let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - rad;
            return premul(t * in.color) * coverage(d, aa);
        }
        case K_SDF: {
            // Distance in alpha, 0.5 at the outline (TextMeshPro). p0.x:
            // edge shift (dilate), p0.y: value change per atlas texel,
            // p0.z: outline width (value units), p1: outline colour.
            let dims = vec2<f32>(textureDimensions(tex0));
            let texels = 0.5 * (length(duv_dx * dims) + length(duv_dy * dims));
            let w = max(texels * in.p0.y, 1e-4);
            let face_edge = 0.5 - in.p0.x + in.p0.z * 0.5;
            let outer_edge = 0.5 - in.p0.x - in.p0.z * 0.5;
            var face = 0.0;
            var outer = 0.0;
            for (var i = 0u; i < 4u; i = i + 1u) {
                let o = vec2<f32>(f32(i & 1u), f32(i >> 1u)) * 0.5 - vec2<f32>(0.25);
                let v = textureSampleLevel(tex0, samp0, in.uv + duv_dx * o.x + duv_dy * o.y, 0.0).a;
                face = face + clamp((v - face_edge) / w + 0.5, 0.0, 1.0);
                outer = outer + clamp((v - outer_edge) / w + 0.5, 0.0, 1.0);
            }
            face = face * 0.25;
            outer = outer * 0.25;
            let fc = premul(in.color);
            let oc = premul(in.p1);
            return mix(oc, fc, face) * outer;
        }
        case K_MASK: {
            if (t.a < in.p0.x) {
                discard;
            }
            return vec4<f32>(0.0);
        }
        case K_COPY: {
            return t;
        }
        case K_BLACKHOLE: {
            return black_hole(in, uvu);
        }
        case K_GRAB_INVERT: {
            let gr = sample_screen(tex1, samp1, in.screen);
            let rgb = gr.rgb * (1.0 - t.a) + (vec3<f32>(1.0) - gr.rgb) * t.a + in.color.rgb;
            let a = t.a * in.color.a;
            return vec4<f32>(rgb * a, a * a);
        }
        case K_DISTORT_SPRITE: {
            return distort_sprite(in, uvu);
        }
        case K_SMEAR: {
            return smear(in, uvu);
        }
        case K_ONLY_WHITE: {
            var a = t.r * t.a;
            a = a * a;
            a = a * a;
            a = a * a * in.color.a;
            return vec4<f32>(in.color.rgb * a, a);
        }
        case K_EXPANDING: {
            return expanding(in, uvu);
        }
        case K_WHITE_TO_COL: {
            let rgb = t.a * (vec3<f32>(1.0) - in.color.rgb) + in.color.rgb;
            return vec4<f32>(rgb * t.a, t.a);
        }
        case K_WALL_MASKED: {
            let a = (1.0 - sample_screen(tex1, samp1, in.screen).a) * t.a;
            return vec4<f32>(in.color.rgb * a, a);
        }
        case K_SINGLE_CHANNEL: {
            let v = min(min(t.r * in.p0.x + t.g * in.p0.y + t.b * in.p0.z, 1.0), t.a) * in.color.a;
            return vec4<f32>(in.color.rgb * v, v);
        }
        case K_CHANNELS3: {
            let rgb = t.g * in.p1.rgb * in.p1.a + t.r * in.p0.rgb * in.p0.a + t.b * in.p2.rgb * in.p2.a;
            let a = (in.p0.a * t.r + in.p2.a * t.b + in.p1.a * t.g) * t.a;
            return vec4<f32>(rgb * t.a, a);
        }
        case K_RPLATFORM: {
            return rplatform(in, aa);
        }
        case K_LIGHTNING: {
            return lightning(in, uvu);
        }
        case K_BEAM: {
            return beam(in, uvu);
        }
        case K_CLONE: {
            return clone_fx(in, uvu);
        }
        case K_PARTICLE_PREMUL: {
            return in.color * t * in.color.a;
        }
        case K_PARTICLE_BLEND: {
            let c = 2.0 * in.color * in.p0 * t;
            return vec4<f32>(c.rgb * c.a, c.a);
        }
        default: {
            return premul(t * in.color);
        }
    }
}
