Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$stamp = Get-Date -Format 'yyyy-MM-dd_HHmmss'
$runDir = Join-Path $root "target/benchmark-runs/$stamp"
New-Item -ItemType Directory -Path $runDir -Force | Out-Null

$benchmarks = @(
    'benchmarks::library_search_scale',
    'benchmarks::tag_scan',
    'benchmarks::library_save_scale',
    'benchmarks::media_read_scale'
)

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$dirty = @(& git -C $root status --porcelain).Count
$environment = @(
    "run_id=$stamp",
    "started=$(Get-Date -Format o)",
    "cwd=$root",
    "rustc=$(& rustc --version)",
    "cargo=$(& cargo --version)",
    "git_commit=$(& git -C $root rev-parse HEAD)",
    "dirty_files=$dirty",
    "cpu=$($cpu.Name.Trim())",
    "physical_cores=$($cpu.NumberOfCores)",
    "logical_processors=$($cpu.NumberOfLogicalProcessors)",
    "ram_gib=$([math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 2))",
    "os=$((Get-CimInstance Win32_OperatingSystem).Caption)",
    "profile=release",
    "threads=1",
    "ignored_tests=true"
)
$environment | Set-Content -Path (Join-Path $runDir 'environment.txt') -Encoding utf8

foreach ($name in $benchmarks) {
    $short = $name.Split('::')[-1]
    $out = Join-Path $runDir "$short.out.log"
    $err = Join-Path $runDir "$short.err.log"
    Write-Host "Running $name"
    $arguments = @(
        'test', '--release', '--locked', '--',
        '--ignored', '--nocapture', '--test-threads=1', $name
    )
    $process = Start-Process -FilePath 'cargo' -ArgumentList $arguments `
        -WorkingDirectory $root -RedirectStandardOutput $out -RedirectStandardError $err `
        -Wait -PassThru
    Get-Content $out | Write-Host
    Get-Content $err | Write-Host
    if ($process.ExitCode -ne 0) {
        throw "Benchmark $name failed with exit code $($process.ExitCode)"
    }
}

Copy-Item (Join-Path $root 'target/benchmark-*.csv') -Destination $runDir -Force
Write-Host "Benchmark run archived at $runDir"
