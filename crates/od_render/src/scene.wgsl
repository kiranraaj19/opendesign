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
    ibl: vec4<f32>,     // x: environment strength, y: last mip, z: roughness
};

@group(0) @binding(0) var<uniform> g: Globals;

// A diffuse/base-color texture, multiplied with the vertex color. Surfaces
// with no fabric texture of their own are bound to a solid white 1x1 pixel
// (see od_render::gpu::Renderer), so sampling it is a no-op.
@group(1) @binding(0) var diffuse_tex: texture_2d<f32>;
@group(1) @binding(1) var diffuse_samp: sampler;

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

@fragment
fn fs_lit(i: LitOut) -> @location(0) vec4<f32> {
    let n = normalize(i.normal);
    let view = normalize(g.eye.xyz - i.world);
    let key = max(dot(n, g.key.xyz), 0.0) * g.key.w;
    let fill = max(dot(n, g.fill.xyz), 0.0) * g.fill.w;
    let ambient = mix(g.ground.rgb, g.sky.rgb, n.y * 0.5 + 0.5);
    // A soft highlight from the key light (Blinn-Phong).
    let half_dir = normalize(g.key.xyz + view);
    let spec = pow(max(dot(n, half_dir), 0.0), 40.0) * 0.12 * g.key.w * step(0.0, dot(n, g.key.xyz));
    let tex = textureSample(diffuse_tex, diffuse_samp, i.uv);
    let base_color = i.color * tex;

    // "Final render" quality (params.y > 0.5), off in the live viewport:
    // a soft rim/back light opposite the key+fill pair, three-point-style,
    // to separate silhouettes from the background; and a cheap per-pixel
    // stand-in for ambient occlusion that dims the sky/ground ambient term
    // on surfaces turned away from the camera (no real occluder geometry,
    // just a view-angle fade).
    let quality = step(0.5, g.params.y);
    let ao = mix(1.0, 0.8 + 0.2 * max(dot(n, view), 0.0), quality);

    // With an environment (g.ibl.x > 0, final renders only), it replaces
    // the sky/ground ambient and the rim light, and adds soft reflections
    // of the studio's light sources.
    let lit_env = g.ibl.x > 0.0;
    let rim_dir = normalize(-(g.key.xyz + g.fill.xyz));
    let rim = max(dot(n, rim_dir), 0.0) * 0.3 * quality * select(1.0, 0.0, lit_env);
    let env_diffuse = irradiance(to_rig(n)) * g.ibl.x;
    let roughness = g.ibl.z;
    let nv = max(dot(n, view), 1e-3);
    let reflected = to_rig(reflect(-view, n));
    let env_spec = textureSampleLevel(env_tex, env_samp, env_uv(reflected), roughness * g.ibl.y).rgb
        * env_brdf(0.04, roughness, nv) * g.ibl.x;
    let diffuse_light = select(ambient, env_diffuse, lit_env) * ao + vec3<f32>(key + fill + rim);

    let rgb = base_color.rgb * diffuse_light + vec3<f32>(spec) + env_spec * ao;
    return vec4<f32>(encode(rgb), base_color.a);
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
