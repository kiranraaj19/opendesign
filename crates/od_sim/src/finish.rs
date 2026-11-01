//! The draped cloth made presentable for a final render, without touching
//! the simulation: seams closed and smoothed across, the surface
//! subdivided, and the cloth given thickness, with its open edges turned
//! back as hems.
//!
//! The simulation keeps each piece's particles apart, sewn by stitches
//! that never quite reach zero length, at a spacing of a couple of
//! centimetres. Shown as is, every seam is a crease or a gap, every edge a
//! jagged polyline, and the cloth a sheet of paper. Here the pieces are
//! welded into one surface along their seams, so Loop subdivision and the
//! normals run straight across them; each face remembers its piece, so it
//! still gets its own fabric.

use std::collections::HashMap;
use std::ops::Range;

use glam::Vec3;
use od_render::mesh::Mesh;

/// How far to finish the cloth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Finish {
    /// Loop subdivision levels; each splits every triangle in four.
    pub subdivisions: u32,
    /// How far an open edge is turned back inside, in metres; 0 leaves
    /// edges raw.
    pub hem_depth: f32,
}

impl Default for Finish {
    /// Two levels and a 15 mm hem: a final render.
    fn default() -> Self {
        Self {
            subdivisions: 2,
            hem_depth: 0.015,
        }
    }
}

/// Sewn particles further apart than this aren't welded: the seam hasn't
/// closed yet (early in a drape), and joining it would tear the surface.
const CLOSE_LIMIT: f32 = 0.02;

/// One piece of cloth: its particles and triangles.
pub struct Piece<'a, K> {
    pub key: K,
    /// Its particles in the shared position list.
    pub range: Range<usize>,
    /// Triangles, indexing the piece's own particles from 0.
    pub faces: &'a [[u32; 3]],
    /// Cloth thickness in metres.
    pub thickness: f32,
}

/// Each piece's finished mesh, keyed like `pieces`. `positions` are every
/// particle; `seams` the pairs of particles sewn together.
pub fn finish<K: Copy + Eq + std::hash::Hash>(
    positions: &[Vec3],
    pieces: &[Piece<'_, K>],
    seams: &[(usize, usize)],
    settings: &Finish,
) -> HashMap<K, Mesh> {
    let welded = weld(positions, seams);
    let signs = orientations(positions, pieces, &welded);

    // One surface over every piece, sewn vertices shared.
    let mut index: HashMap<usize, u32> = HashMap::new();
    let mut surface = Surface::default();
    for (piece, (p, sign)) in pieces.iter().zip(&signs).enumerate() {
        for face in p.faces {
            let [a, b, c] = face.map(|v| {
                let rep = welded.rep[p.range.start + v as usize];
                *index.entry(rep).or_insert_with(|| {
                    surface.positions.push(welded.position[rep]);
                    surface.positions.len() as u32 - 1
                })
            });
            if a == b || b == c || c == a {
                continue; // Collapsed by welding.
            }
            surface
                .faces
                .push(if *sign { [a, b, c] } else { [a, c, b] });
            surface.piece.push(piece);
        }
    }
    for _ in 0..settings.subdivisions {
        surface = surface.subdivided();
    }
    let normals = surface.normals();

    // Boundary edges, as they run in their face (interior on the left
    // seen from the normal), and each boundary vertex's outward direction
    // along the surface.
    let mut edge_faces: HashMap<(u32, u32), u32> = HashMap::new();
    for face in &surface.faces {
        for (a, b) in edges(face) {
            *edge_faces.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let is_open = |a: u32, b: u32| edge_faces.get(&(a.min(b), a.max(b))) == Some(&1);
    let mut outward = vec![Vec3::ZERO; surface.positions.len()];
    for face in &surface.faces {
        for (a, b) in edges(face) {
            if is_open(a, b) {
                let along = surface.positions[b as usize] - surface.positions[a as usize];
                let n = normals[a as usize] + normals[b as usize];
                let out = along.cross(n).normalize_or_zero();
                outward[a as usize] += out;
                outward[b as usize] += out;
            }
        }
    }
    for (o, n) in outward.iter_mut().zip(&normals) {
        *o = (*o - *n * o.dot(*n)).normalize_or_zero();
    }

    let mut meshes = HashMap::new();
    for (i, p) in pieces.iter().enumerate() {
        let mut out = Shell::new(p.thickness);
        for (face, _) in surface
            .faces
            .iter()
            .zip(&surface.piece)
            .filter(|(_, &k)| k == i)
        {
            out.face(face, &surface.positions, &normals);
            for (a, b) in edges(face) {
                if is_open(a, b) {
                    out.edge(
                        a,
                        b,
                        &surface.positions,
                        &normals,
                        &outward,
                        settings.hem_depth,
                    );
                }
            }
        }
        if !out.mesh.indices.is_empty() {
            meshes.insert(p.key, out.mesh);
        }
    }
    meshes
}

fn edges(&[a, b, c]: &[u32; 3]) -> [(u32, u32); 3] {
    [(a, b), (b, c), (c, a)]
}

/// Sewn particles joined into groups: `rep[i]` is the particle standing
/// for particle `i`'s group, at `position[rep]`, the group's mean.
struct Welded {
    rep: Vec<usize>,
    position: Vec<Vec3>,
}

fn weld(positions: &[Vec3], seams: &[(usize, usize)]) -> Welded {
    let mut parent: Vec<usize> = (0..positions.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for &(a, b) in seams {
        if a < positions.len() && b < positions.len() {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..positions.len() {
        let r = root(&mut parent, i);
        if r != i {
            groups.entry(r).or_default().push(i);
        }
    }
    let mut rep: Vec<usize> = (0..positions.len()).collect();
    let mut position = positions.to_vec();
    for (r, mut members) in groups {
        members.push(r);
        let mean = members.iter().map(|&i| positions[i]).sum::<Vec3>() / members.len() as f32;
        if members
            .iter()
            .all(|&i| positions[i].distance(mean) <= CLOSE_LIMIT)
        {
            position[r] = mean;
            for &i in &members {
                rep[i] = r;
            }
        }
    }
    Welded { rep, position }
}

/// Whether each piece's triangles already face out of the garment (true)
/// or need turning over, so the welded surface is oriented one way.
/// Neighbours across a seam should agree; each group of sewn pieces faces
/// away from the garment's vertical axis overall.
fn orientations<K>(positions: &[Vec3], pieces: &[Piece<'_, K>], welded: &Welded) -> Vec<bool> {
    // Each piece's own smooth normals at its particles.
    let mut normal = vec![Vec3::ZERO; positions.len()];
    let mut owner = vec![usize::MAX; positions.len()];
    for (i, p) in pieces.iter().enumerate() {
        for &[a, b, c] in p.faces {
            let [a, b, c] = [a, b, c].map(|v| p.range.start + v as usize);
            let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
            for v in [a, b, c] {
                normal[v] += n;
                owner[v] = i;
            }
        }
    }
    // How much each pair of sewn pieces' normals agree along their seams.
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &r) in welded.rep.iter().enumerate() {
        if owner[i] != usize::MAX {
            groups.entry(r).or_default().push(i);
        }
    }
    let mut agreement: HashMap<(usize, usize), f32> = HashMap::new();
    for members in groups.values() {
        for (k, &a) in members.iter().enumerate() {
            for &b in &members[k + 1..] {
                let (pa, pb) = (owner[a], owner[b]);
                if pa != pb {
                    let key = (pa.min(pb), pa.max(pb));
                    *agreement.entry(key).or_default() += normal[a]
                        .normalize_or_zero()
                        .dot(normal[b].normalize_or_zero());
                }
            }
        }
    }
    let axis = {
        let all: Vec<Vec3> = pieces
            .iter()
            .flat_map(|p| positions[p.range.clone()].iter().copied())
            .collect();
        all.iter().sum::<Vec3>() / all.len().max(1) as f32
    };
    let outwardness = |i: usize| -> f32 {
        let p = &pieces[i];
        p.range
            .clone()
            .map(|v| {
                let mut away = positions[v] - axis;
                away.y = 0.0;
                normal[v].dot(away.normalize_or_zero())
            })
            .sum()
    };

    let mut sign: Vec<Option<bool>> = vec![None; pieces.len()];
    // Largest pieces first: their outward side is the clearest.
    let mut order: Vec<usize> = (0..pieces.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(pieces[i].faces.len()));
    for start in order {
        if sign[start].is_some() {
            continue;
        }
        sign[start] = Some(outwardness(start) >= 0.0);
        let mut queue = vec![start];
        while let Some(i) = queue.pop() {
            let si = sign[i].expect("set before queueing");
            for (&(a, b), &agree) in &agreement {
                let other = match (a == i, b == i) {
                    (true, _) => b,
                    (_, true) => a,
                    _ => continue,
                };
                if sign[other].is_none() {
                    sign[other] = Some(if agree >= 0.0 { si } else { !si });
                    queue.push(other);
                }
            }
        }
    }
    sign.into_iter().map(|s| s.unwrap_or(true)).collect()
}

/// A triangle mesh with each face's piece.
#[derive(Debug, Clone, Default)]
struct Surface {
    positions: Vec<Vec3>,
    faces: Vec<[u32; 3]>,
    piece: Vec<usize>,
}

impl Surface {
    /// One step of Loop subdivision. Edges with one face (open edges) or
    /// more than two (where three pieces meet) are creases, subdivided as
    /// curves of their own, so hems stay where they are.
    fn subdivided(&self) -> Self {
        let mut opposite: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for &[a, b, c] in &self.faces {
            for (u, v, w) in [(a, b, c), (b, c, a), (c, a, b)] {
                opposite.entry((u.min(v), u.max(v))).or_default().push(w);
            }
        }
        let n = self.positions.len();
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); n];
        let mut creases: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (&(a, b), opp) in &opposite {
            neighbours[a as usize].push(b);
            neighbours[b as usize].push(a);
            if opp.len() != 2 {
                creases[a as usize].push(b);
                creases[b as usize].push(a);
            }
        }
        let p = |i: u32| self.positions[i as usize];
        let mut positions: Vec<Vec3> = (0..n)
            .map(|i| {
                let v = self.positions[i];
                match creases[i].len() {
                    0 if !neighbours[i].is_empty() => {
                        let k = neighbours[i].len() as f32;
                        let beta = if neighbours[i].len() == 3 {
                            3.0 / 16.0
                        } else {
                            3.0 / (8.0 * k)
                        };
                        v * (1.0 - k * beta)
                            + neighbours[i].iter().map(|&j| p(j)).sum::<Vec3>() * beta
                    }
                    2 => v * 0.75 + (p(creases[i][0]) + p(creases[i][1])) * 0.125,
                    _ => v, // A corner, or a lone vertex.
                }
            })
            .collect();
        let mut midpoint: HashMap<(u32, u32), u32> = HashMap::new();
        for (&(a, b), opp) in &opposite {
            let m = if opp.len() == 2 {
                (p(a) + p(b)) * 0.375 + (p(opp[0]) + p(opp[1])) * 0.125
            } else {
                (p(a) + p(b)) * 0.5
            };
            midpoint.insert((a, b), positions.len() as u32);
            positions.push(m);
        }
        let mid = |a: u32, b: u32| midpoint[&(a.min(b), a.max(b))];
        let mut faces = Vec::with_capacity(self.faces.len() * 4);
        let mut piece = Vec::with_capacity(self.faces.len() * 4);
        for (&[a, b, c], &k) in self.faces.iter().zip(&self.piece) {
            let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
            faces.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
            piece.extend([k; 4]);
        }
        Self {
            positions,
            faces,
            piece,
        }
    }

    /// Smooth, area-weighted vertex normals.
    fn normals(&self) -> Vec<Vec3> {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for &[a, b, c] in &self.faces {
            let [pa, pb, pc] = [a, b, c].map(|i| self.positions[i as usize]);
            let n = (pb - pa).cross(pc - pa);
            for i in [a, b, c] {
                normals[i as usize] += n;
            }
        }
        normals.into_iter().map(|n| n.normalize_or_zero()).collect()
    }
}

/// One piece's output: the cloth's outer and inner faces, a rim joining
/// them along open edges, and the hem turned back inside.
struct Shell {
    mesh: Mesh,
    half: f32,
    /// Output vertices already made, by (surface vertex, layer).
    made: HashMap<(u32, u8), u32>,
}

/// Which copy of a surface vertex an output vertex is.
const OUTER: u8 = 0;
const INNER: u8 = 1;
const RIM_OUTER: u8 = 2;
const RIM_INNER: u8 = 3;
const HEM_FOLD: u8 = 4;
const HEM_TOP: u8 = 5;

impl Shell {
    fn new(thickness: f32) -> Self {
        Self {
            mesh: Mesh::default(),
            half: thickness / 2.0,
            made: HashMap::new(),
        }
    }

    fn vertex(&mut self, v: u32, layer: u8, position: Vec3, normal: Vec3) -> u32 {
        *self.made.entry((v, layer)).or_insert_with(|| {
            self.mesh.positions.push(position);
            self.mesh.normals.push(normal);
            self.mesh.positions.len() as u32 - 1
        })
    }

    fn face(&mut self, face: &[u32; 3], positions: &[Vec3], normals: &[Vec3]) {
        let outer = face.map(|v| {
            let (p, n) = (positions[v as usize], normals[v as usize]);
            self.vertex(v, OUTER, p + n * self.half, n)
        });
        let inner = face.map(|v| {
            let (p, n) = (positions[v as usize], normals[v as usize]);
            self.vertex(v, INNER, p - n * self.half, -n)
        });
        self.mesh.indices.extend(outer);
        self.mesh.indices.extend([inner[0], inner[2], inner[1]]);
    }

    /// The rim along open edge a→b (interior on its left seen from the
    /// normal), and the hem band inside it.
    fn edge(
        &mut self,
        a: u32,
        b: u32,
        positions: &[Vec3],
        normals: &[Vec3],
        outward: &[Vec3],
        hem_depth: f32,
    ) {
        let at = |v: u32| {
            (
                positions[v as usize],
                normals[v as usize],
                outward[v as usize],
            )
        };
        let (half, hem) = (self.half, hem_depth > 0.0);
        // With a hem the rim is the fold, down to the turned-back layer.
        let inner_offset = if hem { -3.0 * half } else { -half };
        let rim = |s: &mut Self, v: u32| {
            let (p, n, o) = at(v);
            [
                s.vertex(v, RIM_OUTER, p + n * half, o),
                s.vertex(v, RIM_INNER, p + n * inner_offset, o),
            ]
        };
        let [ao, ai] = rim(self, a);
        let [bo, bi] = rim(self, b);
        // Seen from outside the edge, a is on the right.
        self.mesh.indices.extend([ao, ai, bi, ao, bi, bo]);
        if hem {
            let band = |s: &mut Self, v: u32| {
                let (p, n, o) = at(v);
                let base = p + n * inner_offset;
                [
                    s.vertex(v, HEM_FOLD, base, -n),
                    s.vertex(v, HEM_TOP, base - o * hem_depth, -n),
                ]
            };
            let [af, at_] = band(self, a);
            let [bf, bt] = band(self, b);
            // Faces inwards, like the inner shell.
            self.mesh.indices.extend([af, bt, bf, af, at_, bt]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat square of `n`×`n` quads in the XY plane at `z`, `size`
    /// metres across, as a piece's particles and faces.
    fn grid(n: u32, size: f32, origin: Vec3) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let mut positions = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                positions.push(origin + Vec3::new(i as f32, j as f32, 0.0) * (size / n as f32));
            }
        }
        let mut faces = Vec::new();
        let at = |i: u32, j: u32| j * (n + 1) + i;
        for j in 0..n {
            for i in 0..n {
                faces.push([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                faces.push([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
        (positions, faces)
    }

    fn settings(subdivisions: u32, hem_depth: f32) -> Finish {
        Finish {
            subdivisions,
            hem_depth,
        }
    }

    #[test]
    fn subdividing_keeps_a_flat_sheet_flat_with_four_times_the_faces() {
        let (positions, faces) = grid(3, 0.3, Vec3::new(0.0, 0.0, 0.2));
        let pieces = [Piece {
            key: 1,
            range: 0..positions.len(),
            faces: &faces,
            thickness: 0.0,
        }];
        let meshes = finish(&positions, &pieces, &[], &settings(1, 0.0));
        let mesh = &meshes[&1];
        // Outer and inner shells, plus a rim of two triangles per open edge.
        let shell_faces = faces.len() * 4 * 2;
        let rim_faces = 12 * 2 * 2;
        assert_eq!(mesh.triangle_count(), shell_faces + rim_faces);
        for p in &mesh.positions {
            assert!((p.z - 0.2).abs() < 1e-5, "{p:?}");
        }
        // Open edges are creases: the square's corners stay put.
        let b = mesh.bounds().unwrap();
        assert!(b.min.distance(Vec3::new(0.0, 0.0, 0.2)) < 1e-5);
        assert!((b.max.x - 0.3).abs() < 1e-5 && (b.max.y - 0.3).abs() < 1e-5);
    }

    #[test]
    fn thickness_separates_the_faces_and_triangles_face_their_normals() {
        let (positions, faces) = grid(2, 0.2, Vec3::ZERO);
        let pieces = [Piece {
            key: 'a',
            range: 0..positions.len(),
            faces: &faces,
            thickness: 0.002,
        }];
        let mesh = &finish(&positions, &pieces, &[], &settings(0, 0.01))[&'a'];
        let zs: Vec<f32> = mesh.positions.iter().map(|p| p.z).collect();
        let (lo, hi) = zs
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &z| (l.min(z), h.max(z)));
        assert!((hi - 0.001).abs() < 1e-5, "outer face {hi}");
        assert!((lo + 0.003).abs() < 1e-5, "hem layer {lo}");
        for t in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
            let n = (b - a).cross(c - a);
            if n.length() < 1e-12 {
                continue;
            }
            let avg = t.iter().map(|&i| mesh.normals[i as usize]).sum::<Vec3>();
            assert!(n.dot(avg) > 0.0, "a triangle faces against its normals");
        }
        // The hem reaches 1 cm in from every edge, inside the cloth.
        let hem: Vec<&Vec3> = mesh.positions.iter().filter(|p| p.z < -0.0025).collect();
        assert!(hem
            .iter()
            .any(|p| (p.x - 0.01).abs() < 1e-4 && p.y > 0.05 && p.y < 0.15));
    }

    /// Two pieces side by side, sewn along their shared edge with a small
    /// gap, one of them triangulated the other way round: the seam closes
    /// and the normals agree across it.
    #[test]
    fn sewn_pieces_weld_into_one_smooth_surface() {
        let (left, left_faces) = grid(2, 0.2, Vec3::new(-0.2, 0.0, 0.0));
        let (right, mut right_faces) = grid(2, 0.2, Vec3::new(0.003, 0.0, 0.0));
        for f in &mut right_faces {
            f.swap(1, 2);
        }
        let mut positions = left.clone();
        positions.extend(&right);
        let offset = left.len();
        // The left piece's right column (i = 2) to the right's left (i = 0).
        let seams: Vec<(usize, usize)> = (0..3).map(|j| (j * 3 + 2, offset + j * 3)).collect();
        let pieces = [
            Piece {
                key: 0,
                range: 0..offset,
                faces: &left_faces,
                thickness: 0.0,
            },
            Piece {
                key: 1,
                range: offset..positions.len(),
                faces: &right_faces,
                thickness: 0.0,
            },
        ];
        let meshes = finish(&positions, &pieces, &seams, &settings(1, 0.0));
        let outer_normal = |m: &Mesh| m.normals.iter().find(|n| n.z.abs() > 0.5).copied().unwrap();
        let (a, b) = (outer_normal(&meshes[&0]), outer_normal(&meshes[&1]));
        assert!(a.dot(b) > 0.99, "{a:?} {b:?}");
        // The seam closed: both pieces reach the mean of the gap.
        let xs = |m: &Mesh| m.positions.iter().map(|p| p.x).collect::<Vec<_>>();
        let left_max = xs(&meshes[&0]).into_iter().fold(f32::MIN, f32::max);
        let right_min = xs(&meshes[&1]).into_iter().fold(f32::MAX, f32::min);
        assert!((left_max - 0.0015).abs() < 1e-4, "{left_max}");
        assert!((right_min - 0.0015).abs() < 1e-4, "{right_min}");
        // No rim along the seam: only the outside edges are open.
        let rim = meshes[&0].normals.iter().filter(|n| n.x > 0.9).count();
        assert_eq!(rim, 0, "a rim along the welded seam");
    }

    #[test]
    fn far_apart_stitches_are_not_welded() {
        let positions = vec![Vec3::ZERO, Vec3::X * 0.5];
        let welded = weld(&positions, &[(0, 1)]);
        assert_eq!(welded.rep, vec![0, 1]);
        let close = weld(&[Vec3::ZERO, Vec3::X * 0.01], &[(0, 1)]);
        assert_eq!(close.rep, vec![0, 0]);
        assert!((close.position[0].x - 0.005).abs() < 1e-6);
    }
}
