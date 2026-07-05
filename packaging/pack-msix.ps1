# Builds the release exe and packages it as an MSIX for Microsoft Store
# submission (or local side-load testing).
#
#   .\packaging\pack-msix.ps1           # unsigned package, ready for Store upload
#   .\packaging\pack-msix.ps1 -Sign     # signed with .\devcert.pfx for local install
#
# The Store re-signs packages on ingestion, so the unsigned package is what
# gets uploaded to Partner Center. Signing is only needed to Add-AppxPackage
# locally (generate the cert once with: winapp cert generate).

param(
    [switch]$Sign
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

# Release build; skip NSIS/MSI bundling — MSIX packs the bare exe (frontend
# assets are embedded in it by Tauri).
npm run tauri -- build --no-bundle
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

# Stage: just the exe plus the manifest's Assets.
$stage = Join-Path $root "dist-msix"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory $stage | Out-Null
Copy-Item "$root\src-tauri\target\release\pixnip.exe" $stage
Copy-Item -Recurse "$root\Assets" "$stage\Assets"

if ($Sign) {
    winapp pack $stage --cert "$root\devcert.pfx"
} else {
    winapp pack $stage
}
if ($LASTEXITCODE -ne 0) { throw "winapp pack failed" }

Get-ChildItem "$root\*.msix" | ForEach-Object { "packed: $($_.Name) ($([math]::Round($_.Length / 1MB, 1)) MB)" }
