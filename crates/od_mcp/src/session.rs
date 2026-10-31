//! What the server keeps between calls: the document and its undo
//! history, the avatar, the drape in progress and the GPU device used for
//! renders.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use od_core::{Command, Document, History, Joint, PatternId, Pose};
use od_render::arrange::{arrangement_points, ArrangementPoint};
use od_render::avatar::{AvatarKind, Measurements};
use od_render::glam::Vec3;
use od_render::mesh::Mesh;
use od_render::pattern::{Placement, RingLayout};
use od_render::skeleton::{pose_rotations, Rig};
use od_sim::cloth::ClothConfig;
use od_sim::collision::Collider;
use od_sim::drape::Drape;

/// Seconds of simulated time per drape step, and solver substeps per step,
/// as the app uses.
pub const DT: f32 = 1.0 / 60.0;
pub const SUBSTEPS: u32 = 24;
/// How long a fresh drape takes to pull its seams closed: pieces start
/// several centimetres off the body and apart, and snapping them shut at
/// once flings the cloth through the avatar.
pub const SEW_IN_SECONDS: f32 = 1.5;

pub struct Session {
    pub doc: Document,
    pub history: History,
    /// Where the project was opened from or last saved to.
    pub path: Option<PathBuf>,
    pub avatar: AvatarSpec,
    avatar_mesh: Option<(AvatarSpec, Pose, Arc<Mesh>)>,
    pub drape: Option<DrapeState>,
    gpu: Option<Result<Arc<Gpu>, String>>,
    /// Pieces that start a drape wrapped round a limb (sleeves, trouser
    /// legs) rather than flat beside it, so the limb ends up inside.
    pub wraps: HashMap<PatternId, Limb>,
}

/// An arm or leg a piece can be wrapped round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limb {
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl Limb {
    /// The joints the limb runs between, top to bottom.
    fn joints(self) -> (Joint, Joint) {
        match self {
            Limb::LeftArm => (Joint::LeftShoulder, Joint::LeftWrist),
            Limb::RightArm => (Joint::RightShoulder, Joint::RightWrist),
            Limb::LeftLeg => (Joint::LeftHip, Joint::LeftAnkle),
            Limb::RightLeg => (Joint::RightHip, Joint::RightAnkle),
        }
    }

    /// Away from the body, on the limb's side.
    fn side(self) -> Vec3 {
        match self {
            Limb::LeftArm | Limb::LeftLeg => Vec3::X,
            Limb::RightArm | Limb::RightLeg => Vec3::NEG_X,
        }
    }
}

/// Which body and what size; the pose is the document's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarSpec {
    pub kind: AvatarKind,
    pub measurements: Measurements,
}

impl Default for AvatarSpec {
    fn default() -> Self {
        let kind = AvatarKind::Female;
        Self {
            kind,
            measurements: kind.default_measurements(),
        }
    }
}

/// A simulation in progress, and what it was built from, so it can be
/// rebuilt when the document or avatar changes.
pub struct DrapeState {
    pub drape: Drape,
    pub doc: Document,
    pub avatar: Arc<Mesh>,
    /// Simulated seconds so far.
    pub time: f32,
    pub particle_distance: f32,
}

pub struct Gpu {
    pub device: od_render::wgpu::Device,
    pub queue: od_render::wgpu::Queue,
    pub adapter: String,
}

impl Session {
    pub fn new() -> Self {
        Self {
            doc: Document::default(),
            history: History::new(),
            path: None,
            avatar: AvatarSpec::default(),
            avatar_mesh: None,
            drape: None,
            gpu: None,
            wraps: HashMap::new(),
        }
    }

    /// Run one undoable command on the document.
    pub fn execute(&mut self, command: impl Command + 'static) {
        self.history.execute(&mut self.doc, Box::new(command));
    }

    /// Start over with `doc`, its history cleared.
    pub fn replace(&mut self, doc: Document, path: Option<PathBuf>) {
        self.doc = doc;
        self.history = History::new();
        self.path = path;
        self.drape = None;
        self.wraps.clear();
    }

    pub fn pattern_exists(&self, id: PatternId) -> bool {
        self.doc.pattern(id).is_some()
    }

    /// The avatar's mesh in the document's pose, built once per change.
    pub fn avatar_mesh(&mut self) -> Arc<Mesh> {
        let pose = &self.doc.avatar_pose;
        match &self.avatar_mesh {
            Some((spec, p, mesh)) if *spec == self.avatar && p == pose => mesh.clone(),
            _ => {
                let rig = Rig::avatar(self.avatar.kind, self.avatar.measurements);
                let mesh = Arc::new(rig.skin(&pose_rotations(pose, rig.bones())));
                self.avatar_mesh = Some((self.avatar, pose.clone(), mesh.clone()));
                mesh
            }
        }
    }

    /// The named spots around the avatar a piece can be placed at.
    pub fn arrangement_points(&mut self) -> Vec<ArrangementPoint> {
        arrangement_points(&self.avatar_mesh())
    }

    /// Where each shown pattern starts out in 3D, as the app places them:
    /// its arrangement, else its slot on a ring around the avatar.
    pub fn placements(&mut self) -> Vec<(PatternId, Placement)> {
        let bounds = self
            .avatar_mesh()
            .bounds()
            .unwrap_or(od_render::mesh::Aabb {
                min: od_render::glam::Vec3::ZERO,
                max: od_render::glam::Vec3::ONE,
            });
        let ring = RingLayout::around(bounds);
        let count = self.doc.patterns.len();
        self.doc
            .patterns
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.hidden)
            .map(|(i, p)| {
                let placement = match p.arrangement {
                    Some(a) => Placement {
                        center: a.center.into(),
                        right: a.right.into(),
                        up: a.up.into(),
                    },
                    None => ring.placement(i, count),
                };
                (p.id, placement)
            })
            .collect()
    }

    /// The drape, rebuilt from scratch if there is none, the document or
    /// avatar changed since it was built, or `reset` is asked for.
    pub fn drape_for(&mut self, reset: bool, particle_distance: f32) -> &mut DrapeState {
        let avatar = self.avatar_mesh();
        let stale = match &self.drape {
            Some(state) => {
                reset
                    || state.doc != self.doc
                    || !Arc::ptr_eq(&state.avatar, &avatar)
                    || state.particle_distance != particle_distance
            }
            None => true,
        };
        if stale {
            let placements = self.placements();
            let mut drape = Drape::new(
                &self.doc,
                &placements,
                Some(&avatar),
                &ClothConfig {
                    particle_distance,
                    ..Default::default()
                },
            );
            let floor = avatar.bounds().map_or(0.0, |b| b.min.y);
            drape.colliders = vec![
                Collider::mesh((*avatar).clone(), 0.005),
                Collider::ground(floor),
            ];
            self.wrap_limbs(&mut drape, &placements);
            drape.sew_in(SEW_IN_SECONDS);
            self.drape = Some(DrapeState {
                drape,
                doc: self.doc.clone(),
                avatar,
                time: 0.0,
                particle_distance,
            });
        }
        self.drape.as_mut().expect("just made")
    }

    /// Roll each wrapped piece's particles into a tube round its limb, as
    /// posed: the piece's middle on the outside of the limb, its left and
    /// right edges meeting underneath, its top at the shoulder or hip.
    fn wrap_limbs(&self, drape: &mut Drape, placements: &[(PatternId, Placement)]) {
        let rig = Rig::avatar(self.avatar.kind, self.avatar.measurements);
        let joints = rig.joint_positions(&pose_rotations(&self.doc.avatar_pose, rig.bones()));
        for (&id, &limb) in &self.wraps {
            let (Some(range), Some((_, placement))) = (
                drape.range_of(id),
                placements.iter().find(|(p, _)| *p == id),
            ) else {
                continue;
            };
            let (top, bottom) = limb.joints();
            let (top, bottom) = (joints[top.index()], joints[bottom.index()]);
            let down = (bottom - top).normalize_or(Vec3::NEG_Y);
            let out = (limb.side() - down * limb.side().dot(down)).normalize_or(limb.side());
            let right = placement.right - down * placement.right.dot(down);
            let around = (right - out * right.dot(out)).normalize_or(down.cross(out));
            let local: Vec<(f32, f32)> = drape.world.particles[range.clone()]
                .iter()
                .map(|p| {
                    let d = p.position - placement.center;
                    (d.dot(placement.right), d.dot(placement.up))
                })
                .collect();
            let (min_x, max_x, max_y) = local
                .iter()
                .fold((f32::MAX, f32::MIN, f32::MIN), |(lo, hi, up), &(x, y)| {
                    (lo.min(x), hi.max(x), up.max(y))
                });
            let width = max_x - min_x;
            let radius = (width / std::f32::consts::TAU).max(0.06);
            let mid = (min_x + max_x) * 0.5;
            for (p, &(x, y)) in drape.world.particles[range].iter_mut().zip(&local) {
                let angle = (x - mid) / radius;
                let along = top + down * (max_y - y - 0.03);
                p.position = along + (out * angle.cos() + around * angle.sin()) * radius;
                p.velocity = Vec3::ZERO;
            }
        }
    }

    /// The drape, if there is one and it still matches the document and
    /// avatar.
    pub fn current_drape(&mut self) -> Option<&DrapeState> {
        let avatar = self.avatar_mesh();
        self.drape
            .as_ref()
            .filter(|s| s.doc == self.doc && Arc::ptr_eq(&s.avatar, &avatar))
    }

    /// The GPU device for renders, found the first time it is needed; any
    /// adapter will do, a software one included.
    pub fn gpu(&mut self) -> Result<Arc<Gpu>, String> {
        self.gpu
            .get_or_insert_with(|| {
                use od_render::wgpu;
                let instance =
                    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
                let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
                    .map_err(|e| format!("no GPU adapter to render with: {e}"))?;
                let name = format!(
                    "{} ({:?})",
                    adapter.get_info().name,
                    adapter.get_info().backend
                );
                let (device, queue) =
                    pollster::block_on(adapter.request_device(&Default::default()))
                        .map_err(|e| format!("couldn't open the GPU: {e}"))?;
                Ok(Arc::new(Gpu {
                    device,
                    queue,
                    adapter: name,
                }))
            })
            .clone()
    }
}
