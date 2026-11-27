param(
    [Parameter(Mandatory = $true)]
    [string] $Version
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$target = Join-Path $root 'target'

function Get-CargoManifestVersion {
    $manifest = Join-Path $root 'Cargo.toml'
    $section = ''
    foreach ($line in Get-Content -LiteralPath $manifest) {
        if ($line -match '^\s*\[(?<name>[^\]]+)\]\s*$') {
            $section = $Matches['name']
            continue
        }
        if ($section -eq 'package' -and $line -match '^\s*version\s*=\s*"(?<version>[^"]+)"') {
            return $Matches['version']
        }
    }
    throw "Could not read [package] version from $manifest"
}

if ($Version -match '^v(?<requested>\d+\.\d+\.\d+(?:-rc\d+)?)$') {
    $requestedVersion = $Matches['requested']
} else {
    throw "Version must look like v0.7.0 (or v0.7.0-rc2 if Cargo.toml uses a -rc suffix), got: $Version"
}
$manifestVersion = Get-CargoManifestVersion
if ($manifestVersion -ne $requestedVersion) {
    throw "Version label '$Version' does not match Cargo.toml version '$manifestVersion' (expected 'v$manifestVersion'). Update the label or Cargo.toml before packaging."
}

function Invoke-Cargo {
    param(
        [string[]] $Arguments,
        [switch] $PassOutput
    )
    $out = Join-Path $target 'package-cargo.out.log'
    $err = Join-Path $target 'package-cargo.err.log'
    $process = Start-Process -FilePath 'cargo' -ArgumentList $Arguments `
        -WorkingDirectory $root -RedirectStandardOutput $out -RedirectStandardError $err `
        -Wait -PassThru
    $stdout = if (Test-Path -LiteralPath $out) { [string] (Get-Content -Raw -LiteralPath $out) } else { '' }
    $stderr = if (Test-Path -LiteralPath $err) { [string] (Get-Content -Raw -LiteralPath $err) } else { '' }
    if (-not $PassOutput) {
        if ($stdout) { Write-Host $stdout }
        if ($stderr) { Write-Host $stderr }
    }
    if ($process.ExitCode -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $($process.ExitCode)"
    }
    if ($PassOutput) { return $stdout }
}

function New-ThirdPartyLicenseBundle {
    param(
        [string] $MetadataJson,
        [string] $Destination
    )
    $metadata = $MetadataJson | ConvertFrom-Json
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add('Third-party license notices for Rust Player')
    $lines.Add('')
    $lines.Add('Generated at package time from Cargo metadata and the source packages built for this release.')
    $lines.Add('Declared license expressions come from each crate manifest. License texts are copied verbatim')
    $lines.Add('from LICENSE/LICENCE/COPYING/NOTICE files in the matching local package source when present.')
    $lines.Add('No license text is invented: packages without a local license file are listed with their')
    $lines.Add('declared license and crates.io URL so the upstream terms can be checked.')
    $lines.Add('')
    $count = 0
    foreach ($package in @($metadata.packages)) {
        if (-not $package.source) { continue } # local workspace member, not a third-party crate
        $dir = Split-Path -Parent $package.manifest_path
        $lines.Add("===== $($package.name) $($package.version) =====")
        $lines.Add("Source: $($package.source)")
        $declared = if ($package.license) { [string] $package.license } else { 'not declared in crate manifest' }
        $lines.Add("Declared license: $declared")
        $files = @()
        if ($package.license_file) {
            $candidate = [string] $package.license_file
            if (-not [IO.Path]::IsPathRooted($candidate)) { $candidate = Join-Path $dir $candidate }
            $files += $candidate
        }
        $files += @(Get-ChildItem -LiteralPath $dir -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' } |
            ForEach-Object { $_.FullName })
        $files = @($files | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -Unique)
        if ($files.Count -eq 0) {
            $lines.Add("No license file in the local source; see https://crates.io/crates/$($package.name)/$($package.version) for the upstream terms.")
        } else {
            foreach ($file in $files) {
                if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { continue }
                $lines.Add("--- $(Split-Path -Leaf $file) ---")
                $lines.Add(([string] (Get-Content -Raw -LiteralPath $file)).TrimEnd())
            }
        }
        $lines.Add('')
        $count++
    }
    $lines.Insert(2, "Third-party packages: $count")
    Set-Content -LiteralPath $Destination -Value $lines -Encoding utf8
    return $count
}

New-Item -ItemType Directory -Path $target -Force | Out-Null

Write-Host "Building and checking $Version (Windows x64, PC only)"
Invoke-Cargo @('fmt', '--check')
Invoke-Cargo @('clippy', '--release', '--locked', '--all-targets', '--', '-D', 'warnings')
Invoke-Cargo @('test', '--release', '--locked')
Invoke-Cargo @('build', '--release', '--locked')

$playerExe = Join-Path $root 'target/release/rust-player.exe'
$updaterExe = Join-Path $root 'target/release/rust_player_updater.exe'
foreach ($exe in @($playerExe, $updaterExe)) {
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
        throw "Expected build output is missing: $exe"
    }
}

$packageDir = Join-Path $target "release-package/$Version"
if (Test-Path $packageDir) {
    Remove-Item $packageDir -Recurse -Force
}
$stage = Join-Path $packageDir 'stage'
New-Item -ItemType Directory -Path $stage -Force | Out-Null

Copy-Item -LiteralPath $playerExe -Destination (Join-Path $stage 'rust-player.exe')
Copy-Item -LiteralPath $updaterExe -Destination (Join-Path $stage 'Rust-Player-Updater.exe')
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $stage 'LICENSE')
Copy-Item -LiteralPath (Join-Path $root 'THIRD-PARTY-NOTICES.md') -Destination (Join-Path $stage 'THIRD-PARTY-NOTICES.md')
Copy-Item -LiteralPath (Join-Path $root 'docs/CHANGELOG.md') -Destination (Join-Path $stage 'CHANGELOG.md')
Copy-Item -LiteralPath (Join-Path $root 'docs/RELEASE-NOTES-NEXT.md') -Destination (Join-Path $stage 'RELEASE-NOTES-NEXT.md')

$metadataJson = Invoke-Cargo @('metadata', '--locked', '--format-version', '1') -PassOutput
$licenseCount = New-ThirdPartyLicenseBundle -MetadataJson $metadataJson `
    -Destination (Join-Path $stage 'THIRD-PARTY-LICENSES.txt')
Write-Host "Third-party license bundle: $licenseCount packages"

$zipName = "Rust-Player-$Version-Windows-x64.zip"
$zip = Join-Path $packageDir $zipName
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
$hash = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
Set-Content -Path "$zip.sha256" -Value "$hash  $zipName" -Encoding ascii

# src/update.rs downloads this exact asset name and checks GitHub's sha256 digest.
# The updater replaces rust-player.exe; it does not install the ZIP itself.
$updateExeName = "Rust-Player-$Version-Windows-x64.exe"
$updateExe = Join-Path $packageDir $updateExeName
Copy-Item -LiteralPath $playerExe -Destination $updateExe
$updateHash = (Get-FileHash -Algorithm SHA256 -Path $updateExe).Hash.ToLowerInvariant()
Set-Content -Path "$updateExe.sha256" -Value "$updateHash  $updateExeName" -Encoding ascii

Write-Host "Package: $zip"
Write-Host "SHA256:  $hash"
Write-Host "Updater asset: $updateExe"
Write-Host "Updater SHA256: $updateHash"
Write-Host "Publishing the updater asset requires an approved maintainer step; this script does not upload it."
Write-Host "No Git tag, commit, push or GitHub release was created."
