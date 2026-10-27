// 3D garment view. Colours arrive in linear light; when the target is not
// an sRGB format (egui prefers plain Unorm), the shader encodes them itself.

struct Globals {
    view_proj: mat4x4<f32>,
    key: vec4<f32>,     // xyz direction towards the light, w intensity
    fill: vec4<f32>,
    sky: vec4<f32>,
    ground: vec4<f32>,
    eye: vec4<f32>,
    params: vec4<f32>,  // x: encode to sRGB
};

@group(0) @binding(0) var<uniform> g: Globals;

// A diffuse/base-color texture, multiplied with the vertex color. Surfaces
// with no fabric texture of their own are bound to a solid white 1x1 pixel
// (see od_render::gpu::Renderer), so sampling it is a no-op.
@group(1) @binding(0) var diffuse_tex: texture_2d<f32>;
@group(1) @binding(1) var diffuse_samp: sampler;

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
    let rgb = base_color.rgb * (ambient + vec3<f32>(key + fill)) + vec3<f32>(spec);
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
