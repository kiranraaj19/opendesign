//! Arrangement points: named spots just off the avatar's surface where a
//! pattern can be placed before draping, facing out from the body. They
//! are found by casting rays at the avatar mesh, so they follow it when it
//! is resized and work for loaded avatars too.

use glam::Vec3;

use crate::mesh::Mesh;
use crate::pattern::Placement;
use crate::pick::{ray_mesh, Ray};

/// How far off the body a point sits, in metres.
const GAP: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrangementPoint {
    pub name: &'static str,
    /// Just off the body, in metres.
    pub position: Vec3,
    /// Away from the body, unit length and level.
    pub outward: Vec3,
}

impl ArrangementPoint {
    /// A pattern placed here: centred on the point, upright, its front
    /// facing out from the body.
    pub fn placement(&self) -> Placement {
        Placement {
            center: self.position,
            right: Vec3::Y.cross(self.outward).normalize(),
            up: Vec3::Y,
        }
    }
}

/// Each spot: name, height as a fraction of the avatar's, sideways offset
/// as a fraction of its height (+ towards its left, +X), and the direction
/// the ray comes from (the side the piece faces).
const SPOTS: [(&str, f32, f32, Vec3); 6] = [
    ("Front torso", 0.66, 0.0, Vec3::Z),
    ("Back torso", 0.66, 0.0, Vec3::NEG_Z),
    ("Left arm", 0.62, 0.0, Vec3::X),
    ("Right arm", 0.62, 0.0, Vec3::NEG_X),
    ("Left leg", 0.28, 0.055, Vec3::Z),
    ("Right leg", 0.28, -0.055, Vec3::Z),
];

/// The arrangement points on `avatar` (a mesh standing on the floor,
/// facing +Z). Spots whose ray misses the body are left out.
pub fn arrangement_points(avatar: &Mesh) -> Vec<ArrangementPoint> {
    let Some(bounds) = avatar.bounds() else {
        return Vec::new();
    };
    let height = bounds.size().y;
    let centre = bounds.center();
    let reach = bounds.size().length() + 1.0;
    SPOTS
        .iter()
        .filter_map(|&(name, up, side, from)| {
            let target = Vec3::new(
                centre.x + side * height,
                bounds.min.y + up * height,
                centre.z,
            );
            // Along the spot's side, but at the target's height and offset.
            let start = target + from * reach;
            let ray = Ray {
                origin: start,
                dir: -from,
            };
            let t = ray_mesh(&ray, avatar)?;
            Some(ArrangementPoint {
                name,
                position: ray.at(t) + from * GAP,
                outward: from,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::{avatar_mesh, AvatarKind};

    #[test]
    fn points_sit_just_off_the_body_and_follow_its_size() {
        let kind = AvatarKind::Female;
        let body = avatar_mesh(kind, kind.default_measurements());
        let points = arrangement_points(&body);
        let names: Vec<_> = points.iter().map(|p| p.name).collect();
        assert_eq!(names, SPOTS.map(|s| s.0));
        let bounds = body.bounds().unwrap();
        let front = points[0];
        assert!(front.position.z > 0.0 && front.position.z < bounds.max.z + GAP + 1e-3);
        assert!(points[1].position.z < 0.0, "back torso behind");
        assert!(points[2].position.x > 0.0 && points[3].position.x < 0.0);
        // A piece placed there faces outward.
        for p in &points {
            assert!(p.placement().normal().dot(p.outward) > 0.99, "{}", p.name);
        }

        let mut taller = kind.default_measurements();
        taller.height += 20.0;
        let tall = arrangement_points(&avatar_mesh(kind, taller));
        assert!(tall[0].position.y > front.position.y + 0.1);
    }
}
