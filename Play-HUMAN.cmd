@echo off
setlocal EnableExtensions DisableDelayedExpansion
set "mode=%~1"
if "%mode%"=="" set "mode=pareto-residency"
if /i not "%mode%"=="pareto-residency" if /i not "%mode%"=="rt-gi" (
  echo Usage: Play-HUMAN.cmd [pareto-residency^|rt-gi] 1>&2
  exit /b 2
)
if not exist "%VICTORY_GAME_DIR%\game_scarlett_master.exe" (
  echo Set VICTORY_GAME_DIR to your own dump folder containing game_scarlett_master.exe. 1>&2
  exit /b 2
)
set "runner=%~dp0target\release\victory.exe"
if not exist "%runner%" set "runner=%~dp0victory-3d-rtx-pareto-residency.exe"
if not exist "%runner%" (
  echo Run cargo build --release --locked -p victory or place victory.exe next to this launcher. 1>&2
  exit /b 2
)
if not defined XCL_ADAPTER set "XCL_ADAPTER=nvidia"
if not defined VICTORY_AMBIENT_AIR set "VICTORY_AMBIENT_AIR=1.5"
if not defined VICTORY_CACHE_DIR set "VICTORY_CACHE_DIR=%LOCALAPPDATA%\HUMAN\cache-pareto"
if not defined VICTORY_TEXTURE_NO_WAIT set "VICTORY_TEXTURE_NO_WAIT=1"
if not defined VICTORY_RELAY set "VICTORY_RELAY=2"
if not defined VICTORY_WIDE_BACK set "VICTORY_WIDE_BACK=1"
if not defined VICTORY_CARD_REHASH_RUNS set "VICTORY_CARD_REHASH_RUNS=1"
if not defined VICTORY_JOIN_COPIES set "VICTORY_JOIN_COPIES=1"
if not defined VICTORY_INDIRECT_WINDOW set "VICTORY_INDIRECT_WINDOW=1"
if not defined VICTORY_RELAY_COPY set "VICTORY_RELAY_COPY=1"
if not defined VICTORY_CARD_STEPS set "VICTORY_CARD_STEPS=2"
if not defined VICTORY_INDIRECT_BATCH set "VICTORY_INDIRECT_BATCH=2"
if not defined VICTORY_MAP_THREADS set "VICTORY_MAP_THREADS=1"
if not defined VICTORY_CARD_AWAKE set "VICTORY_CARD_AWAKE=1"
if not defined VICTORY_SHOW_AFTER set "VICTORY_SHOW_AFTER=1"
if not defined VICTORY_AMBIENT set "VICTORY_AMBIENT=0.8"
if not defined VICTORY_THIN set "VICTORY_THIN=1"
if not defined VICTORY_JOIN_PARTS set "VICTORY_JOIN_PARTS=8"
if not defined VICTORY_UPLOAD_AHEAD set "VICTORY_UPLOAD_AHEAD=1"
if not defined VICTORY_LEVELS_ON_DEMAND set "VICTORY_LEVELS_ON_DEMAND=1"
if not defined VICTORY_CARD_BUFFERS_MIN_KB set "VICTORY_CARD_BUFFERS_MIN_KB=64"
if not defined VICTORY_LOG if not defined RUST_LOG set "VICTORY_LOG=warn"
if /i "%mode%"=="rt-gi" (
  if not defined VICTORY_RT_GI set "VICTORY_RT_GI=1"
  if not defined VICTORY_BVH_BUILDER set "VICTORY_BVH_BUILDER=1"
  if not defined VICTORY_BVH_FENCES set "VICTORY_BVH_FENCES=0"
  if not defined VICTORY_FAST_BVH set "VICTORY_FAST_BVH=1"
  if not defined VICTORY_RT_LOOP_LIMIT set "VICTORY_RT_LOOP_LIMIT=96"
)
"%runner%" run "%VICTORY_GAME_DIR%\game_scarlett_master.exe" --console series-s --execute
exit /b %errorlevel%
