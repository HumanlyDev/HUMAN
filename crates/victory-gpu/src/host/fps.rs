//! Simple top-left FPS counter overlay, toggleable with F2.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Threading::GetSystemTimes;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;

use super::textbox::{TextBox, key_pressed};
use super::{Host, HostList};

const EVERY: Duration = Duration::from_millis(500);
const WINDOW: Duration = Duration::from_secs(10);
const COLUMNS: u32 = 16;
const LINES: u32 = 6;
const MAX_SAMPLES: usize = 2000;

static SHOWN: AtomicBool = AtomicBool::new(false);

fn toggled() -> bool {
    static DOWN: AtomicBool = AtomicBool::new(false);
    key_pressed(VK_F2, &DOWN)
}

fn system_times() -> Option<(u64, u64, u64)> {
    let ft = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut idle, mut kernel, mut user) = (zero, zero, zero);
    // SAFETY: three valid out pointers.
    let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
    (ok != 0).then(|| (ft(idle), ft(kernel), ft(user)))
}

fn cpu_load_pct(prev: &mut Option<(u64, u64, u64)>) -> f64 {
    let Some(now) = system_times() else { return 0.0 };
    let Some(old) = prev.replace(now) else { return 0.0 };
    let d_idle = now.0.saturating_sub(old.0);
    let d_total = now.1.saturating_sub(old.1) + now.2.saturating_sub(old.2);
    if d_total == 0 {
        0.0
    } else {
        (d_total.saturating_sub(d_idle) as f64 / d_total as f64 * 100.0).clamp(0.0, 100.0)
    }
}

fn avg_fps(samples: &VecDeque<(Instant, f64)>) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let total_ms: f64 = samples.iter().map(|(_, dt)| dt).sum();
    1000.0 * samples.len() as f64 / total_ms.max(0.001)
}

fn one_percent_low(samples: &VecDeque<(Instant, f64)>) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = samples.iter().map(|(_, dt)| *dt).collect();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let count = (sorted.len() / 100).max(1);
    let avg_worst_ms = sorted[..count].iter().sum::<f64>() / count as f64;
    1000.0 / avg_worst_ms.max(0.001)
}

/// Top-left FPS counter state for a presenter.
pub(super) struct FpsOverlay {
    interval_since: Instant,
    interval_frames: u32,
    last: Option<Instant>,
    samples: VecDeque<(Instant, f64)>,
    cpu_prev: Option<(u64, u64, u64)>,
    lines: Vec<String>,
    changed: bool,
    box_: TextBox,
}

impl FpsOverlay {
    pub(super) fn new() -> Self {
        Self {
            interval_since: Instant::now(),
            interval_frames: 0,
            last: None,
            samples: VecDeque::with_capacity(MAX_SAMPLES),
            cpu_prev: None,
            lines: Vec::new(),
            changed: false,
            box_: TextBox::new(COLUMNS, LINES),
        }
    }

    fn reset(&mut self, now: Instant) {
        self.interval_since = now;
        self.interval_frames = 0;
        self.last = None;
        self.samples.clear();
        self.cpu_prev = system_times();
        self.lines.clear();
        self.changed = false;
    }

    /// Called once per presented frame.
    pub(super) fn frame(&mut self) {
        if toggled() {
            let now_shown = !SHOWN.fetch_xor(true, Ordering::Relaxed);
            if now_shown {
                self.reset(Instant::now());
            }
        }
        if !SHOWN.load(Ordering::Relaxed) {
            return;
        }
        let now = Instant::now();
        let Some(prev) = self.last.replace(now) else {
            self.interval_since = now;
            return;
        };
        let dt_ms = (now - prev).as_secs_f64() * 1000.0;
        while let Some(&(t, _)) = self.samples.front() {
            if now.saturating_duration_since(t) > WINDOW || self.samples.len() >= MAX_SAMPLES {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        self.samples.push_back((now, dt_ms));
        self.interval_frames += 1;

        let elapsed = now - self.interval_since;
        if !self.lines.is_empty() && elapsed < EVERY {
            return;
        }
        let cur_ms = elapsed.as_secs_f64() * 1000.0 / f64::from(self.interval_frames);
        let cur_fps = 1000.0 / cur_ms.max(0.001);
        let avg = avg_fps(&self.samples);
        let low_fps = one_percent_low(&self.samples);
        let gpu_pct = super::gpu_load_pct();
        let cpu_pct = cpu_load_pct(&mut self.cpu_prev);

        self.lines = vec![
            format!("FPS    {cur_fps:.1}"),
            format!("AVG    {avg:.1}"),
            format!("1% LOW {low_fps:.1}"),
            format!("FT     {cur_ms:.1} MS"),
            format!("GPU    {gpu_pct}%"),
            format!("CPU    {cpu_pct:.0}%"),
        ];
        self.changed = true;
        self.interval_since = now;
        self.interval_frames = 0;
    }

    /// Draws the overlay onto `back`, which must be in `D3D12_RESOURCE_STATE_COPY_DEST`.
    pub(super) fn draw(
        &mut self,
        host: &Host,
        list: &HostList,
        back: &ID3D12Resource,
        dims: (u32, u32),
        format: DXGI_FORMAT,
    ) {
        if !SHOWN.load(Ordering::Relaxed) {
            return;
        }
        self.box_.draw(host, list, back, dims, format, &self.lines, &mut self.changed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_percent_low_averages_slowest_frames() {
        let now = Instant::now();
        let mut samples: VecDeque<(Instant, f64)> = (0..198).map(|_| (now, 10.0)).collect();
        samples.push_back((now, 50.0));
        samples.push_back((now, 50.0));
        // 200 samples -> worst 1% is 2 samples (50.0 ms each) -> 20.0 FPS.
        let low = one_percent_low(&samples);
        assert!((low - 20.0).abs() < 1e-6);
    }

    #[test]
    fn rolling_avg_fps_from_samples() {
        let now = Instant::now();
        let samples = VecDeque::from([(now, 10.0), (now, 20.0), (now, 30.0)]);
        // 3 frames in 60 ms -> 50.0 FPS.
        assert!((avg_fps(&samples) - 50.0).abs() < 1e-6);
    }
}
