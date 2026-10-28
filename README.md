# OpenDesign

Open-source 3D garment design: draw flat patterns in 2D, sew them, and drape
them on an avatar in 3D.

Phases 1–4 of the roadmap are in: docked panels, project files, 2D pattern
drafting with curves, a GPU-rendered 3D view with sized avatars, and undo/redo
for every change. Sewing and cloth simulation come next.

![OpenDesign app shell](docs/screenshot.jpg)

## Run

```bash
cargo run -p od_app
```

The binary is called `opendesign`. Release build: `cargo build --release`.

### Windows without Visual Studio

This machine builds with Rust's GNU toolchain (`stable-x86_64-pc-windows-gnu`),
which needs MinGW's `dlltool` and `as`. MSYS2's UCRT64 tools provide them, so
put them on PATH for the build (the finished `.exe` does not need them):

```powershell
$env:Path = "C:\msys64\ucrt64\bin;$env:Path"
cargo run -p od_app
```

With Visual Studio Build Tools installed, the default MSVC toolchain works
without this step.

## Use

| Action | How |
|---|---|
| New / Open / Save / Save As | `Ctrl+N` / `Ctrl+O` / `Ctrl+S` / `Ctrl+Shift+S`, or File menu |
| Reopen a recent project | File → Open Recent, or drag an `.odproj` onto the window |
| Quit | `Ctrl+Q` (asks first if there are unsaved changes) |
| Add a starter block | Library → `+ Front bodice` etc. (T-shirt front, back, sleeve) |
| Draw a rectangle pattern | Rectangle tool (`R`), drag in the 2D window |
| Draft an outline | Pen tool (`P`): click points, `Shift`+click for a curve point; click the first point or `Enter` to close, `Esc` cancels |
| Edit points | Edit points tool (`E`): drag a point; double-click an edge to add one, a point to curve it; `C` toggles curve, `Del` deletes |
| Snap to grid / points | Toolbar `Grid snap` / `Point snap`, or View menu; length and angle show while drawing |
| Seam allowance, grainline | Properties: drag or type the value |
| Notches | Notch tool (`N`): click an outline to add, click a notch to remove |
| Internal lines | Internal line tool (`L`): click inside a pattern, `Enter` or double-click to finish |
| Flip, mirror, duplicate | Properties → Symmetry; `Ctrl+D` duplicates |
| Rotate a piece | Select tool: drag the round handle above it (`Shift` for 15° steps), or Properties → Rotate (±45°, ±90°) |
| Cut, copy, paste | `Ctrl+X` / `Ctrl+C` / `Ctrl+V` (or Edit menu): copies keep the seams and tacks between them; `Ctrl+Shift+V` pastes a mirror image beside them |
| Sew | Sew tool (`S`): click a segment (or drag along an outline for a free stretch, which may cross the outline's start), `Tab` for the other side, `Enter` sews; add more stretches per side for one-to-many. Pick both sides near matching ends and the seam is sewn untwisted. Click a sewn edge to unsew it |
| Edit a seam | Select tool: click a seam to see its side lengths in Properties; drag the handles at its ends to move them along the outline; Flip side B untwists it, `Del` deletes it |
| Check sewing length | View → Check sewing length (on, 3% by default): mismatched seams show in the warning colour, and the status bar counts them (click to select the first) |
| Tack | Tack tool (`K`): click a point on a pattern, then a point on another pattern (2D or 3D view) or on the avatar (3D view). Works on the flat pieces and on the draped cloth; an avatar tack stays on the body when it is resized. `Esc` cancels; select a tack and `Del` deletes it. Clicking two exact outline points with the Sew tool also tacks them |
| Drape | Drape button above the 3D view; Pin and Strain toggles while draping; Cloth collision keeps pieces from passing through each other |
| Particle distance | Properties -> Simulation: spacing of a piece's simulation particles (default 20 mm; smaller is finer and slower) |
| Select / move | Select tool (`V`), click or drag a pattern; `Shift`/`Ctrl`+click adds or removes, drag a box from empty space to select what it touches, `Ctrl+A` selects all, `Esc` clears. Moves, flips and deletes act on every selected piece |
| Hide, lock | Objects tab (next to Library): Show and Lock per piece, plus every seam and tack. Hidden pieces leave both views and the drape; locked pieces can't be moved or edited |
| Right-click menu | Right-click a piece in either view: cut, copy, paste, hide, lock, rotate, flip, zoom to, delete |
| Zoom to selection | `F`, in both views; the piece under the pointer is outlined in both |
| Delete | `Del` (the selected point when editing points), or the button in Properties |
| Undo / redo | `Ctrl+Z` / `Ctrl+Y` (or `Ctrl+Shift+Z`) |
| Pan / zoom 2D | Right-drag / scroll |
| Orbit / pan / zoom 3D | Drag / right- or middle-drag / scroll; double-click or Frame to fit everything |
| 3D camera presets | Front, Back, Left, Right, Top, Bottom and Turn ±45° buttons above the 3D view |
| Move pieces in 3D | Before draping, drag the selected piece's arrows to move it along an axis, or its ring to turn it (`Shift` for 15° steps). Arrange shows spots on the avatar: click one to place the selected pieces there. Properties → 3D position turns or resets them |
| Pull the cloth | While draping, drag the cloth to pull it; drag elsewhere to orbit |
| Show and display | Show in 3D: avatar, seams, trims, pins. Display: avatar and cloth solid, wireframe or see-through, and cloth mesh edges |
| Select a pattern in 3D | Click it (the avatar blocks patterns behind it) |
| Change avatar | Library -> Avatars: Female, Male, Kid, or Load avatar file (OBJ/glTF, Y up) |
| Avatar size | Avatar size button in the 3D view: height, chest, waist, hip |
| Rearrange panels | Drag a tab to another panel edge |
| Reset panels, theme | View menu |

Panel layout and theme are remembered between sessions (eframe stores them in
`%APPDATA%\OpenDesign\data\app.ron` on Windows). View → Reset layout restores
the default arrangement.

### Project files

Projects are saved as `.odproj`: versioned JSON
(`{"format": "opendesign-project", "version": 11, "document": {...}}`).
Older versions are upgraded when opened.
Saves go through a temporary file and a rename, so a crash mid-save never
leaves a half-written project. The title bar shows `*` and the status bar
says "Unsaved changes" until you save; New, Open, Quit and closing the window
ask before discarding them.

### Smart App Control

If Windows Smart App Control is on, it blocks freshly built, unsigned programs
("An Application Control policy has blocked this file"). That includes
`opendesign.exe` and the build scripts Cargo compiles, so native Windows builds
fail. Building in WSL still works (`cargo test --workspace`), and
`cargo check --target x86_64-pc-windows-gnu` there type-checks the Windows
build. Code-signing the binary is the long-term fix for releases.

## Layout

```
crates/
  od_core/   Document model, commands, undo/redo history, 2D geometry,
             project file format (project.rs). No UI.
  od_app/    egui + egui_dock desktop app.
    src/app.rs           Shell: menus, toolbar, status bar, dock layout, UI tests
    src/files.rs         Open/save, recent files, unsaved-changes prompt
    src/panels/          One module per dockable tab
    src/theme.rs         Design tokens (light/dark) for widgets and canvases
```

Every edit is a `Command` (`apply` + `undo`) run through `History`, so a new
tool gets undo/redo by adding one command type in `od_core`.

## Test

```bash
cargo test --workspace
```

`od_app` tests use `egui_kittest` to click and type through the real UI
without opening a window.

## Roadmap

1. App shell ✅ docked panels, menus, toolbar, status bar, token-based
   light/dark theme, layout and theme remembered between sessions
2. Core and save/load ✅ commands and undo, `.odproj` files, unsaved-changes
   tracking and prompts, recent files, drag-and-drop open
3. 2D pattern drafting ✅ pen and edit-points tools, curve points, snapping
   with live length/angle, seam allowance, notches, internal lines,
   grainline, flip/mirror/duplicate, T-shirt blocks, project format v2
4. 3D viewport ✅ wgpu renderer (lit, depth-tested, floor grid, contact
   shadow), orbit camera with presets, procedural sized avatars, OBJ/glTF
   avatar loading, patterns placed around the avatar, click-to-select in 3D
5. Sewing ✅ segment and free sewing with a direction per edge (so seams
   drape untwisted), one-to-many seams, seams and tacks kept in step with
   outline edits, seam selection and end-dragging, a tunable length check,
   tacks anywhere on a pattern or to the avatar, seam and tack threads in 3D
6. Cloth simulation ✅ XPBD with interior particles at a set spacing,
   avatar and cloth-to-cloth collision, pins, strain maps
7. Fabrics and materials
8. Asset library
9. Import/export (OBJ, glTF, DXF-AAMA)
10. Grading, measurements, trims, rendering
11. Python plugins (pyo3), localisation, installer
