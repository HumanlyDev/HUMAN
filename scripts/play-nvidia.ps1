# Launches a user-supplied title executable with HUMAN's tuned NVIDIA / 8 GB+ GPU switches.
#
# Usage:
#   .\scripts\play-nvidia.ps1 [-Exe <victory.exe>] [-Title <title.exe>]
#                             [-Console series-s|series-x] [-CardMb 512]
#                             [-DropMips 1] [-Ambient 0.6] [-ComputeLoopLimit 192]
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\victory.exe",
    [string]$Title = '',
    [ValidateSet('series-s', 'series-x')]
    [string]$Console = 'series-s',
    [int]$CardMb = 512,
    [int]$DropMips = 1,
    [string]$Ambient = '0.6',
    [int]$ComputeLoopLimit = 192
)

if (-not (Test-Path $Exe)) {
    Write-Error "$Exe not found. Build first: cargo build --release -p victory"
    exit 1
}

if (-not $Title) {
    $config = Join-Path $PSScriptRoot '..\victory.toml'
    if (-not (Test-Path $config)) {
        Write-Error "No -Title specified and victory.toml not found. Copy victory.example.toml to victory.toml."
        exit 1
    }
    $dir = (Select-String -Path $config -Pattern "^\s*game_dir\s*=\s*'([^']+)'" | Select-Object -First 1).Matches.Groups[1].Value
    if (-not $dir -or -not (Test-Path $dir)) {
        Write-Error "Valid game_dir not found in victory.toml."
        exit 1
    }
    $Title = Get-ChildItem $dir -Filter '*.exe' | Where-Object Name -notmatch '^(victory)' | Select-Object -First 1 -ExpandProperty FullName
}

if (-not $Title -or -not (Test-Path $Title)) {
    Write-Error "Title executable not found. Pass -Title <path-to-title.exe>."
    exit 1
}

$env:XCL_ADAPTER = 'nvidia'
$env:VICTORY_AMBIENT = $Ambient
$env:VICTORY_TEXTURE_DROP_MIPS = "$DropMips"
$env:VICTORY_LEVELS_ON_DEMAND = '1'
$env:VICTORY_PLACED_TEX = '1'
$env:VICTORY_WIDE_TEXTURES = '1'
$env:VICTORY_WIDE_BACK = '1'
$env:VICTORY_TEXTURE_NO_WAIT = '1'
$env:VICTORY_TEXTURE_STAGE_MS = '120'
$env:VICTORY_CARD_BUFFERS_MIN_MB = '1'
$env:VICTORY_CARD_BUFFERS_MB = "$CardMb"
$env:VICTORY_CARD_ROOM_FOR_BUFFERS = '1'
$env:VICTORY_INPLACE = '1'
$env:VICTORY_COPY_PLAIN = '1'
$env:VICTORY_RELAY = '2'
$env:VICTORY_ASYNC_SUBMIT = '1'
$env:VICTORY_QUEUE2 = '1'
$env:VICTORY_INDIRECT_WINDOW = '1'
$env:VICTORY_REFRESH_ONCE = '1'
$env:VICTORY_VIEW_KNOWN = '1'
$env:VICTORY_SHADER_LOOP_LIMIT_CS = "$ComputeLoopLimit"

& $Exe run $Title --execute --console $Console
