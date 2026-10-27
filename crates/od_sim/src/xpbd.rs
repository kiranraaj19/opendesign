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
}

impl Particle {
    pub fn new(position: Vec3, inv_mass: f32) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            inv_mass,
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
    pub pins: Vec<Pin>,
    pub gravity: Vec3,
    /// Fraction of velocity kept each full step (`1.0` = no damping).
    pub damping: f32,
}

impl World {
    pub fn new(particles: Vec<Particle>) -> Self {
        Self {
            particles,
            distance_constraints: Vec::new(),
            pins: Vec::new(),
            gravity: Vec3::new(0.0, -9.81, 0.0),
            damping: 0.995,
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

        for pin in &self.pins {
            self.particles[pin.particle].position = pin.target;
        }

        for collider in colliders {
            for p in &mut self.particles {
                if p.inv_mass > 0.0 {
                    p.position = collider.resolve(p.position);
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
    fn ground_collider_stops_cloth_falling_through() {
        let mut world = World::new(vec![Particle::new(Vec3::new(0.0, 1.0, 0.0), 1.0)]);
        let colliders = [Collider::ground(0.0)];
        for _ in 0..180 {
            world.step(1.0 / 60.0, 8, &colliders);
        }
        assert!(world.particles[0].position.y >= -1e-3);
    }
}
