param(
    [Parameter(Mandatory = $true)]
    [string] $Version
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^v\d+\.\d+\.\d+(-rc\d+)?$') {
    throw "Version must look like v0.6.0 or v0.6.0-rc2, got: $Version"
}

$root = Split-Path -Parent $PSScriptRoot
$target = Join-Path $root 'target'
New-Item -ItemType Directory -Path $target -Force | Out-Null

function Invoke-Cargo {
    param([string[]] $Arguments)
    $out = Join-Path $target 'package-cargo.out.log'
    $err = Join-Path $target 'package-cargo.err.log'
    $process = Start-Process -FilePath 'cargo' -ArgumentList $Arguments `
        -WorkingDirectory $root -RedirectStandardOutput $out -RedirectStandardError $err `
        -Wait -PassThru
    Get-Content $out | Write-Host
    Get-Content $err | Write-Host
    if ($process.ExitCode -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $($process.ExitCode)"
    }
}

Write-Host "Building and checking $Version (Windows x64, PC only)"
Invoke-Cargo @('fmt', '--check')
Invoke-Cargo @('clippy', '--release', '--locked', '--all-targets', '--', '-D', 'warnings')
Invoke-Cargo @('test', '--release', '--locked')
Invoke-Cargo @('build', '--release', '--locked')

$packageDir = Join-Path $target "release-package/$Version"
if (Test-Path $packageDir) {
    Remove-Item $packageDir -Recurse -Force
}
$stage = Join-Path $packageDir 'stage'
New-Item -ItemType Directory -Path $stage -Force | Out-Null

Copy-Item (Join-Path $root 'target/release/rust-player.exe') (Join-Path $stage 'rust-player.exe')
Copy-Item (Join-Path $root 'THIRD-PARTY-NOTICES.md') $stage
Copy-Item (Join-Path $root 'docs/CHANGELOG.md') $stage
Copy-Item (Join-Path $root 'docs/RELEASE-NOTES-NEXT.md') $stage

$zipName = "Rust-Player-$Version-Windows-x64.zip"
$zip = Join-Path $packageDir $zipName
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
$hash = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
Set-Content -Path "$zip.sha256" -Value "$hash  $zipName" -Encoding ascii

Write-Host "Package: $zip"
Write-Host "SHA256:  $hash"
Write-Host "No Git tag, commit, push or GitHub release was created."
