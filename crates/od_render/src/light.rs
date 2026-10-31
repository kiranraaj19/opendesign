//! Image-based lighting for final renders: an environment (an equirect HDR
//! image, read from a file or generated for a preset), its diffuse
//! irradiance as spherical harmonics, and a mip chain prefiltered for rough
//! reflections. Plain CPU data, so it's tested without a GPU;
//! [`crate::gpu::Renderer::set_lighting`] uploads it.
//!
//! Directions are in the light rig's frame: x to the camera's right, y up,
//! z back towards the camera. The rig turns with the camera, like the
//! viewport's two lights, so whichever side is shown is lit from the front.

use std::f32::consts::{PI, TAU};
use std::sync::{Arc, OnceLock};

use glam::Vec3;

/// An equirectangular HDR image in linear light. Row 0 is straight up;
/// the middle column faces the camera, the outer ones meet behind the
/// subject.
#[derive(Debug, Clone, PartialEq)]
pub struct Environment {
    pub width: u32,
    pub height: u32,
    /// `width * height` texels, row-major, top to bottom.
    pub texels: Vec<[f32; 3]>,
}

impl Environment {
    /// Fill an image from radiance by direction, averaging 2x2 samples per
    /// texel so sharp-edged light sources don't alias.
    pub fn from_fn(width: u32, height: u32, radiance: impl Fn(Vec3) -> Vec3) -> Self {
        let mut texels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let mut sum = Vec3::ZERO;
                for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let u = (x as f32 + dx) / width as f32;
                    let v = (y as f32 + dy) / height as f32;
                    sum += radiance(direction(u, v));
                }
                texels.push((sum / 4.0).to_array());
            }
        }
        Self {
            width,
            height,
            texels,
        }
    }

    /// A Radiance `.hdr` file, as most free HDRI libraries publish them.
    /// Scaled down to at most 512 texels wide: lighting needs no more.
    pub fn from_hdr(bytes: &[u8]) -> Result<Self, String> {
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Hdr)
            .map_err(|e| format!("couldn't read the HDR image: {e}"))?
            .to_rgb32f();
        let (width, height) = image.dimensions();
        let env = Self {
            width,
            height,
            texels: image.pixels().map(|p| p.0).collect(),
        };
        Ok(env.resampled(512.min(width)))
    }

    pub fn texel(&self, x: u32, y: u32) -> Vec3 {
        Vec3::from(self.texels[(y * self.width + x) as usize])
    }

    /// Bilinear lookup by direction, wrapping round horizontally.
    pub fn sample(&self, d: Vec3) -> Vec3 {
        let (u, v) = uv(d);
        let fx = u * self.width as f32 - 0.5;
        let fy = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let wrap = |x: f32| (x as i64).rem_euclid(i64::from(self.width)) as u32;
        let (xa, xb) = (wrap(x0), wrap(x0 + 1.0));
        let (ya, yb) = (y0 as u32, (y0 as u32 + 1).min(self.height - 1));
        let top = self.texel(xa, ya).lerp(self.texel(xb, ya), tx);
        let bottom = self.texel(xa, yb).lerp(self.texel(xb, yb), tx);
        top.lerp(bottom, ty)
    }

    /// Half the size, each texel the average of the four it covers.
    pub fn downsampled(&self) -> Self {
        let (width, height) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut texels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let mut sum = Vec3::ZERO;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(self.width - 1);
                    let sy = (y * 2 + dy).min(self.height - 1);
                    sum += self.texel(sx, sy);
                }
                texels.push((sum / 4.0).to_array());
            }
        }
        Self {
            width,
            height,
            texels,
        }
    }

    /// `width` texels wide (and half that high), each texel the average of
    /// the source texels under it.
    pub fn resampled(&self, width: u32) -> Self {
        let width = width.max(2);
        let height = (width / 2).max(1);
        if (width, height) == (self.width, self.height) {
            return self.clone();
        }
        let span = |i: u32, out: u32, src: u32| {
            let a = (i as u64 * src as u64 / out as u64) as u32;
            let b = (((i + 1) as u64 * src as u64).div_ceil(out as u64) as u32).max(a + 1);
            a..b.min(src)
        };
        let mut texels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let mut sum = Vec3::ZERO;
                let mut n = 0.0;
                for sy in span(y, height, self.height) {
                    for sx in span(x, width, self.width) {
                        sum += self.texel(sx, sy);
                        n += 1.0;
                    }
                }
                texels.push((sum / f32::max(n, 1.0)).to_array());
            }
        }
        Self {
            width,
            height,
            texels,
        }
    }
}

/// The direction at equirect coordinates `u`, `v` in 0..1.
pub fn direction(u: f32, v: f32) -> Vec3 {
    let phi = (u - 0.5) * TAU;
    let theta = v * PI;
    Vec3::new(
        theta.sin() * phi.sin(),
        theta.cos(),
        theta.sin() * phi.cos(),
    )
}

/// The equirect coordinates of direction `d`; the inverse of [`direction`].
/// `scene.wgsl`'s `env_uv` must match.
pub fn uv(d: Vec3) -> (f32, f32) {
    let d = d.normalize_or(Vec3::Y);
    (d.x.atan2(d.z) / TAU + 0.5, d.y.clamp(-1.0, 1.0).acos() / PI)
}

/// The nine real spherical-harmonic basis functions (bands 0–2) at unit
/// direction `d`. `scene.wgsl`'s `irradiance` must match.
fn sh_basis(d: Vec3) -> [f32; 9] {
    [
        0.282_095,
        0.488_603 * d.y,
        0.488_603 * d.z,
        0.488_603 * d.x,
        1.092_548 * d.x * d.y,
        1.092_548 * d.y * d.z,
        0.315_392 * (3.0 * d.z * d.z - 1.0),
        1.092_548 * d.x * d.z,
        0.546_274 * (d.x * d.x - d.y * d.y),
    ]
}

/// Mip levels in the prefiltered reflection chain; level `i` is blurred
/// for roughness `i / (SPECULAR_LEVELS - 1)`.
pub const SPECULAR_LEVELS: usize = 6;
/// Width of the sharpest reflection level.
const SPECULAR_WIDTH: u32 = 256;
/// GGX samples per prefiltered texel.
const PREFILTER_SAMPLES: u32 = 64;

/// An environment made ready for the shader.
#[derive(Debug, Clone, PartialEq)]
pub struct Ibl {
    /// Identity for the renderer's upload cache: same id, same pixels.
    pub id: u64,
    /// Spherical-harmonic coefficients of the diffuse light: evaluated at a
    /// normal they give irradiance / π, the colour a white matte surface
    /// facing that way takes on.
    pub sh: [Vec3; 9],
    /// Reflections, sharpest first; each level half the size of the last.
    pub levels: Vec<Environment>,
}

impl Ibl {
    pub fn new(id: u64, env: &Environment) -> Self {
        let base = env.resampled(SPECULAR_WIDTH);
        let mut pyramid = vec![base];
        while pyramid.last().is_some_and(|e| e.width > 4) {
            let next = pyramid.last().expect("not empty").downsampled();
            pyramid.push(next);
        }
        let sh = irradiance_sh(&pyramid[2.min(pyramid.len() - 1)]);
        let levels = (0..SPECULAR_LEVELS)
            .map(|i| {
                if i == 0 {
                    return pyramid[0].clone();
                }
                let width = (SPECULAR_WIDTH >> i).max(4);
                let roughness = i as f32 / (SPECULAR_LEVELS - 1) as f32;
                prefilter(&pyramid, width, roughness)
            })
            .collect();
        Self { id, sh, levels }
    }

    /// Irradiance / π towards normal `n`, from the harmonics.
    pub fn irradiance(&self, n: Vec3) -> Vec3 {
        sh_basis(n.normalize_or(Vec3::Y))
            .iter()
            .zip(&self.sh)
            .map(|(b, c)| *c * *b)
            .sum()
    }
}

/// Project `env` onto the harmonics and convolve with the cosine lobe
/// (Ramamoorthi and Hanrahan), divided by π for a Lambertian surface.
fn irradiance_sh(env: &Environment) -> [Vec3; 9] {
    let mut coeffs = [Vec3::ZERO; 9];
    let texel_area = (TAU / env.width as f32) * (PI / env.height as f32);
    for y in 0..env.height {
        let v = (y as f32 + 0.5) / env.height as f32;
        let solid_angle = texel_area * (v * PI).sin();
        for x in 0..env.width {
            let u = (x as f32 + 0.5) / env.width as f32;
            let radiance = env.texel(x, y) * solid_angle;
            for (c, b) in coeffs.iter_mut().zip(sh_basis(direction(u, v))) {
                *c += radiance * b;
            }
        }
    }
    let band = [PI, 2.0 * PI / 3.0, PI / 4.0];
    for (i, c) in coeffs.iter_mut().enumerate() {
        let l = match i {
            0 => 0,
            1..=3 => 1,
            _ => 2,
        };
        *c *= band[l] / PI;
    }
    coeffs
}

/// One reflection level: GGX-weighted radiance around each texel's
/// direction (with the view along the normal, Karis' split-sum
/// assumption). Samples read a blurrier pyramid level the more solid angle
/// each covers, so a few dozen of them don't alias.
fn prefilter(pyramid: &[Environment], width: u32, roughness: f32) -> Environment {
    let a = (roughness * roughness).max(1e-3);
    let base = &pyramid[0];
    let texel_solid_angle = 4.0 * PI / (base.width * base.height) as f32;
    let samples: Vec<(f32, f32)> = (0..PREFILTER_SAMPLES)
        .map(|i| hammersley(i, PREFILTER_SAMPLES))
        .collect();
    let height = (width / 2).max(1);
    let mut texels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let n = direction(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let (t, b) = tangent_frame(n);
            let mut sum = Vec3::ZERO;
            let mut weight = 0.0;
            for &(e1, e2) in &samples {
                let phi = TAU * e1;
                let cos_theta = ((1.0 - e2) / (1.0 + (a * a - 1.0) * e2)).sqrt();
                let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
                let h = t * (sin_theta * phi.cos()) + b * (sin_theta * phi.sin()) + n * cos_theta;
                let l = h * (2.0 * n.dot(h)) - n;
                let nl = n.dot(l);
                if nl <= 0.0 {
                    continue;
                }
                // With n = v the sample's pdf is D / 4.
                let pdf = ggx(cos_theta, a) / 4.0;
                let sample_solid_angle = 1.0 / (PREFILTER_SAMPLES as f32 * pdf + 1e-4);
                let level = (0.5 * (sample_solid_angle / texel_solid_angle).log2() + 1.0)
                    .clamp(0.0, (pyramid.len() - 1) as f32);
                sum += pyramid[level.round() as usize].sample(l) * nl;
                weight += nl;
            }
            texels.push((sum / weight.max(1e-6)).to_array());
        }
    }
    Environment {
        width,
        height,
        texels,
    }
}

/// The GGX normal distribution at `cos_theta` between normal and half
/// vector, for alpha `a` (roughness squared).
fn ggx(cos_theta: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = cos_theta * cos_theta * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d)
}

fn hammersley(i: u32, n: u32) -> (f32, f32) {
    (
        i as f32 / n as f32,
        i.reverse_bits() as f32 / 4_294_967_296.0,
    )
}

fn tangent_frame(n: Vec3) -> (Vec3, Vec3) {
    let helper = if n.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
    let t = helper.cross(n).normalize();
    (t, n.cross(t))
}

/// A direction in the rig frame from azimuth (degrees, 0 = towards the
/// camera, negative = to its left) and elevation (degrees above the
/// horizon).
pub fn rig_direction(azimuth: f32, elevation: f32) -> Vec3 {
    let (az, el) = (azimuth.to_radians(), elevation.to_radians());
    Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos())
}

/// A rectangular light source in azimuth/elevation (degrees), with soft
/// edges, as seen from the subject.
#[derive(Debug, Clone, Copy)]
struct Softbox {
    azimuth: f32,
    elevation: f32,
    half_width: f32,
    half_height: f32,
    radiance: f32,
}

impl Softbox {
    fn at(&self, d: Vec3) -> f32 {
        let az = d.x.atan2(d.z).to_degrees();
        let el = d.y.clamp(-1.0, 1.0).asin().to_degrees();
        let daz = (az - self.azimuth + 540.0).rem_euclid(360.0) - 180.0;
        let edge = |offset: f32, half: f32| {
            let t = ((half - offset.abs()) / 4.0 + 0.5).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        // Horizontal extent shrinks towards the poles in azimuth terms.
        let half_width = self.half_width / el.to_radians().cos().max(0.2);
        self.radiance * edge(daz, half_width) * edge(el - self.elevation, self.half_height)
    }
}

/// A grey studio: a dim floor, walls brightening upwards, and softboxes.
fn studio(floor: f32, horizon: f32, ceiling: f32, boxes: &[Softbox]) -> impl Fn(Vec3) -> Vec3 + '_ {
    move |d: Vec3| {
        let base = if d.y < 0.0 {
            floor
        } else {
            horizon + (ceiling - horizon) * d.y
        };
        Vec3::splat(base + boxes.iter().map(|b| b.at(d)).sum::<f32>())
    }
}

/// Built-in lighting setups, all generated by code (no third-party HDRI
/// files to license).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LightPreset {
    /// Large soft key, gentle fill and rim strips: an even catalogue look.
    #[default]
    StudioSoft,
    /// One smaller key from the left, little fill, strong rims: more
    /// shape and drama in the folds.
    StudioContrast,
    /// Open sky with a low sun from the left.
    Daylight,
}

impl LightPreset {
    pub const ALL: [LightPreset; 3] = [
        LightPreset::StudioSoft,
        LightPreset::StudioContrast,
        LightPreset::Daylight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            LightPreset::StudioSoft => "studio_soft",
            LightPreset::StudioContrast => "studio_contrast",
            LightPreset::Daylight => "daylight",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LightPreset::StudioSoft => "Studio, soft",
            LightPreset::StudioContrast => "Studio, contrast",
            LightPreset::Daylight => "Daylight",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).expect("listed")
    }

    /// The environment image, generated at `width` texels wide.
    pub fn environment(self, width: u32) -> Environment {
        let height = width / 2;
        match self {
            LightPreset::StudioSoft => {
                let boxes = [
                    softbox(-35.0, 30.0, 32.0, 22.0, 2.4),
                    softbox(50.0, 12.0, 28.0, 26.0, 1.1),
                    softbox(0.0, 82.0, 60.0, 10.0, 0.9),
                    softbox(-140.0, 18.0, 8.0, 35.0, 1.6),
                    softbox(140.0, 18.0, 8.0, 35.0, 1.6),
                ];
                Environment::from_fn(width, height, studio(0.05, 0.12, 0.18, &boxes))
            }
            LightPreset::StudioContrast => {
                let boxes = [
                    softbox(-50.0, 38.0, 22.0, 16.0, 8.0),
                    softbox(60.0, 10.0, 20.0, 20.0, 0.35),
                    softbox(-150.0, 20.0, 6.0, 35.0, 2.5),
                    softbox(150.0, 20.0, 6.0, 35.0, 2.5),
                ];
                Environment::from_fn(width, height, studio(0.015, 0.03, 0.05, &boxes))
            }
            LightPreset::Daylight => Environment::from_fn(width, height, |d| {
                if d.y < 0.0 {
                    Vec3::new(0.24, 0.21, 0.17) * 0.6
                } else {
                    let horizon = Vec3::new(0.62, 0.70, 0.82);
                    let zenith = Vec3::new(0.22, 0.38, 0.75);
                    horizon.lerp(zenith, d.y.sqrt()) * 0.7
                }
            }),
        }
    }

    /// The preset ready to render, built once per process.
    pub fn lighting(self) -> Lighting {
        static CACHE: [OnceLock<Arc<Ibl>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
        let ibl = CACHE[self.index()]
            .get_or_init(|| {
                Arc::new(Ibl::new(
                    0x11_0000 + self.index() as u64,
                    &self.environment(512),
                ))
            })
            .clone();
        let (key, key_intensity) = match self {
            LightPreset::StudioSoft => (rig_direction(-35.0, 30.0), 0.55),
            LightPreset::StudioContrast => (rig_direction(-50.0, 38.0), 1.3),
            LightPreset::Daylight => (rig_direction(-60.0, 45.0), 2.6),
        };
        Lighting {
            ibl,
            key,
            key_intensity,
            environment_intensity: 1.0,
        }
        .normalized()
    }
}

fn softbox(
    azimuth: f32,
    elevation: f32,
    half_width: f32,
    half_height: f32,
    radiance: f32,
) -> Softbox {
    Softbox {
        azimuth,
        elevation,
        half_width,
        half_height,
        radiance,
    }
}

/// How a final render is lit: an environment plus a direct key light
/// (sharp shading the environment's soft light can't give).
#[derive(Debug, Clone, PartialEq)]
pub struct Lighting {
    pub ibl: Arc<Ibl>,
    /// Unit direction towards the key light, in the rig frame.
    pub key: Vec3,
    pub key_intensity: f32,
    /// Scales the environment's light (diffuse and reflections).
    pub environment_intensity: f32,
}

/// How bright a white matte surface gets facing the brightest way, after
/// [`Lighting::normalized`]: just under the point where tone mapping
/// starts rolling highlights off, so white cloth keeps its shading.
pub const PEAK_LIGHT: f32 = 0.95;

impl Lighting {
    /// Lit by an HDR image alone, with no extra lights, at a brightness
    /// set by [`Self::normalized`] (HDR files differ wildly in scale).
    pub fn from_environment(id: u64, env: &Environment) -> Self {
        Self {
            ibl: Arc::new(Ibl::new(id, env)),
            key: Vec3::Y,
            key_intensity: 0.0,
            environment_intensity: 1.0,
        }
        .normalized()
    }

    /// Diffuse light on a white matte surface facing `n` (rig frame).
    pub fn light_towards(&self, n: Vec3) -> f32 {
        let n = n.normalize_or(Vec3::Y);
        let env = self.ibl.irradiance(n).max_element() * self.environment_intensity;
        env + self.key_intensity * n.dot(self.key).max(0.0)
    }

    /// Scaled so the brightest-lit direction gets [`PEAK_LIGHT`]; the
    /// exposure setting then works the same for every setup.
    pub fn normalized(mut self) -> Self {
        // Directions spread evenly over the sphere (a Fibonacci spiral).
        const N: usize = 256;
        let golden = PI * (3.0 - 5f32.sqrt());
        let peak = (0..N)
            .map(|i| {
                let y = 1.0 - 2.0 * (i as f32 + 0.5) / N as f32;
                let r = (1.0 - y * y).sqrt();
                let a = golden * i as f32;
                self.light_towards(Vec3::new(r * a.cos(), y, r * a.sin()))
            })
            .fold(0.0f32, f32::max);
        if peak > 1e-6 {
            let scale = PEAK_LIGHT / peak;
            self.key_intensity *= scale;
            self.environment_intensity *= scale;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(width: u32, value: f32) -> Environment {
        Environment::from_fn(width, width / 2, |_| Vec3::splat(value))
    }

    #[test]
    fn direction_and_uv_round_trip() {
        for d in [
            Vec3::Z,
            Vec3::X,
            -Vec3::X,
            Vec3::new(0.3, 0.5, -0.8).normalize(),
            Vec3::new(-0.2, -0.9, 0.1).normalize(),
        ] {
            let (u, v) = uv(d);
            assert!(direction(u, v).distance(d) < 1e-4, "{d:?}");
        }
        // The camera side sits in the middle of the image, up at the top.
        assert!((uv(Vec3::Z).0 - 0.5).abs() < 1e-6);
        assert!(uv(Vec3::Y).1 < 1e-3);
    }

    /// A uniform environment of radiance L lights a matte surface to L
    /// from any side: irradiance / π = L.
    #[test]
    fn uniform_light_gives_its_radiance_from_every_side() {
        let ibl = Ibl::new(1, &uniform(64, 0.5));
        for n in [Vec3::Y, -Vec3::Y, Vec3::X, Vec3::new(0.4, -0.3, 0.8)] {
            let e = ibl.irradiance(n);
            assert!(
                (e - Vec3::splat(0.5)).abs().max_element() < 0.02,
                "{n:?}: {e:?}"
            );
        }
        for level in &ibl.levels {
            for t in &level.texels {
                assert!((t[0] - 0.5).abs() < 0.02, "{t:?}");
            }
        }
    }

    /// Light from above only: an upward face gets it all, a downward one
    /// none, a sideways one about half.
    #[test]
    fn irradiance_follows_where_the_light_is() {
        let ibl = Ibl::new(
            2,
            &Environment::from_fn(64, 32, |d| Vec3::splat(if d.y > 0.0 { 1.0 } else { 0.0 })),
        );
        let up = ibl.irradiance(Vec3::Y).x;
        let side = ibl.irradiance(Vec3::X).x;
        let down = ibl.irradiance(-Vec3::Y).x;
        assert!((up - 1.0).abs() < 0.08, "up {up}");
        assert!((side - 0.5).abs() < 0.05, "side {side}");
        assert!(down < 0.08, "down {down}");
    }

    /// Rougher levels spread a small bright source out: its peak drops
    /// and the light reaches further from it.
    #[test]
    fn rougher_reflections_are_blurrier() {
        let env = Environment::from_fn(256, 128, |d| {
            Vec3::splat(if d.dot(Vec3::Z) > 0.98 { 20.0 } else { 0.0 })
        });
        let ibl = Ibl::new(3, &env);
        let peak = |i: usize| ibl.levels[i].sample(Vec3::Z).x;
        let off = |i: usize| ibl.levels[i].sample(rig_direction(40.0, 0.0)).x;
        assert!(
            peak(0) > peak(2) && peak(2) > peak(5),
            "{} {} {}",
            peak(0),
            peak(2),
            peak(5)
        );
        assert!(off(5) > off(1), "{} {}", off(1), off(5));
        assert_eq!(ibl.levels.len(), SPECULAR_LEVELS);
        assert_eq!(ibl.levels[1].width, SPECULAR_WIDTH / 2);
    }

    #[test]
    fn resampling_keeps_the_average() {
        let env = Environment::from_fn(300, 150, |d| Vec3::splat(d.y.max(0.0)));
        let mean =
            |e: &Environment| e.texels.iter().map(|t| t[0]).sum::<f32>() / e.texels.len() as f32;
        let small = env.resampled(64);
        assert_eq!((small.width, small.height), (64, 32));
        assert!((mean(&small) - mean(&env)).abs() < 0.01);
    }

    /// Every preset lights from above more than below; the studios also
    /// light the camera side more than the back (the open sky is even all
    /// round, its sun is the key light).
    #[test]
    fn presets_are_named_and_lit_from_above_and_the_front() {
        for preset in LightPreset::ALL {
            assert_eq!(LightPreset::parse(preset.name()), Some(preset));
            let lighting = preset.lighting();
            let at = |d: Vec3| lighting.ibl.irradiance(d).length();
            assert!(at(Vec3::Y) > at(-Vec3::Y), "{preset:?}");
            if preset != LightPreset::Daylight {
                let (front, back) = (at(Vec3::Z), at(-Vec3::Z));
                assert!(front > back, "{preset:?}: {front} vs {back}");
            }
            assert!(lighting.key_intensity > 0.0 && lighting.key.y > 0.0);
        }
        assert_eq!(LightPreset::parse("disco"), None);
    }

    /// Presets and HDR images of any brightness come out at the same peak.
    #[test]
    fn lighting_is_normalized_to_the_same_peak() {
        let peak = |l: &Lighting| {
            (0..2000)
                .map(|i| {
                    let (u, v) = (i as f32 * 0.618_034 % 1.0, (i as f32 + 0.5) / 2000.0);
                    l.light_towards(direction(u, v))
                })
                .fold(0.0f32, f32::max)
        };
        for preset in LightPreset::ALL {
            let p = peak(&preset.lighting());
            assert!((p - PEAK_LIGHT).abs() < 0.05, "{preset:?}: {p}");
        }
        let dim = Lighting::from_environment(7, &uniform(64, 0.01));
        let bright = Lighting::from_environment(8, &uniform(64, 300.0));
        assert!((peak(&dim) - PEAK_LIGHT).abs() < 0.05);
        assert!((peak(&dim) - peak(&bright)).abs() < 1e-3);
    }

    #[test]
    fn reads_a_radiance_hdr_file() {
        let pixels: Vec<image::Rgb<f32>> = (0..8 * 4)
            .map(|i| image::Rgb([i as f32 / 10.0, 1.0, 2.0]))
            .collect();
        let mut bytes = Vec::new();
        image::codecs::hdr::HdrEncoder::new(&mut bytes)
            .encode(&pixels, 8, 4)
            .unwrap();
        let env = Environment::from_hdr(&bytes).unwrap();
        assert_eq!((env.width, env.height), (8, 4));
        assert!((env.texel(0, 0)[2] - 2.0).abs() < 0.05);
        assert!(Environment::from_hdr(b"not an image").is_err());
    }
}
