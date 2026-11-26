Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Opt-in UI frame-time session. The logger writes one row per rendered frame and
# adds no file I/O at all when RUST_PLAYER_FRAME_LOG is not set.
$root = Split-Path -Parent $PSScriptRoot
# Prefer the most recently built executable. Old build directories are removed
# as part of cleanup, so never depend on a single fixed path.
$exe = @(
    'target/ui-build/release/rust-player.exe',
    'target/final-build/release/rust-player.exe',
    'target/release/rust-player.exe'
) | ForEach-Object { Join-Path $root $_ } |
    Where-Object { Test-Path $_ } |
    Sort-Object { (Get-Item $_).LastWriteTime } -Descending |
    Select-Object -First 1
if (-not $exe) {
    throw "No release executable found. Run 'cargo build --release --locked' first."
}

$running = @(Get-Process rust-player -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    throw "rust-player is already running (PID $($running[0].Id)). Close it manually; this script never stops a running player."
}

$stamp = Get-Date -Format 'yyyy-MM-dd_HHmmss'
$runDir = Join-Path $root "target/frame-log-runs/$stamp"
New-Item -ItemType Directory -Path $runDir -Force | Out-Null
$csv = Join-Path $runDir 'frames.csv'

$environment = @(
    "run_id=$stamp",
    "started=$(Get-Date -Format o)",
    "executable=$exe",
    "git_commit=$(& git -C $root rev-parse HEAD)",
    "dirty_files=$(@(& git -C $root status --porcelain).Count)",
    "os=$((Get-CimInstance Win32_OperatingSystem).Caption)",
    "frame_log=$csv"
)
$environment | Set-Content -Path (Join-Path $runDir 'environment.txt') -Encoding utf8

$env:RUST_PLAYER_FRAME_LOG = $csv
try {
    Write-Host "Starting $exe"
    Write-Host "Frame log: $csv"
    Write-Host 'Use the player normally, then close it to finish the session.'
    $process = Start-Process -FilePath $exe -WorkingDirectory $root -Wait -PassThru
    Write-Host "Player exited with code $($process.ExitCode)"
}
finally {
    Remove-Item Env:RUST_PLAYER_FRAME_LOG -ErrorAction SilentlyContinue
}

if (-not (Test-Path $csv)) {
    Write-Warning "No frame log was written."
    exit 0
}

function Quantile([double[]]$sorted, [double]$fraction) {
    $index = [math]::Min($sorted.Count - 1, [math]::Floor($fraction * $sorted.Count))
    return $sorted[$index]
}

$rows = @(Import-Csv $csv)
if ($rows.Count -eq 0) {
    Write-Warning "The frame log is empty."
    exit 0
}

$micros = @($rows | ForEach-Object {
    [double]::Parse($_.micros, [System.Globalization.CultureInfo]::InvariantCulture)
} | Sort-Object)
$over16 = @($micros | Where-Object { $_ -gt 16000 }).Count
$over33 = @($micros | Where-Object { $_ -gt 33000 }).Count
$summary = @(
    "frames=$($micros.Count)",
    "p50_ms=$([math]::Round((Quantile $micros 0.50) / 1000, 3))",
    "p95_ms=$([math]::Round((Quantile $micros 0.95) / 1000, 3))",
    "p99_ms=$([math]::Round((Quantile $micros 0.99) / 1000, 3))",
    "max_ms=$([math]::Round($micros[-1] / 1000, 3))",
    "frames_over_16ms=$over16",
    "frames_over_33ms=$over33"
)
$summary | Set-Content -Path (Join-Path $runDir 'summary.txt') -Encoding utf8
$summary | Write-Host
Write-Host "Frame-log session archived at $runDir"
