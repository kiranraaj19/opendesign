//! The garment view's scene: the avatar, the patterns laid out around it,
//! the floor, and picking. The app supplies its own pattern keys (`K`), so
//! this stays independent of the document model.

use glam::{Vec2, Vec3};

use crate::camera::OrbitCamera;
use crate::mesh::{Aabb, Mesh};
use crate::pattern::{pattern_mesh, RingLayout};
use crate::pick::{closest_hit, Ray};
use crate::scene::{blob_shadow, floor_grid, AvatarDraw, Color, Frame, Globals, GridColors};

pub struct Theme {
    pub avatar: Color,
    pub pattern: Color,
    pub selected: Color,
    pub outline: Color,
    pub grid: GridColors,
    pub shadow: Color,
}

/// A pattern turned into geometry for the 3D view.
#[derive(Debug, Clone)]
pub struct PlacedPattern<K> {
    pub key: K,
    pub mesh: Mesh,
    pub outline: Vec<Vec3>,
    pub normal: Vec3,
}

/// Lay patterns (outlines in millimetres) out on a ring around the avatar.
/// Outlines that can't be triangulated are skipped.
pub fn place_patterns<K>(
    avatar: Aabb,
    patterns: impl ExactSizeIterator<Item = (K, Vec<Vec2>)>,
) -> Vec<PlacedPattern<K>> {
    let ring = RingLayout::around(avatar);
    let count = patterns.len();
    patterns
        .enumerate()
        .filter_map(|(i, (key, outline))| {
            let placement = ring.placement(i, count);
            let mesh = pattern_mesh(&outline, &placement)?;
            Some(PlacedPattern {
                key,
                mesh,
                outline: placement.place(&outline),
                normal: placement.normal(),
            })
        })
        .collect()
}

/// Everything worth framing: the avatar plus the patterns.
pub fn scene_bounds<K>(avatar: Aabb, patterns: &[PlacedPattern<K>]) -> Aabb {
    patterns
        .iter()
        .filter_map(|p| p.mesh.bounds())
        .fold(avatar, Aabb::union)
}

pub struct FrameInput<'a, K> {
    pub camera: &'a OrbitCamera,
    pub aspect: f32,
    pub encode_srgb: bool,
    pub avatar: Option<AvatarDraw>,
    pub avatar_bounds: Aabb,
    pub patterns: &'a [PlacedPattern<K>],
    pub selected: Option<K>,
    pub theme: &'a Theme,
    /// Per-vertex colors for `patterns`, in the same order (e.g. a strain
    /// heatmap), overriding the theme fill for that pattern. An entry
    /// that doesn't match its pattern's vertex count falls back to the
    /// theme fill, as if absent.
    pub pattern_colors: Option<&'a [Vec<Color>]>,
}

pub fn compose_frame<K: PartialEq>(input: FrameInput<'_, K>) -> Frame {
    let theme = input.theme;
    let mut frame = Frame::new(Globals::new(input.camera, input.aspect, input.encode_srgb));
    frame.avatar = input.avatar;
    for (i, p) in input.patterns.iter().enumerate() {
        let selected = input.selected.as_ref() == Some(&p.key);
        let (fill, line) = if selected {
            (theme.selected, theme.selected)
        } else {
            (theme.pattern, theme.outline)
        };
        let varying = input
            .pattern_colors
            .and_then(|c| c.get(i))
            .filter(|c| c.len() == p.mesh.positions.len());
        match varying {
            Some(colors) => frame.add_surface_varying(&p.mesh, colors),
            None => frame.add_surface(&p.mesh, fill),
        }
        frame.add_outline(&p.outline, p.normal, line);
    }
    frame.lines.extend(floor_grid(2.0, 0.1, 5, &theme.grid));
    let b = input.avatar_bounds;
    let size = b.size();
    frame.shadow = blob_shadow(
        Vec2::new(b.center().x, b.center().z),
        Vec2::new(size.x * 0.55, size.z * 0.75).max(Vec2::splat(0.1)),
        theme.shadow,
    );
    frame
}

/// The pattern under the ray, unless the avatar is in front of it.
pub fn pick<K: Copy>(ray: &Ray, avatar: Option<&Mesh>, patterns: &[PlacedPattern<K>]) -> Option<K> {
    let items = avatar
        .map(|m| (None, m))
        .into_iter()
        .chain(patterns.iter().map(|p| (Some(p.key), &p.mesh)));
    closest_hit(ray, items).and_then(|(key, _)| key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar::{avatar_mesh, AvatarKind};
    use crate::camera::ViewPreset;

    fn rect_mm(w: f32, h: f32) -> Vec<Vec2> {
        vec![
            Vec2::ZERO,
            Vec2::new(w, 0.0),
            Vec2::new(w, h),
            Vec2::new(0.0, h),
        ]
    }

    fn scene() -> (Mesh, Vec<PlacedPattern<u32>>) {
        let avatar = avatar_mesh(AvatarKind::Male, AvatarKind::Male.default_measurements());
        let patterns = (1..=4)
            .map(|k| (k, rect_mm(400.0, 500.0)))
            .collect::<Vec<_>>();
        let placed = place_patterns(avatar.bounds().unwrap(), patterns.into_iter());
        (avatar, placed)
    }

    #[test]
    fn patterns_surround_the_avatar_without_touching_it() {
        let (avatar, placed) = scene();
        assert_eq!(placed.len(), 4);
        let body = avatar.bounds().unwrap();
        for p in &placed {
            let b = p.mesh.bounds().unwrap();
            // The pattern plane sits outside the avatar's footprint.
            let centre = b.center();
            let reach = centre.x.abs().max(centre.z.abs());
            assert!(
                reach > body.max.x.max(body.max.z),
                "pattern {} at {centre}",
                p.key
            );
        }
        let all = scene_bounds(body, &placed);
        assert!(all.size().x > body.size().x);
    }

    #[test]
    fn degenerate_patterns_are_skipped() {
        let body = avatar_mesh(AvatarKind::Kid, AvatarKind::Kid.default_measurements());
        let placed = place_patterns(
            body.bounds().unwrap(),
            vec![(1, rect_mm(100.0, 100.0)), (2, vec![Vec2::ZERO])].into_iter(),
        );
        assert_eq!(placed.iter().map(|p| p.key).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn clicking_through_the_view_picks_the_front_pattern() {
        let (avatar, placed) = scene();
        let mut camera = OrbitCamera::default();
        camera.set_view(ViewPreset::Front);
        camera.frame(scene_bounds(avatar.bounds().unwrap(), &placed), 1.0);
        let at = |world: Vec3| camera.ray(camera.project(world, 1.0).unwrap(), 1.0);

        // Pattern 1 hangs in front of the avatar (and of pattern 3).
        let front = placed[0].mesh.bounds().unwrap().center();
        assert_eq!(pick(&at(front), Some(&avatar), &placed), Some(1));
        // Empty floor far to the side.
        assert_eq!(
            pick(&at(Vec3::new(1.5, 0.0, 0.0)), Some(&avatar), &placed),
            None
        );

        // From the front-left, pattern 2 on the avatar's left is in view.
        let mut camera = OrbitCamera::default();
        camera.frame(scene_bounds(avatar.bounds().unwrap(), &placed), 1.0);
        let side = placed[1].mesh.bounds().unwrap().center();
        let ray = camera.ray(camera.project(side, 1.0).unwrap(), 1.0);
        assert_eq!(pick(&ray, Some(&avatar), &placed), Some(2));
    }

    #[test]
    fn the_avatar_hides_patterns_behind_it() {
        let (avatar, mut placed) = scene();
        placed.truncate(1);
        let front = placed[0].mesh.bounds().unwrap().center();
        let mut camera = OrbitCamera::default();
        camera.set_view(ViewPreset::Back);
        camera.frame(scene_bounds(avatar.bounds().unwrap(), &placed), 1.0);
        let ray = camera.ray(camera.project(front, 1.0).unwrap(), 1.0);
        assert_eq!(pick(&ray, Some(&avatar), &placed), None);
        assert_eq!(pick(&ray, None, &placed), Some(1), "hit without the body");
    }

    #[test]
    fn selection_recolours_one_pattern() {
        let (avatar, placed) = scene();
        let theme = Theme {
            avatar: [0.5; 4],
            pattern: [0.2, 0.3, 0.6, 1.0],
            selected: [1.0, 0.6, 0.1, 1.0],
            outline: [0.6, 0.7, 0.9, 1.0],
            grid: GridColors {
                minor: [0.1; 4],
                major: [0.2; 4],
                axis: [0.3; 4],
            },
            shadow: [0.0, 0.0, 0.0, 0.3],
        };
        let frame = compose_frame(FrameInput {
            camera: &OrbitCamera::default(),
            aspect: 1.0,
            encode_srgb: true,
            avatar: None,
            avatar_bounds: avatar.bounds().unwrap(),
            patterns: &placed,
            selected: Some(2),
            theme: &theme,
            pattern_colors: None,
        });
        let per_pattern = placed[0].mesh.positions.len();
        let selected = frame
            .surfaces
            .iter()
            .filter(|v| v.color == theme.selected)
            .count();
        assert_eq!(selected, per_pattern);
        assert_eq!(frame.surfaces.len(), per_pattern * 4);
        assert!(!frame.shadow.is_empty());
        assert!(frame.lines.iter().any(|v| v.color == theme.grid.axis));
    }

    #[test]
    fn pattern_colors_override_the_theme_fill_per_vertex() {
        let (avatar, placed) = scene();
        let theme = Theme {
            avatar: [0.5; 4],
            pattern: [0.2, 0.3, 0.6, 1.0],
            selected: [1.0, 0.6, 0.1, 1.0],
            outline: [0.6, 0.7, 0.9, 1.0],
            grid: GridColors {
                minor: [0.1; 4],
                major: [0.2; 4],
                axis: [0.3; 4],
            },
            shadow: [0.0, 0.0, 0.0, 0.3],
        };
        let per_pattern = placed[0].mesh.positions.len();
        let heat: Color = [0.9, 0.1, 0.1, 1.0];
        // One pattern gets per-vertex colors, matching the mesh's vertex
        // count exactly; a mismatched entry (the empty second one) falls
        // back to the theme fill, same as no entry at all.
        let colors = vec![vec![heat; per_pattern], Vec::new()];
        let frame = compose_frame(FrameInput {
            camera: &OrbitCamera::default(),
            aspect: 1.0,
            encode_srgb: true,
            avatar: None,
            avatar_bounds: avatar.bounds().unwrap(),
            patterns: &placed[..2],
            selected: None,
            theme: &theme,
            pattern_colors: Some(&colors),
        });
        assert!(frame.surfaces[..per_pattern]
            .iter()
            .all(|v| v.color == heat));
        assert!(
            frame.surfaces[per_pattern..per_pattern * 2]
                .iter()
                .all(|v| v.color == theme.pattern),
            "mismatched color count falls back to the theme fill"
        );
    }
}
