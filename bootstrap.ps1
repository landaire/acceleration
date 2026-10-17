<#
.SYNOPSIS
Installs DotSlash into .tools and fetches the Buck2 release that toolchains\buck2 pins.

.DESCRIPTION
For Windows. bin\buck2.cmd runs this on first use. Linux and macOS use
bootstrap.sh.

Buck2 downloads everything else it builds with. The host tools it does not
download are checked at the end and reported, not installed.

Run it with: powershell -ExecutionPolicy Bypass -File bootstrap.ps1
#>
$ErrorActionPreference = "Stop"

$DotSlashVersion = "v0.5.9"
$Assets = @{
    "AMD64" = @{
        Name = "dotslash-windows.$DotSlashVersion.tar.gz"
        Sha256 = "c925840a9bf9105a187cd120278cb023c51dac0cecb3234855152422751b7b41"
    }
    "ARM64" = @{
        Name = "dotslash-windows-arm64.$DotSlashVersion.tar.gz"
        Sha256 = "43d49d3472808738993ccf95a36dda8cc2869996d9a397bb02bde1e39b16146b"
    }
}

$Root = $PSScriptRoot
$Bin = Join-Path $Root ".tools\bin"
$DotSlash = Join-Path $Bin "dotslash.exe"
$Stamp = Join-Path $Bin "dotslash.sha256"
# The Windows curl and tar: Git's GNU tar reads a drive letter as a host name.
$Curl = Join-Path $env:SystemRoot "System32\curl.exe"
$Tar = Join-Path $env:SystemRoot "System32\tar.exe"

function Install-DotSlash($Asset) {
    foreach ($tool in $Curl, $Tar) {
        if (-not (Test-Path $tool)) {
            throw "$tool is required to download DotSlash."
        }
    }

    New-Item -ItemType Directory -Force -Path $Bin | Out-Null
    $scratch = Join-Path $Root (".tools\download-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $scratch | Out-Null
    try {
        Write-Host "Downloading DotSlash $DotSlashVersion"
        $archive = Join-Path $scratch $Asset.Name
        & $Curl -fsSL --retry 3 -o $archive "https://github.com/facebook/dotslash/releases/download/$DotSlashVersion/$($Asset.Name)"
        if ($LASTEXITCODE -ne 0) {
            throw "curl could not download $($Asset.Name)."
        }

        $actual = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
        if ($actual -ne $Asset.Sha256) {
            throw "$($Asset.Name) has SHA-256 $actual, not $($Asset.Sha256)."
        }

        & $Tar -xzf $archive -C $scratch dotslash.exe
        if ($LASTEXITCODE -ne 0) {
            throw "tar could not extract $($Asset.Name)."
        }

        try {
            Move-Item -Force -Path (Join-Path $scratch "dotslash.exe") -Destination $DotSlash
        }
        catch {
            # Another bootstrap installed this release first and is running it,
            # and Windows does not replace a running executable.
            $current = if (Test-Path $Stamp) { (Get-Content -Path $Stamp -TotalCount 1).Trim() }
            if ($current -ne $Asset.Sha256) {
                throw
            }

            return
        }

        Set-Content -Encoding ascii -Path $Stamp -Value $Asset.Sha256
    }
    finally {
        Remove-Item -Recurse -Force -Path $scratch
    }
}

function Write-Missing($Tool, $Reason) {
    if (-not $script:Reported) {
        Write-Host "Missing host tools. Buck2 does not download these:"
        $script:Reported = $true
    }

    Write-Host "  ${Tool}: $Reason"
}

function Test-HostTools($Architecture) {
    # Build scripts compile C and C++, and the MSVC linker links.
    $component = if ($Architecture -eq "ARM64") {
        "Microsoft.VisualStudio.Component.VC.Tools.ARM64"
    }
    else {
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"
    }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    $vcTools = if (Test-Path $vswhere) {
        & $vswhere -latest -products * -requires $component -property installationPath
    }
    if (-not $vcTools) {
        Write-Missing "MSVC build tools" "required to compile and link. Install Visual Studio Build Tools with the C++ workload."
    }

    $kits = Get-ItemProperty -Path "HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots" -Name KitsRoot10 -ErrorAction SilentlyContinue
    if (-not $kits) {
        Write-Missing "Windows SDK" "required to compile and link. The C++ workload of the Build Tools installs it."
    }

    $fileSystem = Get-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" -Name LongPathsEnabled -ErrorAction SilentlyContinue
    if (-not $fileSystem -or $fileSystem.LongPathsEnabled -ne 1) {
        Write-Missing "long paths" "required. buck-out paths exceed 260 characters. As administrator, set LongPathsEnabled to 1 under HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem."
    }

    # Windows puts a python alias on PATH that only opens the Microsoft Store.
    $pythonFound = $false
    if (Get-Command python -ErrorAction SilentlyContinue) {
        try {
            & python -c "import sys; sys.exit(sys.version_info < (3, 11))" 2>$null
            $pythonFound = $LASTEXITCODE -eq 0
        }
        catch {
            $pythonFound = $false
        }
    }
    if (-not $pythonFound) {
        Write-Missing "python" "optional. The build_defs scripts (lint, packaging, build file generation) need Python 3.11 or later."
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Missing "cargo" "optional. Cargo builds and build_defs\update_buck_files.py need rustup and the release in rust-toolchain.toml."
    }
    if (-not (Get-Command reindeer -ErrorAction SilentlyContinue)) {
        Write-Missing "reindeer" "optional. Only build_defs\update_buck_files.py needs it, after a change to a Cargo.toml."
    }
}

# 32-bit PowerShell on a 64-bit system reports x86 here and the system's
# architecture in PROCESSOR_ARCHITEW6432.
$architecture = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
$asset = $Assets[$architecture]
if (-not $asset) {
    throw "No DotSlash release for processor architecture $architecture."
}

$installed = if (Test-Path $Stamp) { (Get-Content -Path $Stamp -TotalCount 1).Trim() }
if (-not (Test-Path $DotSlash) -or $installed -ne $asset.Sha256) {
    Install-DotSlash $asset
}

Write-Host "Fetching Buck2"
& (Join-Path $Root "bin\buck2.cmd") --version
if ($LASTEXITCODE -ne 0) {
    throw "Buck2 does not run."
}

Test-HostTools $architecture
Write-Host "Run bin\buck2 in place of buck2, or add $Root\bin to PATH."
