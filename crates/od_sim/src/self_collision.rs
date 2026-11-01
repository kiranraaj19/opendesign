//! Keeps cloth from passing through cloth: every free particle is pushed
//! out from any triangle it comes near that isn't its own, to the
//! thickness of the two fabrics, on the side it was on a substep earlier,
//! and friction holds back how far it slides along it. A seam's particles
//! ignore the triangles of the side they're sewn to, so a seam isn't
//! pushed apart by the very panels it joins; tack ends are left out.

use std::collections::HashMap;

use glam::Vec3;

use crate::collision::closest_on_triangle;
use crate::xpbd::{friction_correction, Particle};

/// Particle-against-triangle collision between the cloth's own triangles.
#[derive(Debug, Clone)]
pub struct SelfCollision {
    pub enabled: bool,
    /// How far apart two layers of cloth are kept at least, in metres;
    /// thicker fabrics keep the sum of their particles' radii.
    pub thickness: f32,
    /// Every cloth triangle, by global particle index.
    triangles: Vec<[usize; 3]>,
    /// Particles that neither collide nor are collided with.
    skip: Vec<bool>,
    /// Triangles with no skipped corner: the ones that collide.
    active: Vec<u32>,
    /// For a seam's particles, the particles they're sewn to: a particle
    /// ignores triangles with any of them as a corner.
    partners: HashMap<usize, Vec<usize>>,
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
            partners: HashMap::new(),
        };
        this.update_active();
        this
    }

    /// Change which particles are left out, e.g. after tacks were added.
    pub fn set_skip(&mut self, skip: Vec<bool>) {
        self.skip = skip;
        self.update_active();
    }

    /// Say which particles are sewn together (`pairs`, by global index):
    /// each ignores the triangles around the others in its seam, however
    /// many seams meet there.
    pub fn set_partners(&mut self, pairs: &[(usize, usize)]) {
        // Union-find, so where three panels meet each corner knows both
        // others.
        fn find(root: &mut HashMap<usize, usize>, i: usize) -> usize {
            let mut r = i;
            while let Some(&up) = root.get(&r) {
                if up == r {
                    break;
                }
                r = up;
            }
            root.insert(i, r);
            r
        }
        let mut root: HashMap<usize, usize> = HashMap::new();
        for &(a, b) in pairs {
            root.entry(a).or_insert(a);
            root.entry(b).or_insert(b);
            let (ra, rb) = (find(&mut root, a), find(&mut root, b));
            if ra != rb {
                root.insert(ra, rb);
            }
        }
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        let members: Vec<usize> = root.keys().copied().collect();
        for i in members {
            let r = find(&mut root, i);
            groups.entry(r).or_default().push(i);
        }
        self.partners.clear();
        for group in groups.into_values() {
            for &i in &group {
                let others = group.iter().copied().filter(|&j| j != i).collect();
                self.partners.insert(i, others);
            }
        }
    }

    /// Whether particle `i` collides with triangle `tri`: not its own, nor
    /// one around a particle it's sewn to.
    fn collides_with(&self, i: usize, tri: [usize; 3]) -> bool {
        if tri.contains(&i) {
            return false;
        }
        match self.partners.get(&i) {
            Some(sewn) => !tri.iter().any(|j| sewn.contains(j)),
            None => true,
        }
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
    /// which says which side of a triangle a particle belongs on;
    /// `friction` scales the fabrics' friction (see [`crate::xpbd::World`]).
    pub fn resolve(&self, particles: &mut [Particle], prev: &[Vec3], friction: f32) {
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
        let widest = particles.iter().map(|p| p.radius).fold(0.0_f32, f32::max);
        let margin = self.thickness.max(2.0 * widest) + moved;
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
                if self.collides_with(i, tri) {
                    self.push_out(particles, prev, i, tri, friction);
                }
            }
        }
    }

    /// One particle against one triangle.
    fn push_out(
        &self,
        particles: &mut [Particle],
        prev: &[Vec3],
        i: usize,
        tri: [usize; 3],
        friction: f32,
    ) {
        let [a, b, c] = tri.map(|j| particles[j].position);
        let p = particles[i].position;
        let normal = (b - a).cross(c - a);
        let len = normal.length();
        if len < 1e-12 {
            return;
        }
        let n = normal / len;
        let tri_radius = tri.iter().map(|&j| particles[j].radius).sum::<f32>() / 3.0;
        let thickness = self.thickness.max(particles[i].radius + tri_radius);
        let reach = thickness + p.distance(prev[i]);
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
        let depth = thickness - gap.dot(n) * side;
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

        // Friction on how far the particle slid over the triangle this
        // substep, shared between them as the push was.
        let moved_tri: Vec3 = tri
            .iter()
            .zip(weights)
            .map(|(&j, w)| (particles[j].position - prev[j]) * w)
            .sum();
        let relative = (particles[i].position - prev[i]) - moved_tri;
        let tri_friction = tri.iter().map(|&j| particles[j].friction).sum::<f32>() / 3.0;
        let mu = (particles[i].friction * tri_friction).max(0.0).sqrt() * friction;
        let held = friction_correction(relative, n, depth, mu) / total;
        particles[i].position -= held * wp;
        for (&j, w) in tri.iter().zip(weights) {
            let m = particles[j].inv_mass;
            particles[j].position += held * (w * m);
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
        collision.resolve(&mut particles, &prev, 1.0);
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
        collision.resolve(&mut particles, &prev, 1.0);
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
        collision.resolve(&mut particles, &prev, 1.0);
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
            collision.resolve(&mut particles, &prev, 1.0);
        }
        let lower = particles[..4].iter().map(|p| p.position.y).sum::<f32>() / 4.0;
        let upper = particles[4..].iter().map(|p| p.position.y).sum::<f32>() / 4.0;
        assert!(lower < 0.0 && upper > 0.001, "{lower} {upper}");
        assert!(upper - lower >= 0.0029, "{lower} {upper}");
    }

    #[test]
    fn thicker_fabric_keeps_layers_further_apart() {
        let rest_height = |radius: f32| {
            let mut particles = Vec::new();
            let tris = square(&mut particles, 0.0, 0.0);
            particles.push(Particle::new(Vec3::new(0.4, 0.0005, 0.6), 1.0));
            for p in &mut particles {
                p.radius = radius;
            }
            let prev: Vec<Vec3> = particles
                .iter()
                .map(|p| p.position + Vec3::Y * 0.01)
                .collect();
            let collision = SelfCollision::new(tris, vec![false; 5], 0.0015);
            collision.resolve(&mut particles, &prev, 1.0);
            particles[4].position.y
        };
        // The two layers keep their radii apart, but never less than the
        // floor.
        assert!((rest_height(0.002) - 0.004).abs() < 1e-6);
        assert!((rest_height(0.0005) - 0.0015).abs() < 1e-6);
    }

    #[test]
    fn friction_between_layers_holds_back_sliding() {
        let slid = |friction: f32| {
            let mut particles = Vec::new();
            let tris = square(&mut particles, 0.0, 0.0);
            particles.push(Particle::new(Vec3::new(0.5, 0.002, 0.5), 1.0));
            for p in &mut particles {
                p.friction = friction;
            }
            // It moved 1 mm sideways and 1 mm down into the cloth.
            let mut prev: Vec<Vec3> = particles.iter().map(|p| p.position).collect();
            prev[4] = Vec3::new(0.499, 0.003, 0.5);
            let collision = SelfCollision::new(tris, vec![false; 5], 0.003);
            collision.resolve(&mut particles, &prev, 1.0);
            particles[4].position.x - 0.499
        };
        assert!((slid(0.0) - 0.001).abs() < 1e-6);
        assert!(slid(0.8).abs() < 1e-6, "{}", slid(0.8));
    }

    #[test]
    fn a_seam_particle_ignores_the_side_its_sewn_to_but_not_other_cloth() {
        // Panel A at y = 0 and panel B just above, sewn at particle 4 to
        // corner 0 of A; another layer C lies just under particle 4.
        let mut particles = Vec::new();
        let mut tris = square(&mut particles, 0.0, 0.0);
        particles.push(Particle::new(Vec3::new(0.0, 0.001, 0.0), 1.0));
        let seam = particles.len() - 1;
        let c = square(&mut particles, -0.5, 0.0);
        tris.extend(c);
        let prev: Vec<Vec3> = particles
            .iter()
            .map(|p| p.position + Vec3::Y * 0.01)
            .collect();
        let mut collision = SelfCollision::new(tris.clone(), vec![false; particles.len()], 0.003);
        collision.set_partners(&[(seam, 0)]);
        let before = particles[seam].position;
        collision.resolve(&mut particles, &prev, 1.0);
        assert_eq!(particles[seam].position, before, "pushed off its own seam");

        // Moved over the far side of panel A, away from the corner it's
        // sewn to, it still collides with A's other triangle... but every
        // triangle of the square has corner 0, so use C instead: dropped
        // onto C, it lands on top.
        particles[seam].position = Vec3::new(0.5, -0.499, 0.4);
        let prev: Vec<Vec3> = particles
            .iter()
            .map(|p| p.position + Vec3::Y * 0.01)
            .collect();
        collision.resolve(&mut particles, &prev, 1.0);
        assert!(
            (particles[seam].position.y + 0.497).abs() < 1e-6,
            "{}",
            particles[seam].position
        );
    }

    #[test]
    fn partners_join_where_seams_meet() {
        let mut collision = SelfCollision::new(Vec::new(), Vec::new(), 0.003);
        collision.set_partners(&[(1, 2), (2, 3), (7, 8)]);
        assert!(!collision.collides_with(1, [3, 10, 11]));
        assert!(!collision.collides_with(3, [1, 10, 11]));
        assert!(collision.collides_with(1, [7, 10, 11]));
        assert!(collision.collides_with(5, [1, 2, 3]));
    }
}
