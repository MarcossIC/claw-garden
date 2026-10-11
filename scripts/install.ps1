<#
.SYNOPSIS
Installs clawg for the current user, without administrator rights.

.DESCRIPTION
Copies clawg.exe, which must sit next to this script, into -InstallDir and adds
that directory to the user PATH. A prod daemon from a previous install is
stopped first, because Windows will not overwrite a running executable.

.PARAMETER InstallDir
Where clawg.exe is placed. Defaults to %LOCALAPPDATA%\Programs\clawg.

.PARAMETER SkipPath
Leave the user PATH untouched.
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\clawg'),
    [switch]$SkipPath
)

$ErrorActionPreference = 'Stop'

$source = Join-Path $PSScriptRoot 'clawg.exe'
if (-not (Test-Path -LiteralPath $source)) {
    throw "clawg.exe not found next to this script ($source)."
}

$target = Join-Path $InstallDir 'clawg.exe'

# The previous binary is the one that can talk to the running daemon, so it
# does the stopping. "No daemon running" is reported on stderr, which Windows
# PowerShell 5.1 turns into a terminating error under 'Stop', so the check is
# done with errors relaxed.
if (Test-Path -LiteralPath $target) {
    $ErrorActionPreference = 'Continue'
    & $target stop --profile prod *> $null
    $ErrorActionPreference = 'Stop'
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
try {
    Copy-Item -LiteralPath $source -Destination $target -Force
}
catch {
    throw "Could not replace $target. A clawg process still holds it: close the Claude Code sessions that use clawg and run this script again. ($($_.Exception.Message))"
}

if (-not $SkipPath) {
    & $target path add --dir $InstallDir
    if ($LASTEXITCODE -ne 0) {
        throw "clawg could not update the user PATH (exit code $LASTEXITCODE)."
    }
}

& $target --version
Write-Host "Installed to $target"
if (-not $SkipPath) {
    Write-Host 'Open a new terminal so the updated PATH is picked up.'
}
