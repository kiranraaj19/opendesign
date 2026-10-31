# Industry-grade renders: phase plan

Comes from comparing OpenDesign's renders (site/img) with CLO3D's renderer.
CLO ships V-Ray with a CUDA denoiser, HDRI light presets, PBR fabric maps
(normal, roughness, displacement, AO, sheen/velvet, translucency, AxF
measured materials), OpenSubdiv display subdivision, real fabric thickness
and 3D seam puckering and topstitches.

What OpenDesign has today (`od_render/src/scene.wgsl`, `od_mcp/src/render.rs`):
- **Lighting:** Lambert key and fill lights, a hemisphere ambient term and a
  fixed Blinn-Phong highlight. In "quality" mode it adds a rim light and a
  view-angle "AO".
- **Shadow:** a blob shadow on the floor.
- **Fabric:** a colour-only checker or twill texture.
- **Mesh:** 25 mm particles with normals smoothed per piece, so seams crease.
- **Thickness:** the back face sits at the same position as the front, so
  the cloth has none.
- **Output:** 8-bit sRGB with a hard clamp, so whites blow out.

Phases are numbered R1 to R6 so they don't clash with the roadmap or the CLO
gap phases. Each phase works like the others:
- Branch from `origin/master` as `claude/renderN-...`.
- Make one commit per step.
- Run the four checks clean before each commit.
- Fast-forward `master` only when asked.

**How to verify:** render the site garments (sleeveless top, skirt) through
`opendesign-mcp` before and after each phase, front and three-quarter, on
the black background. The PNGs go in the PR. WSL's lavapipe adapter renders
offscreen, so this works without a display. Add a pixel-statistics test
where a property can be measured, for example "white cotton keeps fold
contrast" or "a seam has no normal discontinuity".

Licensing: never copy CLO assets (HDRIs, `.vlp`, `.zfab`, `.sbsar`). Use CC0
HDRIs (Poly Haven) and generated textures only.

---

## R1: Tone, light and backdrop (biggest look change per line of code)

Goal: the same geometry stops looking like a CAD viewport.

1. **HDR render target.** The still renderer draws into `Rgba16Float`. A
   final full-screen pass applies exposure and AgX (or ACES fitted) tone
   mapping, then the sRGB encode. This replaces the `encode()` clamp in
   `scene.wgsl`. The live viewport can keep its direct path behind a flag.
2. **HDRI image-based lighting.** Load an `.hdr` file (the `image` crate's
   Radiance decoder) into an equirect texture, then:
   - Convert it to a cubemap.
   - Precompute a diffuse irradiance map (spherical harmonics or a small
     cube) and a GGX-prefiltered specular mip chain.
   - Add a split-sum BRDF lookup table, generated once and cached.

   Do the precomputation in compute shaders in `od_render`, cached per HDRI.
3. **Light presets.** "Studio soft" (the default), "Studio contrast" and
   "Daylight": a CC0 HDRI plus an optional key-light direction and
   intensity, stored as a small struct (not CLO's `.vlp`). Ship two or
   three HDRIs, downsized to 1–2k, under `assets/hdri/`.
4. **Final-render backdrop.** A seamless curved cyclorama, or a plain floor
   with the grid hidden, plus a contact shadow. The grid stays in the editor
   viewport only.
5. **MCP and app.** `render` gains `lighting` (preset name), `exposure` and
   `backdrop` parameters. The Render dialog in `od_app` gets the same three.

Done when: the white top shows readable fold shading with no clipped white,
and the background no longer reads as a CAD grid.

**Status: built on `claude/render1-light`.** Where it differs from the steps
above:
- **Environment format and BRDF.** IBL uses an equirect mip chain, not a
  cubemap, prefiltered on the CPU at load (`od_render::light`). It uses
  Karis' analytic environment-BRDF fit instead of a lookup texture.
  Diffuse light is 9 spherical-harmonic coefficients in the uniform block.
- **Presets.** The presets are generated in code: soft studio, contrast
  studio and daylight softbox environments. No HDRI files ship. A user's
  own `.hdr` loads through MCP `hdri`.
- **Brightness.** Every lighting setup is normalised so the brightest
  direction lights white to 0.95. Exposure then means the same thing for
  presets and any HDR file.
- **Tone mapping.** Khronos PBR Neutral is the default, with ACES as an
  option. The post pass also composites the background, so a transparent
  PNG works.
- **App.** Instead of a render dialog, the app has a File → Export →
  "Render look" submenu (lighting, backdrop, exposure) for turntable and
  drape exports. It defaults to the soft studio and the studio sweep.
- **MCP.** `render` gains `lighting`, `hdri`, `exposure` (stops),
  `backdrop` (`grid`/`studio`/`none`), `tone_map`, and
  `background: transparent`. The backdrop stays `grid` by default.
- **Live viewport.** It is unchanged: there `ibl.x = 0` and a black
  environment texture is bound.

## R2: Cloth surface geometry (fixes jagged edges and seam creases)

Goal: the garment reads as one continuous, finished piece of fabric.

1. **Weld normals across seams.** When building the draped mesh, map paired
   seam particles to shared normal groups using the `SewingLine` pairs that
   `od_sim::drape` already has, so normals are averaged across pieces. This
   removes the hard line at the shoulder and side seams.
2. **Close seams for display.** For the rendered mesh only, snap each sewn
   particle pair to its midpoint, so the sim's small separation doesn't show
   as a gap.
3. **Display subdivision.** Apply one or two levels of Loop subdivision on
   the draped triangle mesh (in `od_sim/src/mesh.rs` or a new
   `od_render::subdiv`):
   - Keep boundary edges as creases with the boundary rule, so hems stay
     put.
   - Recompute normals after subdividing.
   - Settings: viewport 0 or 1 levels, final 2.
4. **Default particle distance.** Measure solver time at 25 / 20 / 15 mm on
   the dress template. The solver is single-threaded, so pick the finest
   spacing that drapes in a few seconds in release. Subdivision covers the
   rest.
5. **Fabric thickness.** Offset the front and back shells by ±thickness/2
   along the vertex normal, and stitch a rim strip along every outline
   boundary so edges show a real cut thickness. Take the thickness from the
   fabric (new field, see R3; default from fabric weight).
6. **Edge finishing.** Pieces with a hem or facing get a display-only
   turned-back band 10–20 mm deep along the chosen outline edges, rendered
   as a second offset layer. A per-edge `finish: Raw | Hem(mm) | Bound`
   lives in the document (the format change goes in with R3's).

Done when: the armholes and hem are smooth curves with visible thickness,
and no seam shows as a crease or gap at three-quarter view.

## R3: PBR fabric materials

Goal: cotton, satin, denim and velvet look different under the same light.

1. **Shading model.** Replace Lambert plus Blinn-Phong in `fs_lit` with:
   - Lambert diffuse plus a GGX specular term (roughness).
   - A Charlie sheen lobe with the Estevez-Kulla approximation (sheen
     colour, sheen roughness), the same model as glTF
     `KHR_materials_sheen`.
   - Energy-conserving layering.

   IBL from R1 feeds all of these terms.
2. **Fabric fields.** Add `roughness`, `sheen_color`, `sheen_roughness`,
   `thickness_mm`, `opacity` and `translucency` to `od_core::Fabric`. This
   bumps the project format to v13 with a migration that derives values from
   the existing presets. Fabric files get the same fields.
3. **Weave normal maps.** `plain_weave`/`twill_weave` also produce a tiling
   normal map and a roughness map generated from yarn height.
   - Add satin, rib knit, jersey and velvet generators.
   - Add a tangent attribute to the vertex layout; build tangents from
     pattern-space UVs, which are already in mm.
4. **Real-world texture scale.** Set UVs in mm and give each texture a
   mm-per-tile value, so a twill has the right thread size on any garment.
   Add mipmaps and anisotropic filtering on the sampler.
5. **Imported fabric photos.** For a user-supplied image, auto-generate a
   normal map (Sobel on luminance) and a roughness map (inverted, normalised
   luminance). Add an intensity slider for each.
6. **Thin-fabric translucency.** A wrap-lighting back term weighted by
   `translucency`, plus alpha for sheer fabrics. Sheer fabrics need the
   depth sort that is still open from Phase 2.
7. **Presets.** Retune cotton, silk and denim, and add satin, velvet, jersey
   and chiffon.

Done when: one turntable of the same dress in five presets shows clearly
different materials.

## R4: Shadows and ambient occlusion

Goal: depth and contact. The garment sits on the body, and folds have
shadowed valleys.

1. **Key-light shadow map.** Add a depth pass from the key light with a
   bounds-fitted ortho frustum, PCF, and PCSS for soft edges. Garment and
   avatar both cast and receive, so the skirt hem shadows the legs and the
   arms shadow the torso.
2. **Screen-space AO.** Add a GTAO (or SSAO plus a bilateral blur) pass in
   the still renderer, using depth and normals from a G-buffer prepass. It
   multiplies the ambient and IBL diffuse terms only. Remove the view-angle
   fake from `fs_lit`.
3. **Floor contact shadow.** Floor contact comes from the shadow map, plus
   an AO-like term near the feet. Drop `blob_shadow` from final renders.

Done when: fold valleys, underarms and the waistband show occlusion, and
the hem casts a soft shadow on the legs.

## R5: Garment details and avatar quality

Goal: the small details that make it read as a sewn garment.

1. **Rendered puckering.** Phase 4's puckering becomes a procedural normal
   or displacement ripple along seam lines in the final render: amplitude,
   wavelength and falloff from the puckering settings.
2. **3D topstitches.** Thread geometry (small tube or capsule instances
   along the stitch path, offset by thickness) or an alpha-tested decal
   strip, with thread colour and thickness. Use it for hems by default.
3. **Avatar.** Increase the segment counts and subdivide the procedural
   mannequin. Give it proper hands (closed fingers), a refined head and
   feet, and a matte "dress form" material with slight sheen. Optional:
   import an OBJ/glTF avatar with a skeleton mapping.
4. **Buttons, zippers and trims.** Give the Phase 4 trims PBR materials
   (metal: metalness 1, low roughness; plastic and horn: dielectric).

Done when: close-ups show seam texture and stitching, and the avatar no
longer looks low-poly.

## R6: Offline final render

Goal: a "Render" button that produces product shots on par with CLO's
V-Ray output.

1. **Path tracer.** A wgpu compute path tracer that reuses
   `od_render/src/bvh.rs`. It supports the R3 BRDF (diffuse, GGX, sheen),
   HDRI importance sampling, the key light, and progressive accumulation
   with a sample count and time limit. Thin-sheet transmission for sheer
   fabrics.
2. **Denoising.** Run OpenImageDenoise (Apache-2.0) on colour, albedo and
   normal AOVs, behind a cargo feature because it needs the native library.
   Fallback: a simple joint-bilateral denoise.
3. **Camera.** Focal length, depth of field (aperture and focus on the
   garment), and supersampled anti-aliasing for the raster path.
4. **Outputs.** PNG/EXR stills, turntable sequences, and a transparent
   background option for e-commerce cut-outs. Add `render` with
   `quality: "preview" | "final" | "path"` to MCP.
5. **Alternative route.** Export glTF with the sheen extension, or USD, and
   render in headless Blender Cycles. It's cheaper to build but needs
   Blender installed. Decide at the start of R6.

Done when: a path-traced render of the site's top and skirt is published on
the website next to the raster renders.

---

## Order and effort

| Phase | Depends on | Rough size | Why this position |
|---|---|---|---|
| R1 | – | S–M | Largest visual gain; HDR target and IBL are needed by R3/R4 |
| R2 | – (can run parallel to R1) | M | Fixes the most visible flaws in the current site renders |
| R3 | R1 | M–L | Needs IBL to look right; format bump v13 |
| R4 | R1 | M | Needs the HDR target; G-buffer prepass shared with R6 AOVs |
| R5 | R2, R3 | M | Builds on thickness and PBR |
| R6 | R1, R3 (R4 optional) | L | Reuses BRDF and HDRI work |

R1 and R2 touch different crates (`od_render` versus `od_sim`/`od_mcp`), so
they can be built in parallel worktrees.
