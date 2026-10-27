# Packaging

`package-linux.sh` and `package-windows.ps1` each build `od_app` in release
mode and bundle the binary (plus README/license files, if present) into
`dist/opendesign-<version>-<target>.{tar.gz,zip}`. That's the whole MVP: no
installer, no signing, no system package — see "What's deliberately not
here" below.

```sh
packaging/package-linux.sh                       # host target
packaging/package-linux.sh x86_64-pc-windows-gnu # cross build, if toolchain is set up
```

```powershell
packaging/package-windows.ps1                     # host target, on a real Windows machine or CI runner
```

## Releases

`.github/workflows/release.yml` runs both scripts in CI (Linux natively on
`ubuntu-latest`, Windows natively on `windows-latest` — not the project's
own Smart-App-Control-restricted machine, see below) and publishes the two
archives to a GitHub Release whenever a tag like `v0.2.0` is pushed:

```sh
git tag v0.2.0
git push origin v0.2.0
```

`workflow_dispatch` lets the two build jobs run on demand (e.g. from a
branch, to sanity-check the build) without publishing anything — only a
tag push triggers the release job. Anyone can then download the archive
for their platform from the repo's Releases page and run the binary
directly; there's still no installer (see below), so on Windows that
means one unsigned-binary SmartScreen prompt to click through.

## What's deliberately not here

- **No `.deb`/`.rpm`/AppImage/Flatpak.** Real Linux packaging means a
  desktop file, an icon, dependency metadata, and in most cases a
  packaging-format-specific CI job. Worth doing once the app has an
  icon and is further along; a zip is enough to hand someone a build
  today.
- **No code signing, no auto-update.** Both are ongoing commitments
  (a signing identity to renew, an update server to run), not one-time
  setup. Out of scope for an MVP.
- **No cargo-dist config.** cargo-dist generates the same kind of
  GitHub Actions release workflow as `release.yml`, plus installer
  scripts (shell + PowerShell + MSI). Now that there's a tagged release
  process for it to plug into, adopting cargo-dist is a reasonable
  upgrade from the hand-rolled workflow here — just not done yet.

## Windows

This repo's own `CLAUDE.md` already flags why Windows is harder than
zipping a binary: Smart App Control on the Windows *dev* machine used
for this project blocks running a freshly compiled, unsigned `.exe`
(os error 4551). That's a per-machine security policy, not something a
packaging script can route around — and we shouldn't try to. That's
why `release.yml`'s Windows build runs on a GitHub-hosted
`windows-latest` runner rather than the project's own Windows checkout.

A real, double-click Windows *installer* would still need:

- **Code signing** (an Authenticode certificate, EV or OV, from a CA)
  so SmartScreen doesn't warn on the binary. This is the actual
  blocker; nothing below matters without it. Without a cert, anyone
  downloading the CI-built `.zip` and running `opendesign.exe` will
  see one unsigned-binary SmartScreen prompt to click through — the
  binary isn't blocked outright (that was specifically Smart App
  Control on the dev machine), just flagged as unrecognized.
- **An installer**, not a bare `.exe` in a zip — most Windows users
  expect `Add or Remove Programs` integration, a Start Menu entry, and
  an uninstaller. WiX (MSI) or Inno Setup (NSIS-like) are the usual
  choices; both can be driven from the existing CI job once there's a
  signing identity to use.

Neither is implemented here: `release.yml` gets a working unsigned
build onto a GitHub Release, which is as far as packaging goes without
paying for a signing certificate.
