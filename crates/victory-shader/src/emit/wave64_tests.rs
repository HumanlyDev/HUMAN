//! A console wave64 compute program on a host of 32-lane waves: a wave reduction inside a
//! per-lane branch, with that branch taken wave-wide ([`crate::structure::structure_wide`])
//! so that the two host waves can exchange their halves inside it. Lowered, validated by
//! `dxil.dll` when present, and run on the host GPU when it takes waves of 32.

use std::collections::HashSet;

use super::host::{Dispatch, Host, Root, View};
use super::{HostShader, lower, validator};
use crate::native::{
    ComputeIo, HwStage, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource,
};
use crate::{ShaderError, io, ir, resource, structure};

const S_ENDPGM: u32 = 0xBF81_0000;

/// Synthetic tree from `valu_tests::whole_wave_f64_sum_and_cvt_f32_f64` in
/// fxceless/victory-fork at 9376fd6, without that test's resource setup or store.
fn f64_wave32_fixture() -> Vec<u32> {
    let mov = |dst: u8, src: u16| 0x7E00_0200 | (dst as u32) << 17 | src as u32;
    let v3 = |op: u32, dst: u8, a: u16, b: u16, c: u16| {
        [
            0xD400_0000 | op << 16 | dst as u32,
            a as u32 | (b as u32) << 9 | (c as u32) << 18,
        ]
    };
    let mut w = vec![mov(6, 0), mov(7, 1), 0xBE8C_3DC1];
    w.extend(v3(0x101, 2, 128, 262, 12));
    w.extend(v3(0x101, 3, 128, 263, 12));
    for sel in [14570689u32, 9658386, 2340] {
        w.extend([mov(4, 0xE9), 2 | sel << 8, mov(5, 0xE9), 3 | sel << 8]);
        w.extend(v3(0x164, 2, 260, 258, 0));
    }
    w.extend([mov(4, 0xFA), 2 | 0xFF09_4000, mov(5, 0xFA), 3 | 0xFF09_4000]);
    w.extend(v3(0x164, 2, 260, 258, 0));
    w.extend(v3(0x378, 4, 258, 128, 128));
    w.extend(v3(0x378, 5, 259, 128, 128));
    w.extend(v3(0x164, 2, 260, 258, 0));
    w.extend(v3(0x360, 14, 258, 128, 0));
    w.extend(v3(0x360, 15, 259, 128, 0));
    w.extend([0xBE90_030E, 0xBE91_030F, 0xBEFE_030C]);
    w.push(0x7E00_1E10); // v_cvt_f32_f64 v0, s[16:17]
    w
}

/// FP64 tree with distinct lower/upper-half inputs. With partial EXEC, only
/// lanes 0..15 and 32..47 contribute; the other lanes contain poisoned doubles.
/// Multiple console waves use the same inputs: this does not test pair isolation.
fn fp64_program(threads: u32, partial: bool) -> NativeProgram {
    let v3 = |op: u32, dst: u8, a: u16, b: u16, c: u16| {
        [
            0xD400_0000 | op << 16 | dst as u32,
            a as u32 | (b as u32) << 9 | (c as u32) << 18,
        ]
    };
    let mut w = vec![
        0x2C0A_0085, // v_lshrrev_b32 v5, 5, v0
        0x360A_0A81, // v_and_b32 v5, 1, v5: half within each console wave
        0x7D88_0A81, // v_cmp_gt_u32 vcc, 1, v5
    ];
    w.extend(v3(0x101, 6, 2, 0, 106)); // lo: upper half s2, lower s0
    w.extend(v3(0x101, 7, 3, 1, 106)); // hi: upper half s3, lower s1
    if partial {
        w.extend([0x7E0C_0200, 0x7E0E_0201]); // valid terms in both halves = s0:s1
        w.extend([0x360A_0090, 0x7D88_0A81]); // v5 = t & 16; vcc = 1 > v5
        // Poison noncontributing lanes from s2:s3, keeping valid inputs elsewhere.
        w.extend(v3(0x101, 6, 2, 262, 106));
        w.extend(v3(0x101, 7, 3, 263, 106));
        w.push(0xBEFE_046A); // s_mov_b64 exec, vcc
    }
    let f = f64_wave32_fixture();
    // Skip the fixture's uniform input setup and replace wave32 save with wave64.
    w.push(0xBE8C_25C1);
    w.extend_from_slice(&f[3..f.len() - 4]);
    // Fixture already read lanes 0 into s14:s15. Join the other console half.
    w.extend(v3(0x360, 18, 258, 160, 0)); // lane 32 low
    w.extend(v3(0x360, 19, 259, 160, 0)); // lane 32 high
    w.extend(v3(0x164, 2, 18, 258, 0));
    w.extend(v3(0x360, 14, 258, 128, 0));
    w.extend(v3(0x360, 15, 259, 128, 0));
    w.extend([0xBE90_030E, 0xBE91_030F, 0xBEFE_040C]);
    if partial {
        w.push(0xBEFE_04C1); // all threads inspect the masked reduction
    }
    w.extend([
        0x7E02_1E10, // v_cvt_f32_f64 v1, s[16:17]; preserve thread index v0
        0x340E_0082, // v_lshlrev_b32 v7, 2, v0: byte offset = 4 * thread
        0xE070_1000,
        0x8002_0107, // buffer_store_dword v1, v7, s[8:11], 0 offen
    ]);
    native_of(&w, true, threads)
}

#[test]
fn fp64_fixture_stores_each_thread_at_its_own_byte_offset() {
    use crate::gfx10::{Fields, Operand};
    for partial in [false, true] {
        let n = fp64_program(128, partial);
        let p = ir::build(&n.code, true, n.vgprs).expect("FP64 fixture IR");
        let insts: Vec<_> = p.blocks.iter().flat_map(|b| &b.insts).collect();
        let store = insts
            .iter()
            .position(|i| i.name() == "buffer_store_dword")
            .unwrap();
        let Fields::Buffer {
            data,
            addr,
            rsrc,
            soffset,
            offset,
            offen,
            idxen,
            ..
        } = &insts[store].fields
        else {
            panic!("expected MUBUF store");
        };
        assert_eq!(*data, Some(Operand::Vector { reg: 1, dwords: 1 }));
        assert_eq!(*addr, Operand::Vector { reg: 7, dwords: 1 });
        assert_eq!(*rsrc, Operand::Scalar { code: 8, dwords: 4 });
        assert_eq!(*soffset, Operand::Const(128));
        assert_eq!(*offset, 0);
        assert!(*offen);
        assert!(!*idxen);
        assert_eq!(insts[store - 1].name(), "v_lshlrev_b32");
        assert_eq!(insts[store - 1].machine.words, [0x340E_0082]);
        // Address must consume the immediately preceding shift, not a stale v7.
        assert_eq!(
            insts[store].read(ir::Reg::V(7)),
            insts[store - 1].write(ir::Reg::V(7))
        );
    }
}

#[test]
fn fp64_wave64_exchange_and_masked_poison_validate() {
    for (threads, partial) in [(64, false), (128, false), (64, true), (128, true)] {
        let n = fp64_program(threads, partial);
        let p = ir::build(&n.code, true, n.vgprs).expect("FP64 IR");
        let shader = lowered(&n, &p, &HashSet::new()).expect("FP64 lower");
        match validator::Validator::load(validator::Validator::default_path().as_deref()) {
            Ok(mut v) => {
                v.validate(&mut shader.dxil.clone())
                    .expect("FP64 DXIL validation");
            }
            Err(e) => {
                eprintln!("SKIP FP64 validation threads={threads} partial={partial}: {e}");
            }
        }
    }
}

/// Select hardware for this test only: the default adapter may be an iGPU without FP64.
fn fp64_host() -> Option<Host> {
    use windows::Win32::Graphics::{
        Direct3D::D3D_FEATURE_LEVEL_12_0,
        Direct3D12::*,
        Dxgi::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_NOT_FOUND, IDXGIFactory4},
    };
    // SAFETY: DXGI/D3D12 out pointers and feature structures have their matching types
    // and sizes; the selected device remains owned by Host for all later dispatches.
    unsafe {
        let factory: IDXGIFactory4 = match CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) {
            Ok(factory) => factory,
            Err(e) => {
                eprintln!("SKIP FP64 wave64: cannot enumerate hardware adapters: {e}");
                return None;
            }
        };
        let mut index = 0;
        loop {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(adapter) => adapter,
                Err(e) => {
                    if e.code() != DXGI_ERROR_NOT_FOUND {
                        eprintln!("FP64 wave64: adapter enumeration failed at {index}: {e}");
                    }
                    break;
                }
            };
            index += 1;
            let desc = match adapter.GetDesc1() {
                Ok(desc) => desc,
                Err(e) => {
                    eprintln!("FP64 wave64: cannot describe adapter {}: {e}", index - 1);
                    continue;
                }
            };
            let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..end]);
            if desc.Flags & 2 != 0 || (desc.VendorId == 0x1414 && desc.DeviceId == 0x8c) {
                eprintln!("FP64 wave64: ignoring software adapter {name}");
                continue;
            }
            let candidate = (|| -> Result<Host, String> {
                let mut device: Option<ID3D12Device> = None;
                D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_12_0, &mut device)
                    .map_err(|e| format!("device creation: {e}"))?;
                let device = device.ok_or("device creation returned no device")?;
                let mut sm = D3D12_FEATURE_DATA_SHADER_MODEL {
                    HighestShaderModel: D3D_SHADER_MODEL_6_6,
                };
                device.CheckFeatureSupport(
                    D3D12_FEATURE_SHADER_MODEL, (&raw mut sm).cast(), size_of_val(&sm) as u32,
                ).map_err(|e| format!("shader-model query: {e}"))?;
                if sm.HighestShaderModel.0 < D3D_SHADER_MODEL_6_6.0 {
                    return Err(format!("SM {:#x} is below 6.6", sm.HighestShaderModel.0));
                }
                let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS::default();
                device.CheckFeatureSupport(
                    D3D12_FEATURE_D3D12_OPTIONS, (&raw mut options).cast(), size_of_val(&options) as u32,
                ).map_err(|e| format!("FP64 capability query: {e}"))?;
                if !options.DoublePrecisionFloatShaderOps.as_bool() {
                    return Err("DoublePrecisionFloatShaderOps=false".into());
                }
                let mut waves = D3D12_FEATURE_DATA_D3D12_OPTIONS1::default();
                device.CheckFeatureSupport(
                    D3D12_FEATURE_D3D12_OPTIONS1, (&raw mut waves).cast(), size_of_val(&waves) as u32,
                ).map_err(|e| format!("wave capability query: {e}"))?;
                if !waves.WaveOps.as_bool() || waves.WaveLaneCountMin > 32 || waves.WaveLaneCountMax < 32 {
                    return Err(format!(
                        "WaveOps={} lane range={}..{} excludes WaveSize32",
                        waves.WaveOps.as_bool(), waves.WaveLaneCountMin, waves.WaveLaneCountMax
                    ));
                }
                Ok(Host::with_device(device))
            })();
            match candidate {
                Ok(host) => {
                    eprintln!("FP64 wave64: selected hardware adapter {name} (SM6.6, FP64, WaveSize32)");
                    return Some(host);
                }
                Err(e) => eprintln!("FP64 wave64: adapter {name} unavailable for this test: {e}"),
            }
        }
    }
    eprintln!("SKIP FP64 wave64: no hardware adapter supports SM6.6, FP64 and WaveSize32");
    None
}

#[test]
fn fp64_wave64_exchange_and_masked_poison_gpu() {
    let _one = super::valu_tests::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Some(host) = fp64_host() else {
        return;
    };
    for (threads, partial) in [(64, false), (128, false), (64, true), (128, true)] {
        let n = fp64_program(threads, partial);
        let p = ir::build(&n.code, true, n.vgprs).expect("FP64 IR");
        let shader = lowered(&n, &p, &HashSet::new()).expect("FP64 lower");
        let (root, pso) = host.compute_pipeline(&shader).expect("FP64 WaveSize32 PSO");
        // In the partial case the low dword changes the final f32, so dropping it is observable.
        let av = 1.0f64 + 2.0f64.powi(-23);
        let a = av.to_bits();
        let inputs = if partial {
            vec![
                0x7ff0_0000_0000_0000u64, // +infinity
                0x7ff8_0000_1234_5678,    // positive quiet NaN
                0xfff0_0000_89ab_cdef,    // negative NaN
            ]
        } else {
            vec![3.0f64.to_bits()]
        };
        for b in inputs {
            let data = [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32];
            let mut ud = vec![0; shader.user_data_dwords().max(1) as usize];
            for u in &shader.user_data {
                ud[u.index as usize] = match (u.root, u.slot) {
                    (RootType::Constants, 0) => data[u.dword as usize],
                    (RootType::Table, 1) => 0,
                    x => panic!("unexpected root {x:?}"),
                };
            }
            let out = host
                .dispatch(
                    &root,
                    &pso,
                    &Dispatch {
                        roots: vec![(0, Root::Constants(ud))],
                        views: &[View::RawUav(threads)],
                        samplers: &[],
                        groups: [1, 1, 1],
                    },
                )
                .expect("FP64 wave64 dispatch");
            let want = (if partial { 32.0 * av } else { 32.0 * av + 96.0 }) as f32;
            for t in 0..threads {
                assert_eq!(
                    out[0][t as usize],
                    want.to_bits(),
                    "threads={threads} partial={partial} poison={b:#x} t={t}"
                );
            }
        }
        eprintln!("RAN FP64 WaveSize32 hardware threads={threads} partial={partial}");
    }
}

/// 64 threads in one console wave. Each thread t holds the bit `1 << (t >> 1)`; the threads
/// below s0 (a root constant) OR theirs over the wave inside `if (t < s0) { ... }` and store
/// the result to dword 0 of the raw UAV.
/// `skip`: the branch past the body when no thread is below s0: `s_cbranch_execz`, or
/// `s_cbranch_scc0` on the SCC `s_and_saveexec` sets ("some lane is left").
fn program(skip: u32) -> NativeProgram {
    let mut words = vec![
        0x7E0A_0281, // v_mov_b32 v5, 1
        0x2C0C_0081, // v_lshrrev_b32 v6, 1, v0
        0x340E_0B06, // v_lshlrev_b32 v7, v6, v5
        0x7D88_0000, // v_cmp_gt_u32 vcc, s0, v0
        0xBE8C_246A, // s_and_saveexec_b64 s[12:13], vcc
        skip | 23,   // the branch to the restore of EXEC
    ];
    words.extend_from_slice(&super::wave::WAVE_OR_V7_TO_S7);
    words.extend_from_slice(&[
        0x7E00_0207, // v_mov_b32 v0, s7
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
        0xBEFE_040C, // s_mov_b64 exec, s[12:13]
    ]);
    native(&words)
}

/// A wave64 compute program of 64 threads (one console wave; the thread index in v0) with 4
/// root constants in s0..s3 and a raw UAV's descriptor in s[8:11], then `body`.
fn native(body: &[u32]) -> NativeProgram {
    native_of(body, true, 64)
}

/// The same for a console of 32-lane waves when `wave64` is not set, with `threads` threads.
fn native_of(body: &[u32], wave64: bool, threads: u32) -> NativeProgram {
    let mut words = vec![
        0xBE85_0380, // s_mov_b32 s5, 0
        0x3D << 26 | 0x02 << 18 | 8 << 6 | 2,
        125 << 25, // s_load_dwordx4 s[8:11], s[4:5], 0
    ];
    words.extend_from_slice(body);
    words.push(S_ENDPGM);
    NativeProgram {
        stage: HwStage::Cs,
        wave64,
        vgprs: 16,
        layout: vec![
            RootParam {
                root: RootType::Constants,
                table_offset_dwords: 0,
                sgprs: 4,
                base_register: 0,
                user_data: 0,
                slot: 0,
            },
            RootParam {
                root: RootType::Table,
                table_offset_dwords: 0,
                sgprs: 1,
                base_register: 0,
                user_data: 4,
                slot: 1,
            },
        ],
        used: vec![UsedResource {
            kind: ResourceType::Uav,
            root: RootType::Table,
            table_offset_dwords: 0,
            lower_bound: 0,
            stride: 0,
            dwords: 4,
            user_data: 4,
            slot: 1,
        }],
        io: StageIo::Cs(ComputeIo {
            rsrc2: 5 << 1, // 5 user SGPRs: s0..s4
            threads: [threads, 1, 1],
        }),
        code: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
    }
}

/// `program()` for a host of 32-lane waves, with the branches closing `wide` taken wave-wide.
fn lowered(native: &NativeProgram, p: &ir::Program, wide: &HashSet<usize>) -> Result<HostShader, ShaderError> {
    lowered_on(native, p, wide, 32)
}

/// [`lowered`] for a host whose waves have at most `host_wave` lanes.
fn lowered_on(
    native: &NativeProgram,
    p: &ir::Program,
    wide: &HashSet<usize>,
    host_wave: u32,
) -> Result<HostShader, ShaderError> {
    let st = structure::structure_wide(p, wide).expect("structure");
    let res = resource::track(p, &native.layout, native.stage, Some(&st)).expect("resource::track");
    let io = io::raise(p, native).expect("io::raise");
    let was = lower::TEST_HOST_WAVE.replace(Some(host_wave));
    let lowered = lower::lower(&lower::Parts {
        native,
        p,
        st: &st,
        io: &io,
        res: &res,
        inputs: Vec::new(),
        outputs: Vec::new(),
        prims: Vec::new(),
        mesh_stride: 0,
    });
    lower::TEST_HOST_WAVE.set(was);
    let lowered = lowered?;
    Ok(HostShader {
        stage: HwStage::Cs,
        dxil: lowered.dxil,
        user_data: lowered.user_data,
        samplers: lowered.samplers,
        static_samplers: Vec::new(),
        root_cbvs: lowered.root_cbvs,
        root_srvs: lowered.root_srvs,
        root_uavs: lowered.root_uavs,
        root_indexed_views: res.root_indexed_views,
    })
}

#[test]
fn a_sum_of_halves_over_the_wave_is_the_consoles_tree_bit_for_bit() {
    // thread t holds the 16-bit float with bits 0x3C00 + 37 t (1.0 and up, with odd
    // mantissas, so that additions round); all of them store the wave's sum
    let mut words = vec![
        0x340C_0085, // v_lshlrev_b32 v6, 5, v0
        0x340E_0082, // v_lshlrev_b32 v7, 2, v0
        0x4A0E_0F06, // v_add_nc_u32 v7, v6, v7
        0x4A0E_0F00, // v_add_nc_u32 v7, v0, v7
        0x4A0E_0EFF,
        0x0000_3C00, // v_add_nc_u32 v7, 0x3c00, v7
    ];
    // (the wave OR's words with v_add_f16 for v_or_b32: opcode 50 for 28)
    words.extend(super::wave::WAVE_OR_V7_TO_S7.iter().map(|&w| if w >> 25 == 28 { w & 0x01FF_FFFF | 50 << 25 } else { w }));
    words.extend_from_slice(&[
        0x7E00_0207, // v_mov_b32 v0, s7
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
    ]);
    let native = native(&words);
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    assert_eq!(p.blocks.iter().flat_map(|b| &b.insts).filter(|i| i.name() == "v_add_f16").count(), 6);
    let shader = lowered(&native, &p, &HashSet::new()).expect("lower");
    if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        let mut copy = shader.dxil.clone();
        v.validate(&mut copy).expect("DXIL validation failed");
    }
    // the console's sum: lanes 2k and 2k+1, then pairs, ... each addition rounded to 16 bits
    // (to nearest, ties to even); values here are normal numbers
    let wide = |h: u16| f32::from_bits((u32::from(h >> 10) + 112) << 23 | u32::from(h & 0x3FF) << 13);
    let narrow = |f: f32| {
        let b = f.to_bits();
        let h = ((b >> 23) - 112) << 10 | (b >> 13 & 0x3FF);
        let rest = b & 0x1FFF;
        (h + u32::from(rest > 0x1000 || rest == 0x1000 && h & 1 == 1)) as u16
    };
    let add = |a: u16, b: u16| narrow(wide(a) + wide(b));
    let terms: Vec<u16> = (0..64u16).map(|t| 0x3C00 + 37 * t).collect();
    let mut tree = terms.clone();
    while tree.len() > 1 {
        tree = tree.chunks(2).map(|c| add(c[0], c[1])).collect();
    }
    let in_a_row = terms.iter().copied().reduce(add).unwrap();
    assert_ne!(tree[0], in_a_row, "these terms tell the tree from another order");
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(host) = Host::new(false) else {
        eprintln!("no SM 6.6 hardware device; not run");
        return;
    };
    let (root, pso) = match host.compute_pipeline(&shader) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("the hardware device does not take this program (waves of 32?): {e}; not run");
            return;
        }
    };
    let ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
    let out = host
        .dispatch(
            &root,
            &pso,
            &Dispatch {
                roots: vec![(0, Root::Constants(ud))],
                views: &[View::RawUav(1)],
                samplers: &[],
                groups: [1, 1, 1],
            },
        )
        .unwrap_or_else(|e| panic!("dispatch failed: {e}"));
    // (the high half is lane 0's term's: zero)
    assert_eq!(out[0][0], u32::from(tree[0]), "tree {:#x}, in a row {in_a_row:#x}", tree[0]);
    eprintln!("ran on the hardware device: the sum {:#x} of the console's tree", tree[0]);
}

/// The console's 16-bit wave sum where the lanes at or past s0 have left the program (a
/// thread past the end of the data): on the console they add +0, and after each step of the
/// tree they hold their run's partial sum all the same. On a host of 64-lane waves (AMD) the
/// lanes that are here read each partner run's sum from a lane of it that is here.
#[test]
fn a_sum_of_halves_where_lanes_have_left_is_the_consoles_tree_bit_for_bit() {
    let mut words = vec![
        0x7DA8_0000, // v_cmpx_gt_u32 exec, s0, v0: the lanes below s0 stay
        0, // s_cbranch_execz to the end (filled in below)
        0x340C_0085, // v_lshlrev_b32 v6, 5, v0
        0x340E_0082, // v_lshlrev_b32 v7, 2, v0
        0x4A0E_0F06, // v_add_nc_u32 v7, v6, v7
        0x4A0E_0F00, // v_add_nc_u32 v7, v0, v7
        0x4A0E_0EFF,
        0x0000_3C00, // v_add_nc_u32 v7, 0x3c00, v7
    ];
    words.extend(super::wave::WAVE_OR_V7_TO_S7.iter().map(|&w| if w >> 25 == 28 { w & 0x01FF_FFFF | 50 << 25 } else { w }));
    words.extend_from_slice(&[
        0x7E00_0207, // v_mov_b32 v0, s7
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
    ]);
    // past every word after it, to the program's end
    words[1] = 0xBF88_0000 | (words.len() as u32 - 2);
    let native = native(&words);
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    let shader = lowered_on(&native, &p, &HashSet::new(), 64).expect("lower");
    if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        let mut copy = shader.dxil.clone();
        v.validate(&mut copy).expect("DXIL validation failed");
    }
    let wide = |h: u16| {
        if h == 0 {
            0.0
        } else {
            f32::from_bits((u32::from(h >> 10) + 112) << 23 | u32::from(h & 0x3FF) << 13)
        }
    };
    let narrow = |f: f32| {
        if f == 0.0 {
            return 0;
        }
        let b = f.to_bits();
        let h = ((b >> 23) - 112) << 10 | (b >> 13 & 0x3FF);
        let rest = b & 0x1FFF;
        (h + u32::from(rest > 0x1000 || rest == 0x1000 && h & 1 == 1)) as u16
    };
    let add = |a: u16, b: u16| narrow(wide(a) + wide(b));
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = false;
    for stay in [64u32, 63, 40, 33, 32, 17, 5, 1] {
        let terms: Vec<u16> = (0..64u32)
            .map(|t| if t < stay { (0x3C00 + 37 * t) as u16 } else { 0 })
            .collect();
        let mut tree = terms.clone();
        while tree.len() > 1 {
            tree = tree.chunks(2).map(|c| add(c[0], c[1])).collect();
        }
        let Some(got) = on_the_card(&shader, [stay, 0]) else {
            return;
        };
        ran = true;
        // (the high half is lane 0's term's: zero)
        assert_eq!(got, u32::from(tree[0]), "{stay} lanes stay: got {got:#x}, the console's tree {:#x}", tree[0]);
    }
    if ran {
        eprintln!("ran on the hardware device: the console's tree with lanes gone, in every case");
    }
}

/// One `v_add_f16` whose exact result lies between two halves: 1 + 3/1024 (0x3C03) plus 2
/// (0x4000) is 3 + 1.5 steps of that range, which the console rounds up to 0x4202. The
/// host's `f32tof16` may give 0x4201 (toward zero); with `VICTORY_F16_NEAREST=1` the
/// translation gives the console's whatever the host does.
#[test]
fn a_16_bit_sum_rounds_to_nearest_when_asked_to() {
    let words = [
        0x7E0E_02FF,
        0x0000_3C03, // v_mov_b32 v7, 0x3c03
        0x7E0C_02FF,
        0x0000_4000, // v_mov_b32 v6, 0x4000
        0x6400_0D07, // v_add_f16 v0, v7, v6
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
    ];
    let native = native(&words);
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    assert!(p.blocks.iter().flat_map(|b| &b.insts).any(|i| i.name() == "v_add_f16"));
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for nearest in [false, true] {
        let was = lower::TEST_F16_NEAREST.replace(Some(nearest));
        let shader = lowered(&native, &p, &HashSet::new());
        lower::TEST_F16_NEAREST.set(was);
        let shader = shader.expect("lower");
        if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
            let mut copy = shader.dxil.clone();
            v.validate(&mut copy).expect("DXIL validation failed");
        }
        let Ok(host) = Host::new(false) else {
            eprintln!("no SM 6.6 hardware device; not run");
            return;
        };
        let Ok((root, pso)) = host.compute_pipeline(&shader) else {
            eprintln!("the hardware device does not take this program; not run");
            return;
        };
        let ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
        let out = host
            .dispatch(
                &root,
                &pso,
                &Dispatch {
                    roots: vec![(0, Root::Constants(ud))],
                    views: &[View::RawUav(1)],
                    samplers: &[],
                    groups: [1, 1, 1],
                },
            )
            .unwrap_or_else(|e| panic!("dispatch failed: {e}"));
        let got = out[0][0] & 0xFFFF;
        eprintln!("v_add_f16 0x3c03 + 0x4000 on the hardware device, nearest {nearest}: {got:#x}");
        if nearest {
            assert_eq!(got, 0x4202);
        } else {
            assert!(got == 0x4201 || got == 0x4202, "{got:#x}");
        }
    }
}

#[test]
fn a_wave_or_inside_a_lane_branch_needs_the_branch_wave_wide() {
    for (skip, name) in [(0xBF88_0000, "s_cbranch_execz"), (0xBF84_0000, "s_cbranch_scc0")] {
        a_wave_or_inside(skip, name);
    }
}

fn a_wave_or_inside(skip: u32, name: &str) {
    let native = program(skip);
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    let branch = (0..p.blocks.len())
        .find(|&b| p.blocks[b].insts.last().is_some_and(|i| i.name() == name))
        .expect("the branch");
    // per lane, the two host waves cannot meet inside the branch
    lower::take_needs();
    let per_lane = lowered(&native, &p, &HashSet::new());
    assert!(
        matches!(&per_lane, Err(ShaderError::Unsupported { what, .. }) if *what == "a wave reduction where not every lane runs"),
        "{:?}",
        per_lane.err()
    );
    // ... and the lowering names the block, around which that branch is the one to widen
    let needs = lower::take_needs();
    let st = structure::structure(&p).expect("structure");
    assert_eq!(lower::branches_around(&st, &needs), Some(vec![branch]));
    // (taken, past the body, when every lane's bit is clear)
    assert_eq!(st.wave_any[branch], Some(false));
    // wave-wide, every lane runs the reduction (EXEC stays a guard on what it does there)
    let wide = HashSet::from([branch]);
    let st = structure::structure_wide(&p, &wide).expect("structure");
    let inside = match p.blocks[branch].exit {
        ir::Exit::Branch { not_taken, .. } => not_taken,
        _ => unreachable!(),
    };
    assert!(st.guards[inside].iter().any(Option::is_some));
    let shader = lowered(&native, &p, &wide).expect("lower");
    if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        let mut copy = shader.dxil.clone();
        v.validate(&mut copy).expect("DXIL validation failed");
    }
    // On a GPU that takes waves of 32 (two host waves for the console's one): the OR is of
    // both halves, whichever half's lanes take the branch.
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(host) = Host::new(false) else {
        eprintln!("no SM 6.6 hardware device; not run");
        return;
    };
    let (root, pso) = match host.compute_pipeline(&shader) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("the hardware device does not take this program (waves of 32?): {e}; not run");
            return;
        }
    };
    // (threads below s0 take the branch; thread t gives bit t >> 1)
    for (below, expected) in [(64u32, u32::MAX), (40, 0x000F_FFFF), (33, 0x0001_FFFF), (10, 0x1F), (1, 1)] {
        let mut ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
        for u in &shader.user_data {
            if let (RootType::Constants, 0, 0) = (u.root, u.slot, u.dword) {
                ud[u.index as usize] = below;
            }
        }
        let out = host
            .dispatch(
                &root,
                &pso,
                &Dispatch {
                    roots: vec![(0, Root::Constants(ud))],
                    views: &[View::RawUav(1)],
                    samplers: &[],
                    groups: [1, 1, 1],
                },
            )
            .unwrap_or_else(|e| panic!("dispatch failed: {e}"));
        assert_eq!(out[0][0], expected, "{name}: {below} threads in the branch");
    }
    eprintln!("ran on the hardware device ({name}): the OR of both halves in every case");
}

/// Runs `shader` on the hardware device with the root constants `s0`, `s1` and gives dword 0
/// of the UAV; None (and a line) without a device that takes it.
fn on_the_card(shader: &HostShader, constants: [u32; 2]) -> Option<u32> {
    let Ok(host) = Host::new(false) else {
        eprintln!("no SM 6.6 hardware device; not run");
        return None;
    };
    let Ok((root, pso)) = host.compute_pipeline(shader) else {
        eprintln!("the hardware device does not take this program; not run");
        return None;
    };
    let mut ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
    for u in &shader.user_data {
        if let (RootType::Constants, 0, d @ 0..=1) = (u.root, u.slot, u.dword) {
            ud[u.index as usize] = constants[d as usize];
        }
    }
    let out = host
        .dispatch(
            &root,
            &pso,
            &Dispatch {
                roots: vec![(0, Root::Constants(ud))],
                views: &[View::RawUav(1)],
                samplers: &[],
                groups: [1, 1, 1],
            },
        )
        .unwrap_or_else(|e| panic!("dispatch failed: {e}"));
    Some(out[0][0])
}

/// A 32-lane console wave. A register holds a lane mask on one path (the lanes below s0)
/// and a number on the other, and is read as a number after the join, as the trace programs
/// keep their lanes' masks and their node numbers on one stack
/// (`deferred_tiled:CS_LaserLightShadowRaytracing`, s46 at 0x104, 0x75c and 0x188). The
/// mask arrives as the ballot of its lanes.
const MASK_OR_NUMBER: [u32; 8] = [
    0x7D88_0000, // v_cmp_gt_u32 vcc_lo, s0, v0
    0xBE94_036A, // s_mov_b32 s20, vcc_lo
    0xBF06_8001, // s_cmp_eq_u32 s1, 0
    0xBF85_0001, // s_cbranch_scc1 over the next
    0xBE94_0385, // s_mov_b32 s20, 5
    0x7E00_0214, // v_mov_b32 v0, s20
    0x38 << 26 | 0x1C << 18,
    0x80 << 24 | 2 << 16, // buffer_store_dword v0, off, s[8:11], 0
];

/// The program above reads a lane mask as a number (on the path that kept it), so it needs
/// the console's 32 lanes in a host wave: it declares `WaveSize(32)`. Without it, a host of
/// 16-lane waves gave only 16 of the mask's bits (#356).
#[test]
fn a_lane_mask_joined_with_a_number_needs_the_consoles_wave_size() {
    let native = native_of(&MASK_OR_NUMBER, false, 32);
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    let st = structure::structure(&p).expect("structure");
    let res =
        resource::track(&p, &native.layout, native.stage, Some(&st)).expect("resource::track");
    let io = io::raise(&p, &native).expect("io::raise");
    let was = lower::TEST_HOST_WAVE.replace(Some(32));
    let lowered = lower::lower(&lower::Parts {
        native: &native,
        p: &p,
        st: &st,
        io: &io,
        res: &res,
        inputs: Vec::new(),
        outputs: Vec::new(),
        prims: Vec::new(),
        mesh_stride: 0,
    });
    lower::TEST_HOST_WAVE.set(was);
    assert_eq!(lowered.expect("lower").wave_size, Some(32));
}

#[test]
fn a_lane_mask_joined_with_a_number_is_read_as_the_number_it_is() {
    let native = native_of(&MASK_OR_NUMBER, false, 32);
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    let shader = lowered(&native, &p, &HashSet::new()).expect("lower");
    if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        let mut copy = shader.dxil.clone();
        v.validate(&mut copy).expect("DXIL validation failed");
    }
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for (below, other, expected) in [(32u32, 0u32, u32::MAX), (10, 0, 0x3FF), (0, 0, 0), (10, 1, 5)] {
        let Some(got) = on_the_card(&shader, [below, other]) else { return };
        assert_eq!(got, expected, "{below} lanes in the mask, s1 = {other}");
    }
    eprintln!("ran on the hardware device: the mask's bits, or the number");
}

/// A 32-lane console wave. `if (t < s0) { ... SCC = (s1 == 7) }` and a branch on SCC after
/// the join: the skip brings the SCC of `s_and_saveexec` ("some lane is left") along
/// (`rtdirectionalshadows:CS_RTShadowTrace32HQ_TiledFarOnly`, 0x360-0x39c). On the console
/// the skip is taken by the wave, so with some lane inside every lane sees the inner test.
/// A lane that skipped on its own would see "some lane is left" instead: refused per lane,
/// right with the branch taken wave-wide. Lane 31 stores what it saw.
#[test]
fn scc_of_a_mask_operation_joined_with_a_scalar_test_needs_the_skip_wave_wide() {
    let words = [
        0x7D88_0000, // v_cmp_gt_u32 vcc_lo, s0, v0
        0xBE8D_3C6A, // s_and_saveexec_b32 s13, vcc_lo
        0xBF88_0001, // s_cbranch_execz over the next
        0xBF06_8701, // s_cmp_eq_u32 s1, 7
        0xBEFE_030D, // s_mov_b32 exec_lo, s13
        0xBF85_0002, // s_cbranch_scc1 to "seven"
        0x7E0A_0281, // v_mov_b32 v5, 1
        0xBF82_0001, // s_branch over the next
        0x7E0A_0282, // seven: v_mov_b32 v5, 2
        0x7DA4_009F, // v_cmpx_eq_u32 exec_lo, 31, v0
        0x38 << 26 | 0x1C << 18,
        0x80 << 24 | 2 << 16 | 5 << 8, // buffer_store_dword v5, off, s[8:11], 0
    ];
    let native = native_of(&words, false, 32);
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    let branch = (0..p.blocks.len())
        .find(|&b| p.blocks[b].insts.last().is_some_and(|i| i.name() == "s_cbranch_execz"))
        .expect("the skip");
    lower::take_needs();
    let per_lane = lowered(&native, &p, &HashSet::new());
    assert!(
        matches!(&per_lane, Err(ShaderError::Unsupported { what, .. }) if *what == "a lane mask read as data"),
        "{:?}",
        per_lane.err()
    );
    let st = structure::structure(&p).expect("structure");
    assert_eq!(lower::branches_around(&st, &lower::take_needs()), Some(vec![branch]));
    let shader = lowered(&native, &p, &HashSet::from([branch])).expect("lower");
    if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        let mut copy = shader.dxil.clone();
        v.validate(&mut copy).expect("DXIL validation failed");
    }
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // (lanes below s0 inside, s1) -> what lane 31 stores: 2 if SCC was set at the branch
    for (below, s1, expected) in [(0u32, 7u32, 1u32), (10, 7, 2), (10, 3, 1), (32, 3, 1), (32, 7, 2)] {
        let Some(got) = on_the_card(&shader, [below, s1]) else { return };
        assert_eq!(got, expected, "{below} lanes inside, s1 = {s1}");
    }
    eprintln!("ran on the hardware device: the inner test's SCC for every lane once some lane is inside");
}

/// Wave64 compute groups whose size is not a multiple of 32 (e.g. 48 = 32 + 16,
/// and 16 < 32) lowered to 32-lane host waves: every console thread index
/// `0..threads` must execute exactly once per group, with no skipped threads,
/// duplicate wraps, or out-of-bounds indices.
#[test]
fn partial_group_wave64_on_wave32_assigns_every_thread_once() {
    let words = [
        // ds_swizzle_b32 v2, v0 offset:0x80E4 (triggers console_lanes -> halves)
        (0x36 << 26) | (0x35 << 18) | 0x80E4,
        2 << 24,
        0x3402_0082, // v_lshlrev_b32 v1, 2, v0 (byte offset = thread_id.x * 4)
        0x7E06_0281, // v_mov_b32 v3, 1
        // buffer_atomic_add v3, v1, s[8:11], 0 offen
        (0x38 << 26) | (0x32 << 18) | (1 << 12),
        (0x80 << 24) | (2 << 16) | (3 << 8) | 1,
    ];
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for threads in [16u32, 48u32] {
        let native = native_of(&words, true, threads);
        let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
        let shader = lowered(&native, &p, &HashSet::new()).expect("lower");
        if let Ok(mut v) =
            validator::Validator::load(validator::Validator::default_path().as_deref())
        {
            let mut copy = shader.dxil.clone();
            v.validate(&mut copy)
                .unwrap_or_else(|e| panic!("DXIL validation failed for {threads} threads: {e}"));
        }
        let Ok(host) = Host::new(false) else {
            continue;
        };
        let Ok((root, pso)) = host.compute_pipeline(&shader) else {
            continue;
        };
        let groups = 64u32;
        let ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
        let out = host
            .dispatch(
                &root,
                &pso,
                &Dispatch {
                    roots: vec![(0, Root::Constants(ud))],
                    views: &[View::RawUav(64)],
                    samplers: &[],
                    groups: [groups, 1, 1],
                },
            )
            .unwrap_or_else(|e| panic!("dispatch failed ({threads} threads): {e}"));
        for slot in 0..threads as usize {
            assert_eq!(
                out[0][slot], groups,
                "threads={threads}: slot {slot} got {} (expected {groups})",
                out[0][slot]
            );
        }
        for slot in threads as usize..64 {
            assert_eq!(
                out[0][slot], 0,
                "threads={threads}: out-of-range slot {slot} was written ({})",
                out[0][slot]
            );
        }
    }
}
