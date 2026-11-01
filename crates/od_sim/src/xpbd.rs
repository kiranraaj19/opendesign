//! A small XPBD (Extended Position Based Dynamics) solver: particles,
//! distance constraints and pinning, stepped with substeps the way
//! Muller et al.'s XPBD paper describes. Collision is layered on top by
//! [`crate::collision`].

use glam::Vec3;

/// A simulated point mass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    /// `0.0` pins the particle: it ignores forces and constraints move
    /// everything else to it instead.
    pub inv_mass: f32,
    /// Coefficient of friction of its fabric, 0..1.
    pub friction: f32,
    /// Half the thickness its fabric keeps from other cloth, in metres.
    pub radius: f32,
}

/// Friction and collision radius of a particle that wasn't given its
/// fabric's: a medium cotton.
pub const DEFAULT_FRICTION: f32 = 0.3;
pub const DEFAULT_RADIUS: f32 = 0.0015;

/// Static friction is this much stronger than sliding friction: cloth at
/// rest needs a little more to get going.
const STATIC_FACTOR: f32 = 1.25;

impl Particle {
    pub fn new(position: Vec3, inv_mass: f32) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            inv_mass,
            friction: DEFAULT_FRICTION,
            radius: DEFAULT_RADIUS,
        }
    }

    pub fn pinned(position: Vec3) -> Self {
        Self::new(position, 0.0)
    }
}

/// Keeps two particles a fixed distance apart, softened by `compliance`
/// (inverse stiffness, in m/N; `0.0` is rigid). A rest length of `0.0`
/// pulls the particles together, which is how a seam is stitched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceConstraint {
    pub a: usize,
    pub b: usize,
    pub rest_length: f32,
    pub compliance: f32,
}

impl DistanceConstraint {
    pub fn new(a: usize, b: usize, rest_length: f32, compliance: f32) -> Self {
        Self {
            a,
            b,
            rest_length,
            compliance,
        }
    }

    /// One XPBD position correction. `lambda` is this constraint's running
    /// Lagrange multiplier for the current substep; `alpha_tilde` is its
    /// compliance scaled by the substep time step squared.
    fn solve(&self, particles: &mut [Particle], lambda: &mut f32, alpha_tilde: f32) {
        let pa = particles[self.a].position;
        let pb = particles[self.b].position;
        let wa = particles[self.a].inv_mass;
        let wb = particles[self.b].inv_mass;
        let w_sum = wa + wb;
        if w_sum == 0.0 {
            return;
        }
        let delta = pb - pa;
        let dist = delta.length();
        if dist < 1e-9 {
            return;
        }
        let dir = delta / dist;
        let c = dist - self.rest_length;
        let dlambda = (-c - alpha_tilde * *lambda) / (w_sum + alpha_tilde);
        *lambda += dlambda;
        particles[self.a].position -= dir * (wa * dlambda);
        particles[self.b].position += dir * (wb * dlambda);
    }
}

/// A point on the cloth's surface: a weighted mix of up to three
/// particles (a triangle and barycentric weights), so it can sit between
/// particles and still move with them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePoint {
    pub particles: [usize; 3],
    pub weights: [f32; 3],
}

impl SurfacePoint {
    /// Exactly at one particle.
    pub fn at(particle: usize) -> Self {
        Self {
            particles: [particle; 3],
            weights: [1.0, 0.0, 0.0],
        }
    }

    pub fn position(&self, particles: &[Particle]) -> Vec3 {
        self.particles
            .iter()
            .zip(self.weights)
            .map(|(&i, w)| particles[i].position * w)
            .sum()
    }
}

/// Pulls two surface points together (rest length zero), softened by
/// `compliance`: a tack that can hold anywhere on a triangle, not only at
/// a particle. The correction is shared among each side's particles by
/// their weights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceStitch {
    pub a: SurfacePoint,
    pub b: SurfacePoint,
    pub compliance: f32,
}

impl SurfaceStitch {
    fn solve(&self, particles: &mut [Particle], lambda: &mut f32, alpha_tilde: f32) {
        let delta = self.b.position(particles) - self.a.position(particles);
        let dist = delta.length();
        if dist < 1e-9 {
            return;
        }
        let dir = delta / dist;
        let side_weight = |s: &SurfacePoint, particles: &[Particle]| -> f32 {
            s.particles
                .iter()
                .zip(s.weights)
                .map(|(&i, w)| w * w * particles[i].inv_mass)
                .sum()
        };
        let w_sum = side_weight(&self.a, particles) + side_weight(&self.b, particles);
        if w_sum == 0.0 {
            return;
        }
        let dlambda = (-dist - alpha_tilde * *lambda) / (w_sum + alpha_tilde);
        *lambda += dlambda;
        for (&i, w) in self.a.particles.iter().zip(self.a.weights) {
            let m = particles[i].inv_mass;
            particles[i].position -= dir * (w * m * dlambda);
        }
        for (&i, w) in self.b.particles.iter().zip(self.b.weights) {
            let m = particles[i].inv_mass;
            particles[i].position += dir * (w * m * dlambda);
        }
    }
}

/// A particle that must not move (e.g. a user-placed pin, or an anchor to
/// the avatar). Enforced by snapping back every substep, so it holds even
/// through a particle with nonzero inverse mass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pin {
    pub particle: usize,
    pub target: Vec3,
}

/// A cloth system: particles, the constraints between them, and the
/// physical constants the solver steps with.
#[derive(Debug, Clone)]
pub struct World {
    pub particles: Vec<Particle>,
    pub distance_constraints: Vec<DistanceConstraint>,
    pub surface_stitches: Vec<SurfaceStitch>,
    pub pins: Vec<Pin>,
    /// Keeps the cloth from passing through itself, if set.
    pub self_collision: Option<crate::self_collision::SelfCollision>,
    pub gravity: Vec3,
    /// Fraction of velocity kept each full step (`1.0` = no damping).
    pub damping: f32,
    /// How much of each surface's friction applies, 0..1: none while seams
    /// are being sewn, so pieces can slide into place round the body.
    pub friction: f32,
}

impl World {
    pub fn new(particles: Vec<Particle>) -> Self {
        Self {
            particles,
            distance_constraints: Vec::new(),
            surface_stitches: Vec::new(),
            pins: Vec::new(),
            self_collision: None,
            gravity: Vec3::new(0.0, -9.81, 0.0),
            damping: 0.995,
            friction: 1.0,
        }
    }

    /// Advance the whole system by `dt` seconds, split into `substeps`
    /// XPBD substeps (more substeps stiffen constraints without needing a
    /// smaller `dt`). `colliders` are resolved once per substep, after the
    /// constraint pass. Call every frame with the frame's `dt`.
    pub fn step(&mut self, dt: f32, substeps: u32, colliders: &[crate::collision::Collider]) {
        let substeps = substeps.max(1);
        let h = dt / substeps as f32;
        for _ in 0..substeps {
            self.substep(h, colliders);
        }
    }

    fn substep(&mut self, h: f32, colliders: &[crate::collision::Collider]) {
        let prev_positions: Vec<Vec3> = self.particles.iter().map(|p| p.position).collect();

        for p in &mut self.particles {
            if p.inv_mass > 0.0 {
                p.velocity += self.gravity * h;
                p.position += p.velocity * h;
            }
        }

        let mut lambdas = vec![0.0_f32; self.distance_constraints.len()];
        for (c, lambda) in self.distance_constraints.iter().zip(lambdas.iter_mut()) {
            let alpha_tilde = c.compliance / (h * h);
            c.solve(&mut self.particles, lambda, alpha_tilde);
        }
        let mut lambdas = vec![0.0_f32; self.surface_stitches.len()];
        for (c, lambda) in self.surface_stitches.iter().zip(lambdas.iter_mut()) {
            let alpha_tilde = c.compliance / (h * h);
            c.solve(&mut self.particles, lambda, alpha_tilde);
        }

        if let Some(collision) = &self.self_collision {
            collision.resolve(&mut self.particles, &prev_positions, self.friction);
        }

        for pin in &self.pins {
            self.particles[pin.particle].position = pin.target;
        }

        for collider in colliders {
            let surface_friction = collider.friction() * self.friction;
            for (p, prev) in self.particles.iter_mut().zip(&prev_positions) {
                if p.inv_mass == 0.0 {
                    continue;
                }
                if let Some((out, normal)) = collider.contact(p.position) {
                    let depth = (out - p.position).dot(normal).max(0.0);
                    p.position = out;
                    let mu = (p.friction * surface_friction).max(0.0).sqrt();
                    p.position -= friction_correction(p.position - *prev, normal, depth, mu);
                }
            }
        }

        for (p, prev) in self.particles.iter_mut().zip(prev_positions) {
            if p.inv_mass > 0.0 {
                p.velocity = (p.position - prev) / h * self.damping;
            } else {
                p.velocity = Vec3::ZERO;
            }
        }
    }
}

/// How far to move a particle back along a surface it touches: its
/// movement this substep along the surface (`moved`, less the part along
/// `normal`), all of it if that's within static friction of how far the
/// contact pushed it out (`depth`, which stands for the normal force),
/// otherwise as much as sliding friction takes. Coulomb friction in
/// position form, as in Macklin et al.'s unified particle physics.
pub(crate) fn friction_correction(moved: Vec3, normal: Vec3, depth: f32, mu: f32) -> Vec3 {
    let along = moved - normal * moved.dot(normal);
    let slide = along.length();
    if slide <= 1e-12 || mu <= 0.0 || depth <= 0.0 {
        return Vec3::ZERO;
    }
    if slide < mu * STATIC_FACTOR * depth {
        along
    } else {
        along * (mu * depth / slide).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::Collider;

    fn settle(world: &mut World, seconds: f32) {
        let dt = 1.0 / 60.0;
        let steps = (seconds / dt) as u32;
        for _ in 0..steps {
            world.step(dt, 8, &[]);
        }
    }

    #[test]
    fn free_particle_falls_under_gravity() {
        let mut world = World::new(vec![Particle::new(Vec3::ZERO, 1.0)]);
        settle(&mut world, 0.5);
        assert!(world.particles[0].position.y < -0.5);
    }

    #[test]
    fn pinned_particle_does_not_move() {
        let mut world = World::new(vec![Particle::pinned(Vec3::new(1.0, 2.0, 3.0))]);
        settle(&mut world, 1.0);
        assert_eq!(world.particles[0].position, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn pin_holds_a_massive_particle_in_place() {
        let mut world = World::new(vec![Particle::new(Vec3::new(5.0, 5.0, 5.0), 1.0)]);
        world.pins.push(Pin {
            particle: 0,
            target: Vec3::new(5.0, 5.0, 5.0),
        });
        settle(&mut world, 1.0);
        assert_eq!(world.particles[0].position, Vec3::new(5.0, 5.0, 5.0));
    }

    #[test]
    fn rigid_distance_constraint_keeps_particles_apart() {
        let mut world = World::new(vec![
            Particle::pinned(Vec3::new(0.0, 0.0, 0.0)),
            Particle::new(Vec3::new(0.5, 0.0, 0.0), 1.0),
        ]);
        world
            .distance_constraints
            .push(DistanceConstraint::new(0, 1, 1.0, 0.0));
        settle(&mut world, 2.0);
        let dist = world.particles[0]
            .position
            .distance(world.particles[1].position);
        assert!((dist - 1.0).abs() < 1e-2, "settled distance {dist}");
    }

    #[test]
    fn soft_stitch_pulls_two_particles_together() {
        let mut world = World::new(vec![
            Particle::new(Vec3::new(-0.5, 2.0, 0.0), 1.0),
            Particle::new(Vec3::new(0.5, 2.0, 0.0), 1.0),
        ]);
        world
            .distance_constraints
            .push(DistanceConstraint::new(0, 1, 0.0, 1e-6));
        settle(&mut world, 2.0);
        let dist = world.particles[0]
            .position
            .distance(world.particles[1].position);
        assert!(dist < 0.05, "stitched distance {dist}");
    }

    #[test]
    fn surface_stitch_pulls_a_point_between_particles_to_another() {
        // A rigid bar of two particles, and a free particle tacked to the
        // bar's midpoint.
        let mut world = World::new(vec![
            Particle::pinned(Vec3::new(0.0, 2.0, 0.0)),
            Particle::pinned(Vec3::new(1.0, 2.0, 0.0)),
            Particle::new(Vec3::new(0.5, 1.0, 0.0), 1.0),
        ]);
        let mid = SurfacePoint {
            particles: [0, 1, 1],
            weights: [0.5, 0.5, 0.0],
        };
        assert_eq!(mid.position(&world.particles), Vec3::new(0.5, 2.0, 0.0));
        world.surface_stitches.push(SurfaceStitch {
            a: mid,
            b: SurfacePoint::at(2),
            compliance: 1e-7,
        });
        settle(&mut world, 1.0);
        let gap = world.particles[2]
            .position
            .distance(Vec3::new(0.5, 2.0, 0.0));
        assert!(gap < 0.01, "{gap}");
    }

    #[test]
    fn ground_collider_stops_cloth_falling_through() {
        let mut world = World::new(vec![Particle::new(Vec3::new(0.0, 1.0, 0.0), 1.0)]);
        let colliders = [Collider::ground(0.0)];
        for _ in 0..180 {
            world.step(1.0 / 60.0, 8, &colliders);
        }
        assert!(world.particles[0].position.y >= -1e-3);
    }

    /// A particle resting on a slope tilted `degrees` from flat, after
    /// half a second: how far it slid down.
    fn slide_on_slope(degrees: f32, friction: f32) -> f32 {
        let (s, c) = degrees.to_radians().sin_cos();
        let normal = Vec3::new(-s, c, 0.0);
        let start = normal * 0.004;
        let mut p = Particle::new(start, 1.0);
        p.friction = friction;
        let mut world = World::new(vec![p]);
        world.damping = 1.0;
        // A long slope through the origin, as a mesh of two triangles.
        let along = Vec3::new(c, s, 0.0);
        let side = Vec3::Z;
        let corners = [
            -along * 10.0 - side * 2.0,
            along * 10.0 - side * 2.0,
            along * 10.0 + side * 2.0,
            -along * 10.0 + side * 2.0,
        ];
        // Counter-clockwise seen from the normal's side.
        let winding = (corners[2] - corners[0]).cross(corners[1] - corners[0]);
        let indices = if winding.dot(normal) < 0.0 {
            vec![0, 1, 2, 0, 2, 3]
        } else {
            vec![0, 2, 1, 0, 3, 2]
        };
        let mesh = od_render::mesh::Mesh {
            positions: corners.to_vec(),
            normals: vec![normal; 4],
            indices,
        };
        let colliders = [Collider::mesh(mesh, 0.003).with_friction(friction)];
        for _ in 0..30 {
            world.step(1.0 / 60.0, 8, &colliders);
        }
        (start - world.particles[0].position).dot(along)
    }

    #[test]
    fn friction_holds_cloth_on_a_gentle_slope_and_lets_it_slide_on_a_steep_one() {
        // tan 15° = 0.27 < 0.5: it stays put.
        let held = slide_on_slope(15.0, 0.5);
        assert!(held.abs() < 0.002, "slid {held}");
        // Without friction it slides off the same slope.
        let free = slide_on_slope(15.0, 0.0);
        assert!(free > 0.2, "slid only {free}");
        // tan 45° = 1 > 0.5: it slides, but slower than without friction.
        let steep = slide_on_slope(45.0, 0.5);
        let steep_free = slide_on_slope(45.0, 0.0);
        assert!(
            steep > 0.2 && steep < steep_free * 0.8,
            "{steep} vs {steep_free}"
        );
    }

    #[test]
    fn friction_only_removes_movement_along_the_surface() {
        let n = Vec3::Y;
        // Sticks: sideways movement below mu × depth goes entirely.
        let c = friction_correction(Vec3::new(0.001, -0.5, 0.0), n, 0.01, 0.5);
        assert!((c - Vec3::new(0.001, 0.0, 0.0)).length() < 1e-7);
        // Slides: only mu × depth of it.
        let c = friction_correction(Vec3::new(0.1, 0.0, 0.0), n, 0.01, 0.5);
        assert!((c.x - 0.005).abs() < 1e-6 && c.y == 0.0);
        assert_eq!(friction_correction(Vec3::X, n, 0.0, 0.5), Vec3::ZERO);
    }
}
