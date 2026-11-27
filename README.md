# HUMAN

An experimental Xbox Series X|S to Windows x86-64 / Direct3D 12 compatibility layer written in Rust, synced with CGBJ (`codec-d/cool_fox`) Pareto residency and submission optimizations and tuned for Ray-Traced Global Illumination on NVIDIA and AMD GPUs.

**HUMAN** loads a user-supplied Xbox Series title executable directly in a Windows x86-64 process, provides user-mode runtime services, translates console GPU command lists across parallel worker threads to host Direct3D 12, detiles console textures, and recompiles RDNA2 console GPU shaders to validated DirectX Shader Model 6.6 DXIL.

> **Important Notice (No Game Files Included):**
> This repository contains **only original compatibility-layer source code and documentation**. No game executables, archives, shaders, textures, audio, keys, or proprietary assets are included or distributed here. You must supply your own legally obtained files stored **outside** the repository (enforced by `.gitignore` and `cargo xtask policy`). This project is not affiliated with or endorsed by Microsoft or any game developer or publisher.

> **Optimization Pull Requests Wanted:**
> We are actively looking for **Pull Requests focused on CPU, GPU, memory, and shader optimizations**! See [Contributing & Optimization Targets](#contributing--optimization-targets) below.

---

## Current Compatibility & Performance Status

| Milestone | Status |
|---|---|
| **Executable & Boot** | ✅ — GTA VI executable loads and runs, imports resolve, and boots to its first frame, intro logos, and loading screen |
| **Main Menu** | ✅ — GTA VI reaches the main menu and title screen with working controller and keyboard input |
| **Open World & Playability** | ✅ — GTA VI loads into story mode and the open world with playable gameplay, character movement, driving, controller input, and pause menu |
| **Performance (RTX 4070 Ti 12 GB)** | **Stable 10 FPS** in GTA VI's open world on an NVIDIA RTX 4070 Ti (12 GB) using `--console series-x` (higher on upper-tier RTX and Radeon GPUs) |
| **Story Progress** | **Half the story (~50% of the GTA VI main story)** reachable through the prologue, early missions, and first act |
| **Rendering & Lighting** | **Close to the console (lit)** — GTA VI renders lit with textures, shadows, and console Ray-Traced Global Illumination (`ScreenProbes`, `IrradianceCache`, `PrimaryProbes`, `SkyVisibility`, `RaytracingResolve`) via Wave32-compatible `XboxBvhBuilder` and accelerated inline DXIL ray-node traversal; directional screen-space SH radiance fallback is also supported |
| **Stability** | **Fully stable, no crashes** — GTA VI runs stably across sessions with no crashes via Pareto VRAM/commit residency budgets, upload-before-publish bindless barriers, cold BVH arena page recycling, and CPU texture read-back for water/physics |

---

## Key Features & Enhancements

- **CGBJ / `cool_fox` Multi-Threaded Submission & Pareto Residency:**
  - Thin command recording & 8-way parallel translation (`VICTORY_THIN=1`, `VICTORY_JOIN_PARTS=8`).
  - Dedicated copy-queue uploads with upload-before-publish visibility (`VICTORY_UPLOAD_AHEAD=1`, `VICTORY_SHOW_AFTER=1`), async copy-back relay (`VICTORY_RELAY=2`, `VICTORY_WIDE_BACK=1`, `VICTORY_RELAY_COPY=1`), and GPU-side card steps (`VICTORY_CARD_STEPS=2`).
  - 32 GB RAM & 8 GB VRAM residency budgets (`pool.rs`, `residency.rs`, `VICTORY_CARD_BUFFERS_MIN_KB=64`, `VICTORY_LEVELS_ON_DEMAND=1`), plus CPU texture read-back (`VICTORY_TEXTURE_BACK`) and two-plane compose fixes.
- **Optimized Ray-Traced Global Illumination (`VICTORY_RT_GI=1`):**
  - Automatic Wave32 lowering for `XboxBvhBuilder` (`FIRSTLANE_OWN` on spilled builder scans, safe `WAVE_WIDE` widening for single-wave builder and GI trace/resolve kernels) so NVIDIA Wave32 GPUs (including RTX 4060) run full dynamic + static BVH global illumination.
  - **GI-only ray filter:** keeps diffuse GI probes and irradiance shadow visibility enabled while skipping expensive full-res reflection and direct-shadow ray passes (`VICTORY_RT_ALL=1`, `VICTORY_RT_REFLECTIONS=1`, or `VICTORY_RT_SHADOWS=1` re-enable them).
  - **64 KiB BVH page-straddle fast path:** skips the second `SLOT_MEMORY` lookup and heap handle creation on 99.9% of 128-byte `BOX32` nodes.
  - **Hardware `FMad` BVH slab tests (`VICTORY_FAST_BVH=1`)** and **bounded ray traversal (`VICTORY_RT_LOOP_LIMIT=96`)** + fence-free builder atomics (`VICTORY_BVH_FENCES=0`).
  - **VRAM-scaled BVH arena budget & cold-page eviction:** caps `memory.rs` arenas at 1,280 MiB on 8–10 GB GPUs (2 GiB on 12–16 GB, 3 GiB on 24 GB) and recycles cold unowned pages instead of stalling.
- **DirectStorage & BCPack (`victory-hle`):**
  - Parallel DirectStorage / zlib / BCPack decompression, plus optional DX12 Async Compute BCPack offload (`--features gpu-bcpack` + `VICTORY_GPU_BCPACK=1` via `bcpack-gpu-sdk v0.13.0`).
- **Directional Screen-Space Indirect Fallback (`ambient.rs`):**
  - When RT GI is off, estimates directional first-order SH indirect bounce from `CS_CacheScreenSpaceRadiance` (`VICTORY_AMBIENT=0.8`) and clamps indoor fog scattering (`VICTORY_AMBIENT_AIR=1.5`).

---

## Workspace Architecture

| Crate | Purpose |
|---|---|
| `apps/victory` | CLI entry point (`run`, `translate`, `shaders`, `warm`, `inspect`, `census`, `replay`, `config`) |
| `crates/victory-core` | Configuration (`victory.toml`), paths, and repo-safety checks |
| `crates/victory-pe` | PE parsing and import/export inspection |
| `crates/victory-loader` | Binary mapping, relocations, and runtime patching |
| `crates/victory-hle` | Console OS, filesystem, DirectStorage, threading, audio, and input HLE |
| `crates/victory-gpu` | Direct3D 12 translation, memory shadows, textures, BVH arenas, and indirect execution |
| `crates/victory-shader` | Console GPU ISA decoder, IR lowering, Wave32/Wave64 passes, and DXIL emitter |
| `crates/victory-formats` | Console texture swizzle/detile layouts, BCPack, and XMA/Opus audio formats |
| `crates/victory-pipeline` | Pipeline state tracking and content-addressed caching |
| `crates/victory-game` | Title-agnostic build identity matching and container helpers |
| `xtask` | Asset-policy enforcement (`cargo xtask policy`) and git hook installer |

---

## Requirements & Building

- **OS:** Windows 10 / 11 x64
- **CPU:** x86-64 with AVX2
- **GPU:** Direct3D 12 with Shader Model 6.6 & Mesh Shaders (8 GB+ VRAM; tuned for NVIDIA RTX 4060 and up + AMD RDNA2/3)
- **SDK:** Visual Studio Build Tools (C++ x64) + Windows 10/11 SDK (`dxcompiler.dll` / `dxil.dll`)
- **Toolchain:** Rust pinned in [rust-toolchain.toml](rust-toolchain.toml)

```powershell
# 1. Install the asset-policy pre-commit hook
cargo xtask install-hooks

# 2. Create your local git-ignored config and point game_dir outside the repo
copy victory.example.toml victory.toml

# 3. Run unit tests & asset-policy verification (no external files required)
cargo test --workspace --locked
cargo run -p xtask -- policy

# 4. Build the release binary (add --features gpu-bcpack for optional GPU BCPack offload)
cargo build --release --locked -p victory
```

---

## Usage

### Quick Launch (NVIDIA / RTX 4060+ with Ray-Traced GI)

Using `Play-HUMAN.cmd` (or `Play-VIctory.cmd` for CGBJ defaults):

```powershell
$env:VICTORY_GAME_DIR = "D:\Games\YourDump"

# Tuned Pareto residency + Ray-Traced Global Illumination (stable 10 FPS on RTX 4060)
.\Play-HUMAN.cmd rt-gi

# Fast Pareto residency with screen-space SH ambient fallback
.\Play-HUMAN.cmd pareto-residency
```

Or using `scripts\play-nvidia.ps1`:

```powershell
.\scripts\play-nvidia.ps1 -Title "<path-to-title.exe>" -Console series-x -RayTracing
```

Or invoke `victory.exe` directly:

```powershell
.\target\release\victory.exe run "<path-to-title.exe>" --console series-x --execute
```

### Pre-Warm Shaders & Pipelines

```powershell
.\target\release\victory.exe shaders "<path-to-shader-archives>"
.\target\release\victory.exe warm --used
```

### Keyboard Stand-In Controls

When no XInput controller is connected, the keyboard maps to a standard pad while the window has focus:

| Keys | Pad Input |
|---|---|
| `W A S D` / `I J K L` | Left / Right Stick |
| Arrow keys | D-Pad |
| `Enter`, `Backspace`, `X`, `Y` | A, B, X, Y |
| `Q` / `E`, `Z` / `C` | Shoulders (LB/RB), Triggers (LT/RT) |
| `F` / `H` | Stick clicks (L3 / R3) |
| `Esc`, `Tab` | Menu, View |
| `F11` or `Alt+Enter` | Toggle windowed / borderless |

---

## Key Environment Switches

| Variable | Recommended | Effect |
|---|---|---|
| `XCL_ADAPTER` | `nvidia` | Selects the discrete GPU on hybrid systems |
| `VICTORY_RT_GI` | `1` | Enables BVH builder + diffuse GI ray tracing while skipping full-res reflection/shadow ray passes |
| `VICTORY_BVH_BUILDER` | `1` | Runs the console's GPU BVH builder (automatically enabled by `VICTORY_RT_GI=1`) |
| `VICTORY_BVH_FENCES` | `0` | Drops redundant per-atomic UAV fences in builder stages (defaults to `0` under `VICTORY_RT_GI=1`) |
| `VICTORY_FAST_BVH` | `1` | Uses hardware `FMad` + precomputed `-origin * inverse` in BVH box slab tests |
| `VICTORY_RT_LOOP_LIMIT` | `96` | Caps divergent BVH traversal loop iterations per ray |
| `VICTORY_MEMORY_CARD_MB` | auto | Overrides BVH title-memory arena cap (`1280` MiB auto on 8–10 GB GPUs) |
| `VICTORY_THIN` / `VICTORY_JOIN_PARTS` | `1` / `8` | Records commands thinly and translates across 8 worker threads |
| `VICTORY_RELAY` / `VICTORY_WIDE_BACK` | `2` / `1` | Async GPU readback relay with wide AVX2 diff/merge |
| `VICTORY_UPLOAD_AHEAD` / `VICTORY_SHOW_AFTER` | `1` / `1` | Dedicated copy-queue texture uploads published only after completion |
| `VICTORY_CARD_BUFFERS_MIN_KB` | `64` | Promotes buffers `>= 64 KiB` to GPU-resident shadow copies |
| `VICTORY_AMBIENT` / `VICTORY_AMBIENT_AIR` | `0.8` / `1.5` | Directional screen-space SH indirect fallback + indoor fog clamp when RT is off |

---

## Contributing & Optimization Targets

**Pull requests for performance, RT throughput, and memory optimizations are very welcome.**

1. **RT Global Illumination & BVH Traversal (`crates/victory-shader/src/emit/intersect.rs`, `crates/victory-gpu/src/memory.rs`, `pipeline/rt_gi.rs`)**
2. **Submission Prep & Indirect Execution (`crates/victory-gpu/src/draw.rs`, `indirect.rs`, `host.rs`)**
3. **8 GB VRAM Residency & Texture Streaming (`crates/victory-gpu/src/texture.rs`, `pool.rs`, `residency.rs`)**
4. **Wave32 / DXIL Shader Recompiler Passes (`crates/victory-shader`)**

Before opening a PR, please verify:
- `cargo clippy --workspace --all-targets --locked -- -D warnings` passes.
- `cargo test --workspace --locked` passes.
- `cargo run -p xtask -- policy` passes with zero violations.

---

## Credits & License

Built on upstream open-source work:
- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and contributors (`GPL-3.0-or-later`)
- **[codec-d/cool_fox](https://github.com/codec-d/cool_fox)** (CGBJ) Pareto residency, multi-threaded submission, and BVH ray-tracing foundation
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)**, **[DexNom/VIctory](https://github.com/DexNom/VIctory)**, **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**, **[myokex/victory-amd](https://github.com/myokex/victory-amd)**, and **[XCL](https://github.com/ivall/xcl)**
- Community contributors (`POMXARK`, `bdycounter-sudo`, `Geckomacabre`, `romainhedouin`, and fork contributors)

Licensed under **GPL-3.0-or-later**. See [LICENSE](LICENSE) and [docs/asset-policy.md](docs/asset-policy.md).
