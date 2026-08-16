[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$SkipBuild,
    [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA "Programs\Roven")
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    throw "LOCALAPPDATA is required for a per-user Roven installation"
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$resolvedInstallRoot = [IO.Path]::GetFullPath($InstallRoot)
$installedBinary = Join-Path $resolvedInstallRoot "roven.exe"
$releaseBinary = Join-Path $repositoryRoot "target\release\roven.exe"

function Get-PathEntries([string]$pathValue) {
    if ([string]::IsNullOrWhiteSpace($pathValue)) {
        return @()
    }
    return @($pathValue -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

function Remove-UserPathEntry([string]$pathValue, [string]$entryToRemove) {
    $entries = @(Get-PathEntries $pathValue)
    $matchingEntries = @($entries | Where-Object {
        [string]::Equals($_.TrimEnd('\'), $entryToRemove.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)
    })
    if ($matchingEntries.Count -eq 0) {
        return $pathValue
    }
    $remainingEntries = @($entries | Where-Object {
        -not [string]::Equals($_.TrimEnd('\'), $entryToRemove.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)
    })
    return ($remainingEntries -join ';')
}

function Add-UserPathEntry([string]$pathValue, [string]$entryToAdd) {
    $entries = @(Get-PathEntries $pathValue)
    $alreadyPresent = $entries | Where-Object {
        [string]::Equals($_.TrimEnd('\'), $entryToAdd.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)
    }
    if ($null -ne $alreadyPresent) {
        return $pathValue
    }
    $entries += $entryToAdd
    return ($entries -join ';')
}

$userPathValue = [Environment]::GetEnvironmentVariable("Path", "User")
$originalUserPath = if ($null -eq $userPathValue) { "" } else { $userPathValue }

if ($Uninstall) {
    $updatedUserPath = Remove-UserPathEntry $userPathValue $resolvedInstallRoot
    if ($updatedUserPath -ne $originalUserPath) {
        try {
            [Environment]::SetEnvironmentVariable("Path", $updatedUserPath, "User")
        } catch {
            throw "Could not update the user PATH during uninstall. Run this script in a normal user PowerShell session. $($_.Exception.Message)"
        }
        $env:Path = Remove-UserPathEntry $env:Path $resolvedInstallRoot
    }
    if (Test-Path -LiteralPath $installedBinary -PathType Leaf) {
        Remove-Item -LiteralPath $installedBinary -Force
    }
    if (Test-Path -LiteralPath $resolvedInstallRoot -PathType Container) {
        $remainingItems = @(Get-ChildItem -LiteralPath $resolvedInstallRoot -Force)
        if ($remainingItems.Count -eq 0) {
            Remove-Item -LiteralPath $resolvedInstallRoot -Force
        }
    }
    Write-Output "Roven uninstalled from $resolvedInstallRoot"
    Write-Output "Open a new PowerShell session for PATH changes to take effect"
    exit 0
}

if ($SkipBuild) {
    if (-not (Test-Path -LiteralPath $releaseBinary -PathType Leaf)) {
        throw "Release binary not found at $releaseBinary"
    }
} else {
    & cargo build --release --manifest-path (Join-Path $repositoryRoot "Cargo.toml")
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release failed"
    }
}

New-Item -ItemType Directory -Force -Path $resolvedInstallRoot | Out-Null
Copy-Item -LiteralPath $releaseBinary -Destination $installedBinary -Force

$updatedUserPath = Add-UserPathEntry $userPathValue $resolvedInstallRoot
if ($updatedUserPath -ne $originalUserPath) {
    try {
        [Environment]::SetEnvironmentVariable("Path", $updatedUserPath, "User")
    } catch {
        throw "Roven was copied to $installedBinary, but user PATH registration failed. Run this script in a normal user PowerShell session, then add $resolvedInstallRoot to your user PATH. $($_.Exception.Message)"
    }
    $env:Path = Add-UserPathEntry $env:Path $resolvedInstallRoot
}

& $installedBinary --version
if ($LASTEXITCODE -ne 0) {
    throw "The installed Roven binary failed its version check"
}

Write-Output "Roven installed at $installedBinary"
Write-Output "Open a new PowerShell session, then run: roven --help"
