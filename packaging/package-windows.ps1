#!/usr/bin/env pwsh
# Minimal Windows packaging for OpenDesign: build a release binary and
# bundle it with the top-level docs into a .zip under dist/. Mirrors
# package-linux.sh; see packaging/README.md for what this deliberately
# doesn't do (no installer, no signing, no system package).
#
# Usage: packaging/package-windows.ps1 [-Target <triple>]
# Defaults to the host triple (run `rustc -vV` to see what that is).
# Meant to run on a plain Windows machine or CI runner with cargo and
# PowerShell, not WSL.

param(
    [string]$Target = ""
)

$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$binName = "opendesign.exe"
$cargoToml = Get-Content "Cargo.toml" -Raw
$version = ([regex]::Match($cargoToml, '(?m)^version\s*=\s*"([^"]+)"')).Groups[1].Value

if ($Target -ne "") {
    cargo build --release --target $Target -p od_app -p od_mcp
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $binPath = "target/$Target/release/$binName"
    $archiveName = "opendesign-$version-$Target.zip"
} else {
    cargo build --release -p od_app -p od_mcp
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $binPath = "target/release/$binName"
    $hostTriple = ((rustc -vV | Select-String "^host: (.+)$").Matches[0].Groups[1].Value)
    $archiveName = "opendesign-$version-$hostTriple.zip"
}

if (-not (Test-Path $binPath)) {
    Write-Error "expected binary at $binPath, build did not produce it"
    exit 1
}

$stageDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid())
$pkgDir = Join-Path $stageDir "opendesign-$version"
New-Item -ItemType Directory -Force -Path $pkgDir | Out-Null
Copy-Item $binPath $pkgDir
# The MCP server, for driving OpenDesign from an AI assistant.
Copy-Item (Join-Path (Split-Path $binPath) "opendesign-mcp.exe") $pkgDir
foreach ($f in @("README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE")) {
    if (Test-Path $f) { Copy-Item $f $pkgDir }
}

New-Item -ItemType Directory -Force -Path "dist" | Out-Null
$out = "dist/$archiveName"
if (Test-Path $out) { Remove-Item $out }
Compress-Archive -Path $pkgDir -DestinationPath $out

Remove-Item -Recurse -Force $stageDir

Write-Output "Wrote $out"
