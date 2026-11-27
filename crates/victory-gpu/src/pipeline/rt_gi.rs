//! Global Illumination ray-tracing policy (`VICTORY_RT_GI=1`): enables the
//! BVH builder and diffuse GI probe/resolve passes (`ScreenProbes`,
//! `IrradianceCache`, `PrimaryProbes`, `SkyVisibility`, `RaytracingResolve`)
//! while skipping expensive full-resolution reflection and direct shadow ray
//! traces unless explicitly requested (`VICTORY_RT_ALL=1`,
//! `VICTORY_RT_REFLECTIONS=1`, `VICTORY_RT_SHADOWS=1`).

use std::sync::OnceLock;

fn env_flag(name: &str) -> Option<bool> {
    std::env::var(name)
        .ok()
        .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        })
}

/// Whether `VICTORY_RT_GI=1` mode is active.
pub(crate) fn gi_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| env_flag("VICTORY_RT_GI").unwrap_or(false))
}

/// Whether the ray-tracing structure builder (`XboxBvhBuilder`) is allowed to
/// run (`VICTORY_BVH_BUILDER=1`, or `VICTORY_RT_GI=1` unless
/// `VICTORY_BVH_BUILDER=0`).
pub(crate) fn builder_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| env_flag("VICTORY_BVH_BUILDER").unwrap_or_else(gi_enabled))
}

fn skip_non_gi_trace_with(
    gi: bool,
    all: bool,
    reflections: bool,
    shadows: bool,
    container: &[u8],
) -> bool {
    if !gi || all {
        return false;
    }
    let has = |needle: &[u8]| container.windows(needle.len()).any(|w| w == needle);
    if has(b"IrradianceCache")
        || has(b"ScreenProbes")
        || has(b"PrimaryProbes")
        || has(b"SkyVisibility")
        || has(b"RaytracingResolve")
    {
        return false;
    }
    // Note: reflection traces (CS_RaytraceVBuffer / CS_Reflections_Raytrace /
    // CS_GlossyReflections) are NOT skipped as dropped pipelines here: instead,
    // `victory-shader` forces instant BVH root-misses for them when
    // `!reflections`, so `CS_RayMiss` still shades off-screen foreground water
    // reflections with the sky cubemap instead of leaving them black.
    let _ = reflections;
    if !shadows && (has(b"CS_RaytracedShadows") || has(b"ShadowTrace")) {
        return true;
    }
    false
}

/// Whether a compute shader container is a non-GI reflection or direct-shadow
/// ray trace skipped under `VICTORY_RT_GI=1`.
pub(crate) fn skip_non_gi_trace(container: &[u8]) -> bool {
    static FLAGS: OnceLock<(bool, bool, bool, bool)> = OnceLock::new();
    let (gi, all, reflections, shadows) = *FLAGS.get_or_init(|| {
        (
            gi_enabled(),
            env_flag("VICTORY_RT_ALL").unwrap_or(false),
            env_flag("VICTORY_RT_REFLECTIONS").unwrap_or(false),
            env_flag("VICTORY_RT_SHADOWS").unwrap_or(false),
        )
    });
    skip_non_gi_trace_with(gi, all, reflections, shadows, container)
}

#[cfg(test)]
mod tests {
    use super::skip_non_gi_trace_with;

    #[test]
    fn gi_passes_stay_enabled_while_non_gi_traces_skip() {
        for gi_shader in [
            b"CS_ScreenProbes_Trace_Wave64".as_slice(),
            b"CS_IrradianceCache_ShadowTrace_WithFar".as_slice(),
            b"CS_PrimaryProbes_Trace".as_slice(),
            b"CS_SkyVisibility_Trace".as_slice(),
            b"CS_RaytracingResolve_Probe".as_slice(),
        ] {
            assert!(
                !skip_non_gi_trace_with(true, false, false, false, gi_shader),
                "GI shader {:?} must not be skipped",
                String::from_utf8_lossy(gi_shader)
            );
        }

        for reflection_shader in [
            b"CS_RaytraceVBuffer_Primary".as_slice(),
            b"CS_Reflections_Raytrace_Glossy".as_slice(),
            b"CS_GlossyReflections_Trace".as_slice(),
        ] {
            assert!(
                !skip_non_gi_trace_with(true, false, false, false, reflection_shader),
                "reflection shader {:?} must stay enabled (using forced BVH miss) so CS_RayMiss shades off-screen water",
                String::from_utf8_lossy(reflection_shader)
            );
        }

        for non_gi in [
            b"CS_RaytracedShadows_Sun".as_slice(),
            b"CS_LocalLight_ShadowTrace".as_slice(),
        ] {
            assert!(
                skip_non_gi_trace_with(true, false, false, false, non_gi),
                "non-GI shader {:?} should be skipped in RT-GI mode",
                String::from_utf8_lossy(non_gi)
            );
            assert!(!skip_non_gi_trace_with(false, false, false, false, non_gi));
            assert!(!skip_non_gi_trace_with(true, true, false, false, non_gi));
        }

        assert!(!skip_non_gi_trace_with(
            true,
            false,
            true,
            false,
            b"CS_RaytraceVBuffer_Primary"
        ));
        assert!(!skip_non_gi_trace_with(
            true,
            false,
            false,
            true,
            b"CS_RaytracedShadows_Sun"
        ));
    }
}
