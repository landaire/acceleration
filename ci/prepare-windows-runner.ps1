<#
.SYNOPSIS
Prepares a GitHub-hosted Windows runner for Buck2 and copies the checkout to a short path.

.DESCRIPTION
- Windows Defender real-time scanning holds files that Buck2 has just
  written, and Buck2 then fails to replace them with "Access is denied". The
  build directories are excluded from scanning.
- buck-out paths carry configuration hashes and target names twice, so long
  paths are enabled.
- Buck2 resolves junctions to the real project root, and link.exe still
  exceeds MAX_PATH under the default workspace path, so the checkout is
  copied to Destination.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Destination,
    # Other directories to exclude from scanning, for example where Buck2 is installed.
    [string[]]$AlsoExclude = @()
)

$ErrorActionPreference = "Stop"

$exclusions = @($Destination) + $AlsoExclude
if ($env:RUNNER_TEMP) {
    $exclusions += $env:RUNNER_TEMP
}
if ((Get-MpComputerStatus).RealTimeProtectionEnabled) {
    foreach ($path in $exclusions) {
        Add-MpPreference -ExclusionPath $path
    }
    $applied = @((Get-MpPreference).ExclusionPath)
    foreach ($path in $exclusions) {
        if (-not ($applied | Where-Object { $_.TrimEnd('\') -ieq $path.TrimEnd('\') })) {
            throw "The Defender exclusion for $path did not apply."
        }
    }
}

$fileSystem = 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem'
Set-ItemProperty -Path $fileSystem -Name LongPathsEnabled -Value 1
if ((Get-ItemProperty -Path $fileSystem -Name LongPathsEnabled).LongPathsEnabled -ne 1) {
    throw "LongPathsEnabled did not take the value 1."
}

robocopy $Source $Destination /E /NFL /NDL /NJH /NJS /NP | Out-Null
# robocopy exit codes below 8 report success.
if ($LASTEXITCODE -ge 8) {
    throw "robocopy from $Source to $Destination failed with exit code $LASTEXITCODE."
}
if (-not (Test-Path (Join-Path $Destination ".buckroot"))) {
    throw "$Destination has no .buckroot after the copy."
}
exit 0
