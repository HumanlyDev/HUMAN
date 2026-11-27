# Launches a user-supplied title executable with HUMAN's CGBJ Pareto residency
# and tuned NVIDIA / 8 GB+ GPU switches (with optional Ray-Traced GI mode).
#
# Usage:
#   .\scripts\play-nvidia.ps1 [-Exe <victory.exe>] [-Title <title.exe>]
#                             [-Console series-s|series-x] [-Ambient 0.8]
#                             [-RayTracing]
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\victory.exe",
    [string]$Title = '',
    [ValidateSet('series-s', 'series-x')]
    [string]$Console = 'series-s',
    [string]$Ambient = '0.8',
    [switch]$RayTracing,
    [int]$RtLoopLimit = 96
)

if (-not (Test-Path $Exe)) {
    Write-Error "$Exe not found. Build first: cargo build --release --locked -p victory"
    exit 1
}

if (-not $Title) {
    $config = Join-Path $PSScriptRoot '..\victory.toml'
    if (Test-Path $config) {
        $dir = (Select-String -Path $config -Pattern "^\s*game_dir\s*=\s*['""]([^'""]+)['""]" | Select-Object -First 1).Matches.Groups[1].Value
        if ($dir -and (Test-Path $dir)) {
            $Title = Get-ChildItem $dir -Filter '*.exe' | Where-Object Name -notmatch '^(victory)' | Select-Object -First 1 -ExpandProperty FullName
        }
    }
    if (-not $Title -and $env:VICTORY_GAME_DIR -and (Test-Path $env:VICTORY_GAME_DIR)) {
        $Title = Get-ChildItem $env:VICTORY_GAME_DIR -Filter '*.exe' | Where-Object Name -notmatch '^(victory)' | Select-Object -First 1 -ExpandProperty FullName
    }
}

if (-not $Title -or -not (Test-Path $Title)) {
    Write-Error "Title executable not found. Pass -Title <path-to-title.exe> or configure game_dir in victory.toml."
    exit 1
}

$env:XCL_ADAPTER = 'nvidia'
$env:VICTORY_THIN = '1'
$env:VICTORY_JOIN_PARTS = '8'
$env:VICTORY_UPLOAD_AHEAD = '1'
$env:VICTORY_RELAY = '2'
$env:VICTORY_WIDE_BACK = '1'
$env:VICTORY_CARD_REHASH_RUNS = '1'
$env:VICTORY_JOIN_COPIES = '1'
$env:VICTORY_RELAY_COPY = '1'
$env:VICTORY_CARD_STEPS = '2'
$env:VICTORY_INDIRECT_WINDOW = '1'
$env:VICTORY_INDIRECT_BATCH = '2'
$env:VICTORY_MAP_THREADS = '1'
$env:VICTORY_CARD_AWAKE = '1'
$env:VICTORY_SHOW_AFTER = '1'
$env:VICTORY_TEXTURE_NO_WAIT = '1'
$env:VICTORY_CARD_BUFFERS_MIN_KB = '64'
$env:VICTORY_LEVELS_ON_DEMAND = '1'
$env:VICTORY_AMBIENT_AIR = '1.5'
$env:VICTORY_AMBIENT = $Ambient
$env:VICTORY_NAN_SCRUB = '1'

if ($RayTracing) {
    $env:VICTORY_RT_GI = '1'
    $env:VICTORY_BVH_BUILDER = '1'
    $env:VICTORY_BVH_FENCES = '0'
    $env:VICTORY_FAST_BVH = '1'
    $env:VICTORY_RT_LOOP_LIMIT = "$RtLoopLimit"
    Remove-Item Env:VICTORY_BVH_MISS -ErrorAction SilentlyContinue
} else {
    $env:VICTORY_BVH_MISS = '1'
}

& $Exe run $Title --execute --console $Console
exit $LASTEXITCODE
