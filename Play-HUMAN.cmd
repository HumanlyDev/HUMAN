@echo off
setlocal EnableExtensions

cd /d "%~dp0"

set "EXE=%~1"
if "%EXE%"=="" if defined VICTORY_GAME_EXE set "EXE=%VICTORY_GAME_EXE%"

if "%EXE%"=="" if defined VICTORY_GAME_DIR (
    for %%F in ("%VICTORY_GAME_DIR%\*.exe") do (
        if /I not "%%~nxF"=="victory.exe" if /I not "%%~nxF"=="human.exe" (
            if not defined EXE set "EXE=%%~fF"
        )
    )
)

if "%EXE%"=="" (
    echo Usage: %~nx0 ^<path-to-game.exe^>
    echo Or set VICTORY_GAME_DIR / VICTORY_GAME_EXE before running.
    exit /b 1
)

for %%I in ("%EXE%") do set "GAME_DIR=%%~dpI"
if "%GAME_DIR:~-1%"=="\" set "GAME_DIR=%GAME_DIR:~0,-1%"

if not defined XCL_ADAPTER set "XCL_ADAPTER=nvidia"
if not defined XCL_DXC_DIR if exist "C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\dxcompiler.dll" (
    set "XCL_DXC_DIR=C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64"
)
if not defined VICTORY_CACHE_DIR set "VICTORY_CACHE_DIR=%LOCALAPPDATA%\HUMAN\cache-pareto"

rem Memory and streaming defaults for <= 32 GB RAM / 8 GB VRAM systems
if not defined VICTORY_INPLACE set "VICTORY_INPLACE=1"
if not defined VICTORY_TEXTURE_DROP_MIPS set "VICTORY_TEXTURE_DROP_MIPS=1"
if not defined VICTORY_EARLY_TEXTURES_MS set "VICTORY_EARLY_TEXTURES_MS=32"
if not defined VICTORY_CARD_BUFFERS_MIN_MB set "VICTORY_CARD_BUFFERS_MIN_MB=1"
if not defined VICTORY_CARD_BUFFERS_MB set "VICTORY_CARD_BUFFERS_MB=512"
if not defined VICTORY_LEVELS_ON_DEMAND set "VICTORY_LEVELS_ON_DEMAND=1"
if not defined VICTORY_AMBIENT set "VICTORY_AMBIENT=0.8"

"%~dp0target\release\victory.exe" run --console series-s "%EXE%" -- --root="%GAME_DIR%"
