//! Keeps cloth from passing through cloth: every free particle is pushed
//! out to `thickness` from any triangle it comes near that isn't its own,
//! on the side it was on a substep earlier. Particles held by seams and
//! tacks, and the triangles around them, are left out so a seam isn't
//! pushed apart by the very panels it joins.

use glam::Vec3;

use crate::collision::closest_on_triangle;
use crate::xpbd::Particle;

/// Particle-against-triangle collision between the cloth's own triangles.
#[derive(Debug, Clone)]
pub struct SelfCollision {
    pub enabled: bool,
    /// How far apart two layers of cloth are kept, in metres.
    pub thickness: f32,
    /// Every cloth triangle, by global particle index.
    triangles: Vec<[usize; 3]>,
    /// Particles that neither collide nor are collided with.
    skip: Vec<bool>,
    /// Triangles with no skipped corner: the ones that collide.
    active: Vec<u32>,
}

/// Cloth thickness for self-collision, in metres.
pub const DEFAULT_THICKNESS: f32 = 0.003;

type Cell = (i32, i32, i32);

/// Which spatial-hash bucket a cell falls in, before masking to the
/// table size.
fn bucket_of((x, y, z): Cell) -> usize {
    let h = (x as u32).wrapping_mul(73_856_093)
        ^ (y as u32).wrapping_mul(19_349_663)
        ^ (z as u32).wrapping_mul(83_492_791);
    h as usize
}

impl SelfCollision {
    /// Collide `triangles` (global particle indices) with the particles
    /// they're made of. Particles where `skip` is true (seam and tack
    /// ends) are left out, with every triangle touching them.
    pub fn new(triangles: Vec<[usize; 3]>, skip: Vec<bool>, thickness: f32) -> Self {
        let mut this = Self {
            enabled: true,
            thickness,
            triangles,
            skip,
            active: Vec::new(),
        };
        this.update_active();
        this
    }

    /// Change which particles are left out, e.g. after tacks were added.
    pub fn set_skip(&mut self, skip: Vec<bool>) {
        self.skip = skip;
        self.update_active();
    }

    fn skipped(&self, i: usize) -> bool {
        self.skip.get(i).copied().unwrap_or(true)
    }

    fn update_active(&mut self) {
        self.active = (0..self.triangles.len() as u32)
            .filter(|&t| !self.triangles[t as usize].iter().any(|&i| self.skipped(i)))
            .collect();
    }

    /// Push particles out of triangles they're closer than `thickness` to.
    /// `prev` holds every particle's position at the start of the substep,
    /// which says which side of a triangle a particle belongs on.
    pub fn resolve(&self, particles: &mut [Particle], prev: &[Vec3]) {
        if !self.enabled || self.active.is_empty() {
            return;
        }

        // Each triangle goes in every grid cell its bounds (widened by how
        // far a particle may be from it and still collide) overlap, so a
        // particle only needs to look in its own cell.
        let mut moved = 0.0_f32;
        for (p, q) in particles.iter().zip(prev) {
            moved = moved.max(p.position.distance(*q));
        }
        let margin = self.thickness + moved;
        // Each triangle's bounds, widened by how far a particle may be from
        // it and still collide.
        let boxes: Vec<(Vec3, Vec3)> = self
            .active
            .iter()
            .map(|&t| {
                let [a, b, c] = self.triangles[t as usize].map(|i| particles[i].position);
                (
                    a.min(b).min(c) - Vec3::splat(margin),
                    a.max(b).max(c) + Vec3::splat(margin),
                )
            })
            .collect();
        let extent: f32 = boxes.iter().map(|(lo, hi)| (*hi - *lo).max_element()).sum();
        let cell_size = (extent / boxes.len() as f32).max(1e-4);
        let cell_of = |p: Vec3| -> Cell {
            let v = p / cell_size;
            (v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32)
        };
        // A spatial hash in flat arrays (counting sort into buckets): each
        // triangle goes in every cell its bounds overlap, so a particle
        // only looks in its own. Cells that share a bucket only add
        // candidates the bounds and exact tests below reject.
        let ranges: Vec<(Cell, Cell)> = boxes
            .iter()
            .map(|(lo, hi)| (cell_of(*lo), cell_of(*hi)))
            .collect();
        let cells = |(lo, hi): (Cell, Cell)| {
            (lo.0..=hi.0).flat_map(move |x| {
                (lo.1..=hi.1).flat_map(move |y| (lo.2..=hi.2).map(move |z| (x, y, z)))
            })
        };
        let entries: usize = ranges
            .iter()
            .map(|(lo, hi)| ((hi.0 - lo.0 + 1) * (hi.1 - lo.1 + 1) * (hi.2 - lo.2 + 1)) as usize)
            .sum();
        let buckets = (2 * entries).next_power_of_two();
        let bucket = |c: Cell| bucket_of(c) & (buckets - 1);
        let mut start = vec![0u32; buckets + 1];
        for &range in &ranges {
            for c in cells(range) {
                start[bucket(c) + 1] += 1;
            }
        }
        for k in 0..buckets {
            start[k + 1] += start[k];
        }
        let mut fill = start.clone();
        let mut slots = vec![0u32; entries];
        for (k, &range) in ranges.iter().enumerate() {
            for c in cells(range) {
                let b = bucket(c);
                slots[fill[b] as usize] = k as u32;
                fill[b] += 1;
            }
        }

        for i in 0..particles.len().min(self.skip.len()) {
            if self.skip[i] || particles[i].inv_mass == 0.0 {
                continue;
            }
            let p = particles[i].position;
            let b = bucket(cell_of(p));
            for s in start[b]..start[b + 1] {
                let k = slots[s as usize] as usize;
                let (lo, hi) = boxes[k];
                if p.cmplt(lo).any() || p.cmpgt(hi).any() {
                    continue;
                }
                let tri = self.triangles[self.active[k] as usize];
                if !tri.contains(&i) {
                    self.push_out(particles, prev, i, tri);
                }
            }
        }
    }

    /// One particle against one triangle.
    fn push_out(&self, particles: &mut [Particle], prev: &[Vec3], i: usize, tri: [usize; 3]) {
        let [a, b, c] = tri.map(|j| particles[j].position);
        let p = particles[i].position;
        let normal = (b - a).cross(c - a);
        let len = normal.length();
        if len < 1e-12 {
            return;
        }
        let n = normal / len;
        let reach = self.thickness + p.distance(prev[i]);
        // Too far from the triangle's plane to touch it.
        if (p - a).dot(n).abs() >= reach {
            return;
        }
        let q = closest_on_triangle(a, b, c, p);
        let gap = p - q;
        if gap.length() >= reach {
            return;
        }
        // Only where the particle is over the face (or just off its edge),
        // not beside it: past the edge the neighbouring triangle, or
        // nothing, is in the way.
        let over = p - n * (p - a).dot(n);
        let weights = od_render::pick::triangle_weights([a, b, c], over);
        if weights.iter().any(|&w| w < -0.01) {
            return;
        }
        // Keep the particle on the side it came from.
        let before = (prev[i] - a).dot(n);
        let side = if before.abs() > 1e-9 {
            before.signum()
        } else {
            gap.dot(n).signum()
        };
        let depth = self.thickness - gap.dot(n) * side;
        if depth <= 0.0 {
            return;
        }
        let weights = od_render::pick::triangle_weights([a, b, c], q);
        let wp = particles[i].inv_mass;
        let w_tri: f32 = tri
            .iter()
            .zip(weights)
            .map(|(&j, w)| w * w * particles[j].inv_mass)
            .sum();
        let total = wp + w_tri;
        if total <= 0.0 {
            return;
        }
        let push = n * side * (depth / total);
        particles[i].position += push * wp;
        for (&j, w) in tri.iter().zip(weights) {
            let m = particles[j].inv_mass;
            particles[j].position -= push * (w * m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A horizontal square of two triangles at height `y`, as particles
    /// `base..base + 4`.
    fn square(particles: &mut Vec<Particle>, y: f32, inv_mass: f32) -> Vec<[usize; 3]> {
        let base = particles.len();
        for (x, z) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            particles.push(Particle::new(Vec3::new(x, y, z), inv_mass));
        }
        vec![[base, base + 1, base + 2], [base, base + 2, base + 3]]
    }

    #[test]
    fn a_particle_falling_onto_cloth_stops_on_top() {
        let mut particles = Vec::new();
        let tris = square(&mut particles, 0.0, 0.0);
        particles.push(Particle::new(Vec3::new(0.4, 0.002, 0.6), 1.0));
        let prev: Vec<Vec3> = particles
            .iter()
            .map(|p| p.position + Vec3::Y * 0.01)
            .collect();
        let collision = SelfCollision::new(tris, vec![false; 5], 0.003);
        collision.resolve(&mut particles, &prev);
        assert!(
            (particles[4].position.y - 0.003).abs() < 1e-6,
            "{}",
            particles[4].position
        );
    }

    #[test]
    fn a_particle_that_passed_through_goes_back_to_its_side() {
        let mut particles = Vec::new();
        let tris = square(&mut particles, 0.0, 0.0);
        // Was above, is now just below: pushed back above.
        particles.push(Particle::new(Vec3::new(0.4, -0.002, 0.6), 1.0));
        let mut prev: Vec<Vec3> = particles.iter().map(|p| p.position).collect();
        prev[4] = Vec3::new(0.4, 0.004, 0.6);
        let collision = SelfCollision::new(tris, vec![false; 5], 0.003);
        collision.resolve(&mut particles, &prev);
        assert!(
            (particles[4].position.y - 0.003).abs() < 1e-6,
            "{}",
            particles[4].position
        );
    }

    #[test]
    fn skipped_and_far_particles_are_left_alone() {
        let mut particles = Vec::new();
        let tris = square(&mut particles, 0.0, 0.0);
        particles.push(Particle::new(Vec3::new(0.4, 0.001, 0.6), 1.0));
        particles.push(Particle::new(Vec3::new(0.4, 0.5, 0.6), 1.0));
        let prev: Vec<Vec3> = particles.iter().map(|p| p.position).collect();
        let mut skip = vec![false; 6];
        skip[4] = true;
        let collision = SelfCollision::new(tris, skip, 0.003);
        let before = particles.clone();
        collision.resolve(&mut particles, &prev);
        assert_eq!(particles, before);
    }

    #[test]
    fn two_free_layers_share_the_correction() {
        let mut particles = Vec::new();
        let mut tris = square(&mut particles, 0.0, 1.0);
        tris.extend(square(&mut particles, 0.001, 1.0));
        let prev: Vec<Vec3> = particles.iter().map(|p| p.position).collect();
        let collision = SelfCollision::new(tris, vec![false; 8], 0.003);
        // Each pass gets part of the way, as each solver substep does.
        for _ in 0..20 {
            collision.resolve(&mut particles, &prev);
        }
        let lower = particles[..4].iter().map(|p| p.position.y).sum::<f32>() / 4.0;
        let upper = particles[4..].iter().map(|p| p.position.y).sum::<f32>() / 4.0;
        assert!(lower < 0.0 && upper > 0.001, "{lower} {upper}");
        assert!(upper - lower >= 0.0029, "{lower} {upper}");
    }
}
