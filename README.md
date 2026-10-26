# OpenDesign

Open-source 3D garment design: draw flat patterns in 2D, sew them, and drape
them on an avatar in 3D.

This is the starter app shell (Phases 1–2 of the roadmap): docked panels,
a 2D pattern canvas, a placeholder 3D view, and undo/redo for every change.

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
| Add a starter block | Library → `+ Front bodice` etc. |
| Draw a rectangle pattern | Rectangle tool (`R`), drag in the 2D window |
| Select / move | Select tool (`V`), click or drag a pattern |
| Delete | `Del`, or the button in Properties |
| Undo / redo | `Ctrl+Z` / `Ctrl+Y` (or `Ctrl+Shift+Z`) |
| Pan / zoom 2D | Right-drag / scroll |
| Orbit / zoom 3D | Drag / scroll |
| Reset panels, theme | View menu |

## Layout

```
crates/
  od_core/   Document model, commands, undo/redo history, 2D geometry. No UI.
  od_app/    egui + egui_dock desktop app.
    src/app.rs           Shell: menus, toolbar, status bar, dock layout, UI tests
    src/panels/          One module per dockable tab
    src/theme.rs         Canvas colour tokens (light/dark)
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

1. App shell ✅
2. Core architecture: commands and undo ✅, save/load next
3. 2D pattern editor: points, curves, notches, seam allowance
4. 3D viewport on wgpu, avatar loading
5. Sewing
6. Cloth simulation
7. Fabrics and materials
8. Asset library
9. Import/export (OBJ, glTF, DXF-AAMA)
10. Grading, measurements, trims, rendering
11. Python plugins (pyo3), localisation, installer
