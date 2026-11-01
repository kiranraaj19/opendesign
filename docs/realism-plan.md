# Lifelike garments and mannequins: phase plan

Comes from reading the CLO3D install (`C:\Program Files\CLO Standalone
OnlineAuth`): its avatar data (`Preset/Avatar`, 937 MB), solver settings
(`Preset/Simulation_Property/*.smp`), block library (`Preset/Modular`),
menu actions (`Style/Tooltip_Info.xml`) and the property names in its
executable. Nothing was copied: CLO's scans, PCA weights, blocks, arrangement
files and fabrics stay out of this repository. These phases come before
R4–R6 in `docs/render-plan.md`. Lighting can only polish a garment that
already drapes like one.

## Why the dress looked home-made

The MCP dress after R3 shows the gap. It looked the same after 3 s and
8 s of draping, so it had settled; the problem is what it settled into.

- **Pattern.** `od_mcp/src/garment.rs` makes the dress from the T-shirt
  body made longer: two rectangles, chest plus 12 cm wide, with square
  armholes, no darts, no waist and no skirt.
- **Solver.** It has no friction (`Fabric::friction` is never read), so the
  bodice slides off the shoulders.
  - Bending uses distance springs across each pair of triangles, with no
    dihedral angle and no buckling, so folds are soft and rubbery.
  - Collision only pushes cloth points out of the body's surface.
  - Self-collision only checks points against triangles, and leaves seam
    particles out.
- **Mannequin.** It is assembled from simple shapes, driven by 4
  measurements, and rigid.
- **Placement.** Pieces start as flat panels in front of and behind the
  body, not wrapped around it.

## What CLO has, and where it lands here

| Area | CLO (observed) | OpenDesign today | Phase |
|---|---|---|---|
| Friction | Fabric `Friction`, body and cloth | body and cloth (G1 done) | G1 |
| Collision thickness | separate from render thickness; avatar `Skin Offset` | per fabric, document skin offset (G1 done) | G1 |
| Bending | warp/weft/bias bending, `Buckling Ratio`/`Buckling Stiffness` (bias too) | distance springs | G2 |
| Stretch | warp/weft, nonlinear curves, `Stretch Test` input | linear XPBD | G2 |
| Damping, density, shrinkage | internal and air damping, density, `Shrinkage` warp/weft | gravity only | G2 |
| Fabric measurement | `Stretch Test`, `Bending Test` input → parameters | firmness sliders | G2 |
| Self-collision | point-triangle and edge-edge, proximity, intersection resolution | point-triangle only; seams collide except with their own seam (G1) | G3 |
| Layers | per-piece `Layer`, sewing layers, layer clone over/under | none | G3 |
| Solver | implicit CG solver, substeps, Normal/Complete/Nonlinear presets | single-threaded XPBD | G3 |
| Blocks | Dress (One Piece, Waisted), T-shirt (Basic, Raglan, Ribbing), Shirt, Coat (Basic, Raglan, Dolman), Jacket, Pants, Skirt | a T-shirt template, 1 bodice starter | G4 |
| Sewing | easing, gathering, fold angle and strength per seam, elastic | 1:1 by fraction | G4 |
| Placement | ~60 named points per body plus "bounding volumes" pieces wrap onto; learned auto-arrangement | flat placement around the body | G5 |
| Body shape | scanned base mesh plus statistical shape model, 6/~10/~15/20-measurement levels, breast/hip/crotch/hand/head shapes, height and weight to size | 4 measurements, simple shapes | G6 |
| Rig | 74 joints, skin weights, IK mappings (Mixamo, Daz, SMPL-X…) | simple skeleton, BVH | G6 |
| Soft body | tetrahedral body, per-region stiffness, skin weights | rigid mesh | G7 |
| Fitting tools | Strengthen, Solidify, Freeze, Steam, Press, Line Tack, Pressure, Fit Map | tacks, pins, strain map | G8 |
| Parametric and modular | modular blocks with snapping, parametric patterns from body landmarks | none | G9 |

## How every phase works

- **Branch and commits:** branch from `origin/master` as `claude/gN-...`,
  one commit per step, all four checks clean before each commit, and merge
  only when asked.
- **Data:** any new document data bumps `project::VERSION` (currently 16)
  and adds a migration and a load test. Any new fabric data bumps the fabric
  file version (currently 3).
- **Verification:** render the MCP dress, T-shirt and skirt front, back and
  three-quarter, before and after, plus a timing line.
  - Where possible, add a numeric test: shoulder seam within N mm of the
    shoulder after draping, no cloth inside the body, fold count, and so
    on.
  - For a side-by-side, the user can drape the matching CLO block in CLO.
    Compare the pictures only; never import CLO files.
- **Performance:** a garment of about 5k particles must stay interactive in
  release. Record the drape time for every phase.

---

## G1: Contact that holds a garment up (small; quickest visible win)

1. **Friction.**
   - **Body:** static and kinetic friction in body collision. After the
     position correction, take the tangential part of the particle's
     movement in the substep. Cancel it if it's below
     μ<sub>s</sub>·depth, otherwise scale it down by μ<sub>k</sub>.
   - **Fabric:** use `Fabric::friction` for cloth on body; the body's own
     friction (skin, default 0.5) is combined by geometric mean.
   - **Cloth on cloth:** the same treatment in self-collision.
2. **Collision thickness.** `FabricLook::thickness_mm` stays for rendering.
   Add a physical `collision_mm` to `Fabric`, default 2.5 mm; self-collision
   uses the sum for the two pieces.
3. **Skin offset.** Each avatar gets a distance cloth keeps from it (CLO
   default about 3 mm), replacing the fixed body thickness.
4. **Seam particles.** Let seam particles back into self-collision, ignoring
   only the pairs their own seams join.

Done when the dress and the T-shirt stay on the shoulders, and a skirt
stays on the hips without being held up by the waist seam.

### G1 status: done on `claude/g1-contact`

- **Friction.** `Particle` carries its fabric's `friction` and a collision
  `radius`.
  - **Body:** `Collider::contact` returns the push-out and the normal.
    `xpbd::friction_correction` then removes the tangential movement: all of
    it below μ·1.25·depth (static), otherwise μ·depth (kinetic).
  - **Combining:** μ is the geometric mean of the fabric's and the
    surface's friction. Skin and ground are 0.5 (`SKIN_FRICTION`).
  - **Cloth on cloth:** the same rule in self-collision, on the movement
    relative to the triangle, shared by inverse mass like the push.
  - **Sew-in:** `World::friction` scales all of it and is 0 while seams
    are sewn in. Otherwise the pieces stick where they were placed and
    bunch up as the seams pull.
- **Collision thickness.** `Fabric::collision_mm` (cotton 2.5, silk 2.0,
  denim 3.5, satin 2.0, velvet 4.0, jersey 2.5, chiffon 1.5; 3.0 for older
  files) sets each particle's radius.
  - Two layers keep r<sub>i</sub> + mean r<sub>triangle</sub> apart, but
    never less than 1.5 mm.
  - It is a "Collision thickness" field in the fabric editor and
    `collision_mm` in the MCP `fabric` op, which also takes `friction`.
- **Skin offset.** `Document::skin_offset_mm` (default 3, 0.5–20) has an
  undoable `SetSkinOffset`. It is in the avatar size panel and the MCP
  `set_avatar` tool, and a running drape picks it up.
- **Seams.** Seam and zipper particles collide again.
  `SelfCollision::set_partners` groups stitched particles transitively, and
  a particle skips only triangles that touch its group. Tack ends and
  anchors are still left out.
- **Formats.** Project v16, fabric files v3, with load tests.
- **Result** (MCP, 3 s drape, studio renders):
  - The dress and T-shirt stay on the shoulders and look a little cleaner
    at the sides.
  - The skirt now stays up on the hips. Before, it slid down into a V at
    the back waist.
  - Mean stretch is unchanged (dress 6.9% vs 7.5%, T-shirt 10.9% vs 11.4%,
    skirt 1.4% vs 1.8%). The high peaks are at the armholes and predate G1.
  - Drape times are about the same.
- **Left for later phases.**
  - Friction holds whatever shape sewing-in leaves. The skirt's back-left
    side seam closes with a ruffle, and the skirt then hangs narrower than
    its A-line. Without friction, that only went away because the skirt
    slid down the hips.
  - Better placement before sewing (G5) and real pattern blocks (G4) are
    the fix, not less friction.

## G2: Real fabric mechanics (large)

1. **Dihedral bending.** Replace distance bending with an XPBD dihedral-
   angle (or isometric quadratic bending) constraint per interior edge.
   Stiffness depends on the edge's angle to the grain, mixing warp, weft
   and bias as `stretch_compliance_at` does today.
2. **Buckling.** Each bend gets a stiffness ratio below 1 past a buckling
   curvature (CLO's `Buckling Ratio`/`Buckling Stiffness`, warp/weft/bias).
   Cloth resists bending at first, then gives into a crisp fold. This is
   the main source of realistic wrinkles.
3. **Nonlinear stretch.** Each direction gets a piecewise-linear
   stress-strain curve (`Vec<(strain, stiffness)>`); compliance is looked
   up from the current strain. A preset of a single segment behaves as
   today.
4. **Damping and density.**
   - Internal damping: XPBD damping on the stretch and bend constraints.
   - Air damping: drag that scales with velocity squared along the normal.
   - Density: becomes mass per area as now, shown in g/m².
5. **Shrinkage.** Warp and weft percentages scale rest lengths. This
   enables ruched and smocked effects.
6. **Fabric test kit.** Stretch-test and bending-test inputs, using
   published lab test methods: load against elongation, and the bending
   length from a cantilever test. These are fitted to the parameters
   above, with presets refitted from published fabric data.
7. **Format.** Fabric gets `bending: [warp, weft, bias]`, buckling
   ratio/stiffness, stretch curves, damping and shrinkage. Project v16,
   fabric files v3, with migrations from today's per-direction
   compliances.

Done when the same dress in cotton, denim and silk shows visibly different
fold patterns (fold count and width measured along a horizontal cut), and
silk's folds are narrower than denim's.

## G3: Robust collision, layers and a faster solver (large)

1. **Edge-edge self-collision** plus point-triangle proximity, with the
   combined thickness from G1, using a spatial hash.
2. **Intersection resolution.** Find cloth-cloth intersection contours
   (global intersection analysis) and push them apart over a few steps.
   CLO's `bIntersectionResolution` is the same idea.
3. **Layers.**
   - Each pattern gets `layer: i8`. A contact between different layers
     pushes the higher layer outward.
   - Each seam gets a sewing layer.
   - Add "Layer clone over/under".
4. **Two-way body contact** as a hook for G7: body collision returns the
   contact impulses.
5. **Speed.**
   - Run constraint colouring and parallel projection with rayon, so
     constraints that don't share particles run in parallel.
   - Collision broad phase on worker threads.
   - Target 3× on today's 5k-particle drape.
   - Look at a GPU compute path behind a feature flag; don't make it
     required.
6. **Quality presets:** Normal (current substeps) and Complete (more
   substeps and iterations, edge-edge on), matching CLO's two common
   presets.

Done when a jacket over a shirt (two layers) drapes without intersecting,
and the dress drape is at least 2× faster than before G3.

## G4: Drafted pattern blocks and proper sewing (large)

1. **Drafting module.** `od_core::draft` drafts blocks from body
   measurements, using published drafting methods (Aldrich- or
   Armstrong-style rules written as code), not CLO's blocks.
   - **Bodice:** front and back, with bust and waist darts, shoulder slope,
     curved neck and armhole.
   - **Sleeve:** set-in, with a cap height from the armhole depth and cap
     ease.
   - **Skirt:** straight, A-line, circle and gathered.
   - **Trousers:** front and back, with crotch curves.
   - **Variants:** raglan and dolman sleeves, a shirt collar with stand, a
     cuff, rib bands.
2. **Garments assembled from those blocks:**
   - **Dress:** one-piece (princess seams) and waisted (bodice plus skirt).
   - **T-shirt:** basic, raglan, ribbing.
   - **Shirt.**
   - **Jacket and coat:** basic, raglan, dolman.
   - **Trousers and skirt.**
   - The MCP `create_garment` and the app's block library both use them,
     replacing `garment.rs`'s template.
3. **Grading.** Blocks are drafted at every size in the size chart, so
   grading comes from the drafting rules, not hand-written deltas.
4. **Easing and gathering.** A seam whose sides differ in length spreads
   the difference evenly (`SewingLine.ease`). A gathered seam adds
   short-rest-length stitches along the longer side. This fixes the "solver
   can't ease a sleeve cap" note in `garment.rs`.
5. **Fold angle and strength per seam** (CLO's Seam Fold Angle/Strength),
   so seam allowances turn the right way.
6. **Elastic** on an edge or internal line: a target length ratio and
   stiffness.

Done when the waisted dress on the default body hangs from the shoulders,
shows a waist, and its side seams run straight down the side of the body.

## G5: Arrangement around the body (medium)

1. **Named points.** About 60 named arrangement points generated from the
   body's landmarks: body front and back centre and sides at 5 heights,
   arm front, back, inside and outside at 3 heights, neck, waist, skirt
   front and back.
2. **Bounding volumes.** Generated per body part (torso, each arm, each
   leg, a skirt cone): elliptic cylinders fitted to cross-sections at each
   point's height.
3. **Wrapping.** A piece placed on a point is bent onto its volume, keeping
   its lengths, and offset by the skin offset plus a gap. Garments then
   start as a tube around the body instead of two flat sheets.
4. **Automatic arrangement.** Pieces carry a `role` (front, back, sleeve
   L/R, collar, cuff, skirt front and back, waistband, pocket…), set by the
   blocks from G4 or by a rule-based guess from shape and name. "Arrange
   all" places them. There is no learned model; a rule table covers the
   block library.
5. **Format:** `Pattern.role` and an arrangement with a point name plus a
   wrap flag, in project v17.

Done when every G4 garment is placed in one click, starts wrapped and needs
under 1 s of sewing-in to close.

## G6: A body model with real proportions (extra large; the biggest asset)

1. **Data source.** Use an openly licensed parametric body: the MakeHuman
   1.1+ base mesh and targets are CC0. Before importing anything, check the
   exact licence file in the version we take and record it in `NOTICE`.
   - Don't use SMPL/SMPL-X: their licence is non-commercial.
   - Don't use CAESAR: it is commercial.
   - Ship the targets as compressed delta files, generated once by a
     script in `scripts/`.
2. **Shape space.** Base mesh plus linear blend of targets: gender, age,
   weight, muscle, height, proportions, plus regional targets for bust
   size and shape, hips, waist, shoulder slope, neck, arms and legs.
3. **Fit to measurements.** Solve for target weights matching measurements
   taken on the mesh, by least squares with regularisation. This takes:
   - CLO's measurement levels: simple (6: height, bust, waist, low hip,
     arm, crotch height), basic, intermediate and advanced (20: adds under
     bust, neck base, high hip, thigh, knee, calf, ankle, bicep, wrist,
     shoulder drop, across shoulder, CF/CB neck to waist, CB neck to wrist,
     total rise);
   - a height-and-weight shortcut through a size table.
   - Measurements on the mesh reuse `od_render::measure` (girth loops,
     surface paths) at landmark vertices defined on the base mesh.
4. **Shape sliders.** Bust shape (high, low, wide, narrow, teardrop,
   round), hip dips and volume, crotch, belly. Each is a target with a
   slider in the avatar editor.
5. **Rig.** Map the existing skeleton to the base mesh's joints, transfer
   or compute skin weights, and keep BVH poses working. A Mixamo-style IK
   mapping table imports other rigs' motion.
6. **Variants:** female, male, kid. Hands and feet come with the mesh, so
   no more "mitten" feet.
7. **Document:** the avatar is saved as a measurement set plus slider
   values (not a mesh), in project v18; old projects map their 4
   measurements onto the new solver.
8. **Skin material** (feeds R5): procedurally generated skin colour,
   normal and roughness maps, never CLO's textures.

Done when the 20 measurements of a published size chart come back within
1 cm from the solved body, and the body reads as a person at the default
render framing.

## G7: Soft body (large)

1. **Tetrahedral body.** Build a volume mesh between the skin and a
   simplified inner skeleton surface with a constrained tetrahedraliser,
   or use the `tetgen` approach with a permissive implementation. Do this
   once per body shape, cached.
2. **Simulation.** XPBD volume and Neo-Hookean (or ARAP) constraints on
   the tetrahedra. Inner vertices follow the skeleton by linear blend
   skinning; per-region stiffness comes from a painted map: soft belly,
   bust and thighs, firm shins and head.
3. **Two-way contact.** Cloth contact impulses from G3 push skin vertices.
   Tight garments, waistbands and bra straps press in.
4. **Mode:** off by default and on for "Complete" quality, since it costs
   time.

Done when a tight waistband visibly presses into the waist and the
cloth-body gap stays at the skin offset.

## G8: Fitting and finishing tools (medium)

1. **Strengthen:** a temporary stiffness multiplier on selected pieces while
   draping.
2. **Solidify:** freeze the shape as rest shape, for stiff collars and
   cuffs.
3. **Freeze:** selected pieces stop moving but still collide.
4. **Steam/Press:** set rest lengths and angles to the current shape
   locally under a brush, to flatten wrinkles and set creases.
5. **Line tack:** tack along a whole line, to the body or to cloth.
6. **Pressure:** a per-piece inflation force along the normal, for puffer
   jackets and pillows.
7. **Fit map:** colour by stress, strain and contact pressure; there's a
   strain map today.
8. **Garment fit report:** ease at bust, waist and hip, measured on the
   draped garment against the body, in Properties and the MCP.

## G9: Parametric and modular patterns (medium)

1. **Modular.** Blocks become modules (bodice, sleeve, collar, cuff,
   skirt, waistband) with named seam interfaces. Swapping a module, for
   example a raglan sleeve for a set-in one, re-sews automatically.
2. **Parametric.** Pattern points can be tied to body measurements, using
   G4's drafting expressions kept in the document. Changing the body or
   the size redrafts the pattern; manual edits become offsets on top of
   the drafted point.
3. **MCP:** `create_garment` takes modules and style parameters (neckline,
   sleeve type, skirt type, length, ease) instead of one template.

---

## Order and effort

| Phase | Depends on | Size | Visible result |
|---|---|---|---|
| G1 contact | none | S | garments stay on |
| G4 blocks and sewing | G1 | L | it's a dress, not a sack |
| G5 arrangement | G4 (roles) | M | clean, quick drapes |
| G2 fabric mechanics | G1 | L | real folds and wrinkles |
| G3 collision, layers, speed | G1 | L | layered garments, faster |
| G6 body model | none (G5's points regenerate per body) | XL | a person, not a doll |
| G7 soft body | G3, G6 | L | cloth presses into flesh |
| G8 fitting tools | G2, G3 | M | CLO's finishing workflow |
| G9 parametric and modular | G4, G6 | M | restyle and refit in one click |

**Suggested order:** G1 → G4 → G5 → G2 → G3 → G6 → G7 → G8 → G9, then R4–R6.

- G1 and G4 fix most of the "a kid sewed it" look on today's body.
- G2 and G3 make the cloth behave like fabric.
- G6 and G7 are the largest pieces, and they matter most once garments
  are already good.
- G6 can run in parallel with G2 and G3 (different crates: `od_render`
  avatar against `od_sim`). G4 and G5 touch `od_core` and `od_mcp`.

**Out of scope (not core to fit realism):**
- 3D body scanning from photos, and CLO's "Create 3D Body Model".
- AI fabric creation (CLO's FabricCreator).
- Camera-based fabric scanning.
- Omniverse/USD live links.
