// Final-render post pass: the HDR scene (premultiplied by coverage) is
// exposed, tone mapped to display range and composited over the
// background, then encoded to sRGB for an Rgba8Unorm target.

struct Post {
    // Linear-light background colour and its alpha (0: transparent).
    background: vec4<f32>,
    // x: exposure multiplier, y: operator (0 Khronos PBR Neutral, 1 ACES).
    params: vec4<f32>,
};

@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var<uniform> post: Post;

@vertex
fn vs_post(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the screen.
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// Khronos PBR Neutral: leaves colours below about 0.76 untouched, so a
// fabric's base colour reads true, and rolls highlights off to white.
fn neutral(color: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desaturation = 0.15;
    let x = min(color.r, min(color.g, color.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var c = color - offset;
    let peak = max(c.r, max(c.g, c.b));
    if peak < start {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let gray = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), gray);
}

// Narkowicz's fit of the ACES filmic curve: more contrast, film-like.
fn aces(color: vec3<f32>) -> vec3<f32> {
    let x = color * 0.6;
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn encode(c: vec3<f32>) -> vec3<f32> {
    let c1 = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c1 * 12.92;
    let high = 1.055 * pow(c1, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c1 <= vec3<f32>(0.0031308));
}

@fragment
fn fs_post(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(hdr, vec2<i32>(p.xy), 0);
    let a = clamp(c.a, 0.0, 1.0);
    var scene = vec3<f32>(0.0);
    if a > 0.0 {
        let exposed = max(c.rgb / a, vec3<f32>(0.0)) * post.params.x;
        scene = select(neutral(exposed), aces(exposed), post.params.y > 0.5) * a;
    }
    let bg = post.background;
    let alpha = a + bg.a * (1.0 - a);
    let premultiplied = scene + bg.rgb * bg.a * (1.0 - a);
    let straight = select(vec3<f32>(0.0), premultiplied / alpha, alpha > 0.0);
    return vec4<f32>(encode(straight), alpha);
}
