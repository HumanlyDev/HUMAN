//! Windows' timeout detection and recovery (TDR) for the host GPU, read once
//! when the host device is made (#1054).
//!
//! With `TdrLevel` 0 a GPU that stops answering is never reset: a slow or
//! hung world frame freezes the whole desktop, and the runtime never sees a
//! device removal it could report. With 1 the timeout is a blue screen. Both
//! are set by hand (often to keep slow frames from being reset), so the log
//! names them; raising `TdrDelay` is what gives slow frames room.

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW};

/// Where Windows keeps the values.
const KEY: &str = r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers";

/// The TDR values in the registry; `None` where one is not set and Windows
/// uses its default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tdr {
    /// 0 off, 1 bugcheck, 2 recover to VGA, 3 recover (the default).
    pub(crate) level: Option<u32>,
    /// Seconds the GPU may take before it counts as hung (default 2).
    pub(crate) delay: Option<u32>,
    /// Seconds a driver call may take (default 5).
    pub(crate) ddi_delay: Option<u32>,
}

impl Tdr {
    pub(crate) fn read() -> Self {
        Tdr {
            level: dword("TdrLevel"),
            delay: dword("TdrDelay"),
            ddi_delay: dword("TdrDdiDelay"),
        }
    }

    /// What a hung GPU would do to the PC with these values, when that is
    /// worse than a device removal the runtime reports.
    pub(crate) fn warning(&self) -> Option<&'static str> {
        match self.level {
            Some(0) => Some(
                "TdrLevel is 0: Windows never resets a hung GPU, so a GPU hang freezes \
                 the whole PC instead of ending the run with a device removal; delete \
                 TdrLevel (or set it to 3) and raise TdrDelay to give slow frames room",
            ),
            Some(1) => Some(
                "TdrLevel is 1: a GPU timeout stops Windows with a blue screen instead \
                 of ending the run with a device removal; delete TdrLevel (or set it \
                 to 3) and raise TdrDelay to give slow frames room",
            ),
            _ => None,
        }
    }

    /// Warns when recovery is active (`TdrLevel` default/2/3) but `TdrDelay` is
    /// still at or near the 2-second Windows default, which can reset the GPU
    /// during heavy RT GI or cold shader-compilation frames.
    pub(crate) fn delay_warning(&self) -> Option<&'static str> {
        if matches!(self.level, Some(0 | 1)) {
            return None;
        }
        (self.delay.unwrap_or(2) < 10).then_some(
            "TdrDelay is under 10 seconds (Windows default is 2s): heavy RT GI or cold \
             pipeline compilation frames may trigger spurious DXGI_ERROR_DEVICE_HUNG \
             resets; consider setting TdrDelay (and TdrDdiDelay) to 30 or 60 in \
             HKLM\\SYSTEM\\CurrentControlSet\\Control\\GraphicsDrivers",
        )
    }
}

/// A DWORD value under [`KEY`]; `None` when it is missing or not a DWORD.
fn dword(name: &str) -> Option<u32> {
    let key = wide(KEY);
    let name = wide(name);
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `key` and `name` are NUL-terminated UTF-16 strings that outlive
    // the call, and `value` has room for the `size` bytes a DWORD takes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut value).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn shown(value: Option<u32>) -> String {
    value.map_or_else(|| "default".to_owned(), |v| v.to_string())
}

/// Logs the values, and warns when a GPU hang would freeze the PC or stop
/// it with a blue screen.
pub(crate) fn log() {
    let tdr = Tdr::read();
    tracing::info!(
        level = %shown(tdr.level),
        delay = %shown(tdr.delay),
        ddi_delay = %shown(tdr.ddi_delay),
        "GPU timeout detection (TdrLevel, TdrDelay, TdrDdiDelay)"
    );
    if let Some(warning) = tdr.warning() {
        tracing::warn!("{warning}");
    }
    if let Some(delay_warning) = tdr.delay_warning() {
        tracing::warn!("{delay_warning}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(level: Option<u32>) -> Tdr {
        Tdr {
            level,
            ..Tdr::default()
        }
    }

    #[test]
    fn off_and_bugcheck_warn() {
        assert!(level(Some(0)).warning().unwrap().contains("freezes"));
        assert!(level(Some(1)).warning().unwrap().contains("blue screen"));
        assert_eq!(level(Some(0)).delay_warning(), None);
        assert_eq!(level(Some(1)).delay_warning(), None);
    }

    #[test]
    fn recovery_and_the_default_do_not_warn() {
        for l in [None, Some(2), Some(3)] {
            assert_eq!(level(l).warning(), None, "TdrLevel {l:?}");
        }
    }

    #[test]
    fn short_or_default_delay_warns_under_recovery_levels() {
        for l in [None, Some(2), Some(3)] {
            assert!(
                Tdr {
                    level: l,
                    delay: None,
                    ddi_delay: None
                }
                .delay_warning()
                .unwrap()
                .contains("TdrDelay is under 10 seconds")
            );
            assert!(
                Tdr {
                    level: l,
                    delay: Some(5),
                    ddi_delay: None
                }
                .delay_warning()
                .is_some()
            );
            assert_eq!(
                Tdr {
                    level: l,
                    delay: Some(10),
                    ddi_delay: None
                }
                .delay_warning(),
                None
            );
        }
    }

    #[test]
    fn delay_alone_does_not_warn() {
        let tdr = Tdr {
            level: Some(3),
            delay: Some(60),
            ddi_delay: Some(60),
        };
        assert_eq!(tdr.warning(), None);
    }

    #[test]
    fn a_missing_value_reads_as_the_default() {
        assert_eq!(dword("VictoryTestNoSuchValue"), None);
    }

    #[test]
    fn shown_names_the_default() {
        assert_eq!(shown(None), "default");
        assert_eq!(shown(Some(60)), "60");
    }
}
