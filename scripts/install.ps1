<#
.SYNOPSIS
Installs clawg for the current user, without administrator rights.

.DESCRIPTION
Copies clawg.exe, which must sit next to this script, into -InstallDir and adds
that directory to the user PATH. A prod daemon from a previous install is
stopped first, because Windows will not overwrite a running executable.

A legacy install (the old "Claw Garden" installer, or any other clawg.exe
reachable through PATH) can shadow the new binary, so the script refuses to
continue while one exists, and checks at the end that a new terminal resolves
clawg to the file it just installed.

.PARAMETER InstallDir
Where clawg.exe is placed. Defaults to %LOCALAPPDATA%\Programs\clawg.

.PARAMETER SkipPath
Leave the user PATH untouched. The check that clawg resolves to the new binary
is skipped too, since the PATH is no longer this script's doing.

.PARAMETER RemoveLegacy
Uninstall a legacy "Claw Garden" install silently before installing.
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\clawg'),
    [switch]$SkipPath,
    [switch]$RemoveLegacy
)

$ErrorActionPreference = 'Stop'

$LegacyDisplayName = 'Claw Garden'
$UninstallRoots = @(
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
)

function Test-SamePath([string]$A, [string]$B) {
    if (-not $A -or -not $B) { return $false }
    $left = [IO.Path]::GetFullPath($A).TrimEnd('\')
    $right = [IO.Path]::GetFullPath($B).TrimEnd('\')
    return $left -ieq $right
}

# Every distinct clawg.exe a shell would find in PathValue, in resolution order.
# Windows takes quotes around an entry and ignores blank ones.
function Find-ClawgInPath([string]$PathValue, [string]$ExeName = 'clawg.exe') {
    $found = @()
    foreach ($entry in $PathValue.Split(';')) {
        $dir = $entry.Trim().Trim('"')
        if (-not $dir) { continue }
        $candidate = Join-Path $dir $ExeName
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) { continue }
        if ($found | Where-Object { Test-SamePath $_ $candidate }) { continue }
        $found += $candidate
    }
    return , $found
}

# The PATH a newly opened terminal gets: Machine, then User, straight from the
# registry. $env:Path belongs to this process and would miss what was just added.
function Get-FreshPath {
    $parts = foreach ($hive in @(
        'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
        'HKCU:\Environment')) {
        $key = Get-Item -LiteralPath $hive
        $raw = $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        [Environment]::ExpandEnvironmentVariables($raw)
    }
    return ($parts -join ';')
}

# The uninstaller path from an UninstallString, which may be quoted and may
# carry arguments.
function Get-UninstallExe([string]$UninstallString) {
    $s = $UninstallString.Trim()
    if ($s.StartsWith('"')) { return $s.Substring(1).Split('"')[0] }
    if ($s -match '^(.+?\.exe)(\s|$)') { return $Matches[1] }
    return $s
}

function Get-LegacyEntry([string[]]$Roots = $UninstallRoots, [string]$DisplayName = $LegacyDisplayName) {
    foreach ($root in $Roots) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        foreach ($key in Get-ChildItem -LiteralPath $root) {
            if ($key.GetValue('DisplayName') -ne $DisplayName) { continue }
            $uninstall = [string]$key.GetValue('UninstallString')
            # NSIS stores InstallLocation quoted, which Test-Path would not find.
            $location = ([string]$key.GetValue('InstallLocation')).Trim('"')
            if (-not $location -and $uninstall) {
                $location = Split-Path -Parent (Get-UninstallExe $uninstall)
            }
            [pscustomobject]@{
                Key             = $key.PSPath
                UninstallString = $uninstall
                Location        = $location
            }
        }
    }
}

# clawg.exe files on PATH outside the install directory.
function Get-ForeignClawg([string]$PathValue, [string]$InstallDir) {
    $all = Find-ClawgInPath $PathValue
    return , @($all | Where-Object { -not (Test-SamePath (Split-Path -Parent $_) $InstallDir) })
}

# Decides what the shadowing check should say: $null when the first hit is the
# installed binary, otherwise the message naming who wins.
function Get-ShadowMessage([string[]]$Resolved, [string]$Target) {
    if (-not $Resolved -or $Resolved.Count -eq 0) {
        return "clawg.exe is not on the PATH a new terminal gets, so $Target will not be found by name."
    }
    if (Test-SamePath $Resolved[0] $Target) { return $null }
    return "A new terminal would run $($Resolved[0]) (directory $(Split-Path -Parent $Resolved[0])), not $Target. That copy shadows the one just installed: remove it or move it after $(Split-Path -Parent $Target) on the PATH."
}

function Remove-LegacyInstall($Entry) {
    if (-not $Entry.UninstallString) {
        throw "The legacy install at $($Entry.Location) has no UninstallString; remove it by hand."
    }
    $exe = Get-UninstallExe $Entry.UninstallString
    # NSIS uninstallers copy themselves to a temp directory and relaunch, so the
    # process waited on exits early; the entry and folder are what prove it is done.
    Start-Process -FilePath $exe -ArgumentList '/S' -Wait
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline) {
        $gone = -not (Test-Path -LiteralPath $Entry.Key) -and
            (-not $Entry.Location -or -not (Test-Path -LiteralPath $Entry.Location))
        if ($gone) { return }
        Start-Sleep -Milliseconds 500
    }
    throw "The legacy uninstaller finished but $($Entry.Location) or its registry entry ($($Entry.Key)) is still there. Remove it by hand and run this script again."
}

$source = Join-Path $PSScriptRoot 'clawg.exe'
if (-not (Test-Path -LiteralPath $source)) {
    throw "clawg.exe not found next to this script ($source)."
}

$target = Join-Path $InstallDir 'clawg.exe'

# Decided before anything is touched, so a refusal leaves the machine as it was.
$legacy = @(Get-LegacyEntry)
$foreign = Get-ForeignClawg $env:Path $InstallDir
if (($legacy.Count -gt 0 -or $foreign.Count -gt 0) -and -not $RemoveLegacy) {
    $where = @($legacy | ForEach-Object { "$($_.Location) (registered as '$LegacyDisplayName')" }) + @($foreign)
    throw "Another clawg install is present and would shadow the new one: $($where -join '; '). Nothing was changed. Run this script again with -RemoveLegacy to uninstall the registered one."
}
foreach ($entry in $legacy) {
    Write-Host "Removing legacy install at $($entry.Location)"
    Remove-LegacyInstall $entry
}
# A copy no uninstaller owns cannot be removed from here; the check at the end
# still fails loudly if it is the one that wins.
foreach ($path in $foreign) {
    if (Test-Path -LiteralPath $path) {
        Write-Warning "Another clawg.exe is on the PATH and was not removed: $path"
    }
}

# The previous binary is the one that can talk to the running daemon, so it
# does the stopping. "No daemon running" is reported on stderr, which Windows
# PowerShell 5.1 turns into a terminating error under 'Stop', so the check is
# done with errors relaxed.
#
# `stop` exits 1 both when no daemon was running and when it did not end in
# time, so its exit code cannot tell the two apart. `status` exits 0 only when
# a daemon still answers, which is exactly the case where replacing the binary
# would leave the old version serving with the store locked.
if (Test-Path -LiteralPath $target) {
    $ErrorActionPreference = 'Continue'
    & $target stop --profile prod *> $null
    & $target status --profile prod *> $null
    $stillRunning = $LASTEXITCODE -eq 0
    $ErrorActionPreference = 'Stop'
    if ($stillRunning) {
        throw "The clawg daemon for the prod profile is still running after 'clawg stop', so the binary was not replaced. Stop it (or close the editor whose MCP keeps restarting it) and run this script again."
    }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Binaries set aside by earlier runs. One that is still mapped cannot be
# deleted yet and is left for a later run.
Get-ChildItem -LiteralPath $InstallDir -Filter 'clawg.exe.old-*' |
    Remove-Item -Force -ErrorAction SilentlyContinue

# Windows refuses to overwrite an executable that another process has mapped:
# a running clawg, but also an antivirus or the search indexer reading it, with
# no clawg process left. Renaming a mapped file is allowed, so the previous
# binary is moved aside instead of overwritten.
$aside = $null
if (Test-Path -LiteralPath $target) {
    $aside = "$target.old-$(Get-Date -Format yyyyMMddHHmmssfff)"
    try {
        Move-Item -LiteralPath $target -Destination $aside
    }
    catch {
        throw "Could not move the previous $target out of the way. Nothing was changed. ($($_.Exception.Message))"
    }
}
try {
    Copy-Item -LiteralPath $source -Destination $target
}
catch {
    $reason = $_.Exception.Message
    if ($aside) { Move-Item -LiteralPath $aside -Destination $target -ErrorAction SilentlyContinue }
    throw "Could not copy clawg.exe to $target; the previous binary was left in place. ($reason)"
}

if (-not $SkipPath) {
    & $target path add --dir $InstallDir
    if ($LASTEXITCODE -ne 0) {
        throw "clawg could not update the user PATH (exit code $LASTEXITCODE)."
    }

    $shadow = Get-ShadowMessage (Find-ClawgInPath (Get-FreshPath)) $target
    if ($shadow) { throw $shadow }
}

& $target --version
Write-Host "Installed to $target"
if ($SkipPath) {
    Write-Host 'PATH left untouched: not checked whether "clawg" resolves to this install.'
}
else {
    Write-Host 'Open a new terminal so the updated PATH is picked up.'
}
