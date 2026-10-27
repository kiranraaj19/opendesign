# Packaging

`package-linux.sh` builds `od_app` in release mode and bundles the binary
(plus README/license files, if present) into
`dist/opendesign-<version>-<target>.tar.gz`. That's the whole MVP: no
installer, no signing, no system package.

```sh
packaging/package-linux.sh                       # host target
packaging/package-linux.sh x86_64-pc-windows-gnu # cross build, if toolchain is set up
```

## What's deliberately not here

- **No `.deb`/`.rpm`/AppImage/Flatpak.** Real Linux packaging means a
  desktop file, an icon, dependency metadata, and in most cases a
  packaging-format-specific CI job. Worth doing once the app has an
  icon and is further along; a zip is enough to hand someone a build
  today.
- **No code signing, no auto-update.** Both are ongoing commitments
  (a signing identity to renew, an update server to run), not one-time
  setup. Out of scope for an MVP.
- **No cargo-dist config.** cargo-dist generates GitHub Actions
  workflows, installer scripts (shell + PowerShell + MSI), and release
  automation in one go — a good fit once this project is cutting
  tagged releases on a real cadence. Right now there's no release
  process for it to plug into, so adopting it would mean configuring
  a tool around a workflow that doesn't exist yet.

## Windows

This repo's own `CLAUDE.md` already flags why Windows is harder than
zipping a binary: Smart App Control on the Windows dev machine used for
this project blocks running a freshly compiled, unsigned `.exe`
(os error 4551). That's a per-machine security policy, not something a
packaging script can route around — and we shouldn't try to.

A real Windows package would need, roughly:

- **Code signing** (an Authenticode certificate, EV or OV, from a CA)
  so Smart App Control and SmartScreen don't block the binary. This is
  the actual blocker; nothing below matters without it.
- **An installer**, not a bare `.exe` — most Windows users expect
  `Add or Remove Programs` integration, a Start Menu entry, and an
  uninstaller. WiX (MSI) or Inno Setup (NSIS-like) are the usual
  choices; both can be driven from CI once there's a signing identity
  to use.
- **A build host that isn't the Smart-App-Control-restricted machine**,
  or an exception/policy change on it — this project's notes are
  explicit that we don't change that setting, so Windows packaging
  should happen on CI (e.g. a `windows-latest` GitHub Actions runner)
  rather than the local Windows checkout.

None of that is implemented here. `package-linux.sh`'s target-triple
argument will cross-build the Windows binary if a cross toolchain is
set up, but the result is exactly the kind of unsigned `.exe` that
can't be run on the project's own Windows machine — it only
demonstrates that the build step works, not a usable Windows package.
