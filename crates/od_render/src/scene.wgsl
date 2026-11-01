// 3D garment view. Colours arrive in linear light; when the target is not
// an sRGB format (egui prefers plain Unorm), the shader encodes them itself.

struct Globals {
    view_proj: mat4x4<f32>,
    key: vec4<f32>,     // xyz direction towards the light, w intensity
    fill: vec4<f32>,
    sky: vec4<f32>,
    ground: vec4<f32>,
    eye: vec4<f32>,
    params: vec4<f32>,  // x: encode to sRGB, y: "final render" quality
    sh: array<vec4<f32>, 9>,
    rig: vec4<f32>,
    ibl: vec4<f32>,     // x: environment strength, y: last mip
};

@group(0) @binding(0) var<uniform> g: Globals;

// A surface's material (see od_render::material). The colour texture is
// multiplied with the vertex colour; the detail texture holds a tangent-
// space normal and, in alpha, a roughness scale (0.5 for x1). Surfaces
// with no material of their own get a white pixel, a flat normal and
// matte defaults (see od_render::gpu::Renderer).
struct Surface {
    params: vec4<f32>,  // roughness, sheen roughness, translucency, opacity
    sheen: vec4<f32>,   // sheen colour, detail normal strength
    extra: vec4<f32>,   // x: samples per pixel
};

@group(1) @binding(0) var diffuse_tex: texture_2d<f32>;
@group(1) @binding(1) var diffuse_samp: sampler;
@group(1) @binding(2) var detail_tex: texture_2d<f32>;
@group(1) @binding(3) var<uniform> surf: Surface;

// The environment for image-based lighting, an equirect image whose mip
// levels are prefiltered for rougher and rougher reflections (see
// od_render::light). A black pixel while the live viewport has none.
@group(2) @binding(0) var env_tex: texture_2d<f32>;
@group(2) @binding(1) var env_samp: sampler;

const PI: f32 = 3.14159265;

// A world direction in the light rig's frame (x right, y up, z towards the
// camera), which turns with the camera.
fn to_rig(d: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(d.x * g.rig.x + d.z * g.rig.y, d.y, d.x * g.rig.z + d.z * g.rig.w);
}

// Must match od_render::light::uv.
fn env_uv(d: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(atan2(d.x, d.z) / (2.0 * PI) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI);
}

// Diffuse light / pi towards rig-frame normal n; the basis must match
// od_render::light::sh_basis.
fn irradiance(n: vec3<f32>) -> vec3<f32> {
    var e = g.sh[0].rgb * 0.282095;
    e += g.sh[1].rgb * (0.488603 * n.y);
    e += g.sh[2].rgb * (0.488603 * n.z);
    e += g.sh[3].rgb * (0.488603 * n.x);
    e += g.sh[4].rgb * (1.092548 * n.x * n.y);
    e += g.sh[5].rgb * (1.092548 * n.y * n.z);
    e += g.sh[6].rgb * (0.315392 * (3.0 * n.z * n.z - 1.0));
    e += g.sh[7].rgb * (1.092548 * n.x * n.z);
    e += g.sh[8].rgb * (0.546274 * (n.x * n.x - n.y * n.y));
    return max(e, vec3<f32>(0.0));
}

// Karis' analytic fit of the split-sum environment BRDF, in place of a
// lookup texture: how much of the prefiltered reflection reaches the eye.
fn env_brdf(f0: f32, roughness: f32, nv: f32) -> f32 {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

fn encode(c: vec3<f32>) -> vec3<f32> {
    if g.params.x < 0.5 {
        return c;
    }
    let c1 = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c1 * 12.92;
    let high = 1.055 * pow(c1, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c1 <= vec3<f32>(0.0031308));
}

// ---- Shaded surfaces: avatar and patterns ----

struct LitIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct LitOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

@vertex
fn vs_lit(v: LitIn) -> LitOut {
    var out: LitOut;
    out.clip = g.view_proj * vec4<f32>(v.position, 1.0);
    out.world = v.position;
    out.normal = v.normal;
    out.color = v.color;
    out.uv = v.uv;
    return out;
}

// Where the surface is covered, as a sample mask: opaque surfaces
// cover every sample; one with opacity below 1 covers that share of
// them, spread evenly over each 4x4 block of pixels by an ordered
// (Bayer) dither, and the multisample resolve blends it with what's
// behind. No sorting.
fn coverage(opacity: f32, pixel: vec2<f32>) -> u32 {
    if opacity >= 0.999 {
        return 0xffffffffu;
    }
    let n = u32(max(surf.extra.x, 1.0));
    let p = vec2<u32>(pixel) % vec2<u32>(4u);
    let bayer = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
    let rank = bayer[p.y * 4u + p.x];
    let dither = (f32(rank) + 0.5) / 16.0;
    let count = min(u32(floor(opacity * f32(n) + dither)), n);
    if count >= 32u {
        return 0xffffffffu;
    }
    let bits = (1u << count) - 1u;
    // Turn which samples by the pixel too, so they don't all leave the
    // same sample position empty.
    let turn = rank % n;
    let mask = select((1u << n) - 1u, 0xffffffffu, n >= 32u);
    return ((bits << turn) | (bits >> ((n - turn) % 32u))) & mask;
}

// The normal tilted by the detail map, in a tangent frame built from how
// position and texture coordinates change across the pixel (no stored
// tangents). Where the coordinates don't change, the normal is kept.
fn detailed_normal(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>, m: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let len2 = max(dot(t, t), dot(b, b));
    let ok = len2 > 1e-30;
    let inv = inverseSqrt(select(1.0, len2, ok));
    let tilted = normalize(t * (inv * m.x) + b * (inv * m.y) + n * m.z);
    return select(n, tilted, ok);
}

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Height-correlated Smith visibility for GGX.
fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-5);
}

// Estevez and Kulla's "Charlie" sheen distribution: fibres standing off
// the cloth catch light most at grazing angles.
fn d_charlie(a: f32, nh: f32) -> f32 {
    let inv = 1.0 / a;
    let sin2 = max(1.0 - nh * nh, 0.0078125);
    return (2.0 + inv) * pow(sin2, inv * 0.5) / (2.0 * PI);
}

fn v_neubelt(nv: f32, nl: f32) -> f32 {
    return 1.0 / max(4.0 * (nl + nv - nl * nv), 1e-4);
}

// Roughly how much of the light from all round the sheen lobe sends
// towards the eye: little face on, much more at the silhouette.
fn sheen_albedo(nv: f32, a: f32) -> f32 {
    return (0.12 + 0.65 * pow(1.0 - nv, 3.0)) * (1.0 - 0.45 * a);
}

// Specular and sheen light from one direction (diffuse is added apart),
// in the same units as a Lambert term `base * nl * intensity`.
fn lobes(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, a: f32, sheen: vec3<f32>, sa: f32) -> vec3<f32> {
    let nl = max(dot(n, l), 0.0);
    let h = normalize(l + v);
    let nh = max(dot(n, h), 0.0);
    let nv = max(dot(n, v), 1e-3);
    let vh = max(dot(v, h), 0.0);
    let fresnel = 0.04 + 0.96 * pow(1.0 - vh, 5.0);
    let spec = d_ggx(nh, a) * v_smith(nv, nl, a) * fresnel;
    let glow = d_charlie(sa, nh) * v_neubelt(nv, nl);
    return (vec3<f32>(spec) + sheen * glow) * (PI * nl);
}

struct LitFragment {
    @location(0) color: vec4<f32>,
    @builtin(sample_mask) mask: u32,
};

@fragment
fn fs_lit(i: LitOut) -> LitFragment {
    let tex = textureSample(diffuse_tex, diffuse_samp, i.uv);
    let detail = textureSample(detail_tex, diffuse_samp, i.uv);
    let geometric = normalize(i.normal);
    var m = detail.rgb * 2.0 - 1.0;
    m = vec3<f32>(m.xy * surf.sheen.w, max(m.z, 1e-3));
    let n = detailed_normal(geometric, i.world, i.uv, m);
    let view = normalize(g.eye.xyz - i.world);
    let base_color = i.color * tex;
    let base = base_color.rgb;

    let roughness = clamp(surf.params.x * detail.a * 2.0, 0.03, 1.0);
    let a = roughness * roughness;
    let sheen = surf.sheen.rgb;
    let sa = max(surf.params.y * surf.params.y, 0.07);
    let translucency = surf.params.z;
    let nv = max(dot(n, view), 1e-3);
    // The sheen takes its light before the base gets it.
    let sheen_cover = 1.0 - max(max(sheen.r, sheen.g), sheen.b) * sheen_albedo(nv, sa);

    // "Final render" quality (params.y > 0.5), off in the live viewport:
    // a soft rim/back light opposite the key+fill pair, three-point-style,
    // to separate silhouettes from the background; and a cheap per-pixel
    // stand-in for ambient occlusion that dims the sky/ground ambient term
    // on surfaces turned away from the camera (no real occluder geometry,
    // just a view-angle fade).
    let quality = step(0.5, g.params.y);
    let ao = mix(1.0, 0.8 + 0.2 * max(dot(n, view), 0.0), quality);

    // With an environment (g.ibl.x > 0, final renders only), it replaces
    // the sky/ground ambient and the rim light, and reflects in the
    // surface as sharply as it is smooth.
    let lit_env = g.ibl.x > 0.0;
    let ambient = mix(g.ground.rgb, g.sky.rgb, n.y * 0.5 + 0.5);
    let rim_dir = normalize(-(g.key.xyz + g.fill.xyz));
    let rim = max(dot(n, rim_dir), 0.0) * 0.3 * quality * select(1.0, 0.0, lit_env);
    let env_diffuse = irradiance(to_rig(n)) * g.ibl.x;
    let reflected = to_rig(reflect(-view, n));
    let env_spec = textureSampleLevel(env_tex, env_samp, env_uv(reflected), roughness * g.ibl.y).rgb
        * env_brdf(0.04, roughness, nv) * g.ibl.x;
    let env_sheen = env_diffuse * sheen * sheen_albedo(nv, sa);
    let sky = select(ambient, env_diffuse, lit_env);

    let key = max(dot(n, g.key.xyz), 0.0) * g.key.w;
    let fill = max(dot(n, g.fill.xyz), 0.0) * g.fill.w;
    // Thin cloth lit from behind glows through.
    let behind = max(dot(-n, g.key.xyz), 0.0) * g.key.w + max(dot(-n, g.fill.xyz), 0.0) * g.fill.w
        + select(0.0, max(dot(-n, rim_dir), 0.0) * 0.3 * quality, !lit_env);
    let through = (vec3<f32>(behind) + select(ambient, irradiance(to_rig(-n)) * g.ibl.x, lit_env))
        * translucency;
    let diffuse_light = (sky * ao + vec3<f32>(key + fill + rim)) * (1.0 - 0.5 * translucency)
        + through;

    let direct = lobes(n, view, g.key.xyz, a, sheen, sa) * g.key.w
        + lobes(n, view, g.fill.xyz, a, sheen, sa) * g.fill.w;
    let rgb = base * diffuse_light * sheen_cover + direct + (env_spec + env_sheen) * ao;
    var out: LitFragment;
    out.color = vec4<f32>(encode(rgb), base_color.a);
    out.mask = coverage(surf.params.w, i.clip.xy);
    return out;
}

// ---- Unlit lines and the floor shadow ----

struct FlatIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct FlatOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_flat(v: FlatIn) -> FlatOut {
    var out: FlatOut;
    out.clip = g.view_proj * vec4<f32>(v.position, 1.0);
    out.color = v.color;
    return out;
}

@fragment
fn fs_flat(i: FlatOut) -> @location(0) vec4<f32> {
    return vec4<f32>(encode(i.color.rgb), i.color.a);
}
