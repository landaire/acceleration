<#
.SYNOPSIS
Installs the Buck2 release that toolchains/buck2_release.json pins.

.DESCRIPTION
Windows runners have no Nix, which provides Buck2 elsewhere. Downloads the
asset for this host, verifies its SHA-256, and installs it as buck2.exe in
InstallDir. Under GitHub Actions, also adds InstallDir to PATH for later
steps.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$InstallDir
)

$ErrorActionPreference = "Stop"

$pins = Get-Content -Raw (Join-Path $PSScriptRoot "..\toolchains\buck2_release.json") | ConvertFrom-Json
$cpu = switch ($env:PROCESSOR_ARCHITECTURE) {
    "AMD64" { "x86_64" }
    "ARM64" { "aarch64" }
    default { throw "No Buck2 asset for processor architecture $env:PROCESSOR_ARCHITECTURE." }
}
$pin = $pins.assets."$cpu-windows"
if (-not $pin) {
    throw "toolchains/buck2_release.json pins no Buck2 asset for $cpu-windows."
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$compressed = Join-Path $InstallDir $pin.asset
$url = "https://github.com/facebook/buck2/releases/download/$($pins.tag)/$($pin.asset)"
Invoke-WebRequest -Uri $url -OutFile $compressed

$actual = (Get-FileHash -Algorithm SHA256 $compressed).Hash.ToLowerInvariant()
if ($actual -ne $pin.sha256) {
    throw "$($pin.asset) has SHA-256 $actual, not $($pin.sha256)."
}

$executable = Join-Path $InstallDir "buck2.exe"
& zstd --quiet --force --decompress $compressed -o $executable
if ($LASTEXITCODE -ne 0) {
    throw "zstd could not decompress $compressed."
}
Remove-Item $compressed

& $executable --version
if ($LASTEXITCODE -ne 0) {
    throw "$executable does not run."
}
if ($env:GITHUB_PATH) {
    Add-Content -Path $env:GITHUB_PATH -Value $InstallDir
}
