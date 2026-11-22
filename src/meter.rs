//! Live spectrum of what the headset's Game channel is playing.
//!
//! The meter only *reads* audio: it records the monitor of the exact Quantum
//! Game sink with `pw-record` (a capture stream; nothing is linked into or
//! out of the playback graph and no route is changed) and turns it into
//! [`BANDS`] log-spaced levels plus a peak. No HID device is touched.
//!
//! Two front ends share the analysis:
//! - `soniccore meter --format json [--frames N]` prints one JSON line per
//!   frame, for diagnostics and tests;
//! - `soniccore meter --dbus` serves `org.jambalinux.soniccore.Meter` on the
//!   session bus. D-Bus activation starts it when the widget asks for a
//!   frame; capture stops two seconds after the last request and the process
//!   exits after thirty idle seconds, so it costs nothing while unused.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::raw::{c_int, c_ulong};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Number of spectrum bands, from [`LOWEST_HZ`] to [`HIGHEST_HZ`].
pub const BANDS: usize = 32;
const LOWEST_HZ: f32 = 31.0;
const HIGHEST_HZ: f32 = 16_000.0;
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// FFT window: 2048 samples is ~43 ms at 48 kHz (23 Hz per bin).
const WINDOW: usize = 2048;
/// New frame every 1920 samples: 40 ms, 25 frames per second.
const HOP: usize = 1920;
/// Levels map -70..-10 dBFS onto 0..1.
const FLOOR_DB: f32 = -70.0;
const CEILING_DB: f32 = -10.0;
/// Below this peak the frame is silence and the FFT is skipped.
const SILENCE_PEAK: f32 = 0.000_5;

const BUS_NAME: &str = "org.jambalinux.soniccore.Meter";
const OBJECT_PATH: &str = "/org/jambalinux/soniccore/Meter";
const CAPTURE_IDLE: Duration = Duration::from_secs(2);
const EXIT_IDLE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub bands: [f32; BANDS],
    pub peak: f32,
    /// False for silence (or no capture): the widget can calm its visuals.
    pub active: bool,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            bands: [0.0; BANDS],
            peak: 0.0,
            active: false,
        }
    }
}

impl Frame {
    pub fn to_json(&self) -> String {
        let bands = self
            .bands
            .iter()
            .map(|level| format!("{level:.3}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"bands\":[{bands}],\"peak\":{:.3},\"active\":{}}}",
            self.peak, self.active
        )
    }
}

/// In-place iterative radix-2 FFT; `real.len()` must be a power of two.
fn fft(real: &mut [f32], imag: &mut [f32]) {
    let size = real.len();
    let mut j = 0;
    for i in 1..size {
        let mut bit = size >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            real.swap(i, j);
            imag.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= size {
        let angle = -2.0 * std::f32::consts::PI / length as f32;
        let (step_sin, step_cos) = angle.sin_cos();
        for start in (0..size).step_by(length) {
            let (mut w_real, mut w_imag) = (1.0_f32, 0.0_f32);
            for offset in 0..length / 2 {
                let even = start + offset;
                let odd = even + length / 2;
                let t_real = real[odd] * w_real - imag[odd] * w_imag;
                let t_imag = real[odd] * w_imag + imag[odd] * w_real;
                real[odd] = real[even] - t_real;
                imag[odd] = imag[even] - t_imag;
                real[even] += t_real;
                imag[even] += t_imag;
                let next = w_real * step_cos - w_imag * step_sin;
                w_imag = w_real * step_sin + w_imag * step_cos;
                w_real = next;
            }
        }
        length <<= 1;
    }
}

/// Turns windows of mono samples into band levels.
pub struct Analyzer {
    window: Vec<f32>,
    window_sum: f32,
    /// FFT bin range `[start, end)` of each band.
    bins: Vec<(usize, usize)>,
}

impl Analyzer {
    pub fn new() -> Self {
        let window = (0..WINDOW)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * index as f32 / (WINDOW - 1) as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect::<Vec<_>>();
        let window_sum = window.iter().sum();
        let bin_hz = SAMPLE_RATE as f32 / WINDOW as f32;
        let ratio = HIGHEST_HZ / LOWEST_HZ;
        let bins = (0..BANDS)
            .map(|band| {
                let low = LOWEST_HZ * ratio.powf(band as f32 / BANDS as f32);
                let high = LOWEST_HZ * ratio.powf((band + 1) as f32 / BANDS as f32);
                let start = ((low / bin_hz).round() as usize).max(1);
                // Low bands are narrower than one bin: always cover one.
                let end = ((high / bin_hz).round() as usize).max(start + 1);
                (start, end.min(WINDOW / 2))
            })
            .collect();
        Self {
            window,
            window_sum,
            bins,
        }
    }

    /// `samples` must hold exactly [`WINDOW`] mono samples in -1..1.
    pub fn analyze(&self, samples: &[f32]) -> Frame {
        let peak = samples
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        if peak < SILENCE_PEAK || samples.len() != WINDOW {
            return Frame {
                peak,
                ..Frame::default()
            };
        }
        let mut real = samples
            .iter()
            .zip(&self.window)
            .map(|(sample, weight)| sample * weight)
            .collect::<Vec<_>>();
        let mut imag = vec![0.0; WINDOW];
        fft(&mut real, &mut imag);

        let mut bands = [0.0; BANDS];
        for (level, &(start, end)) in bands.iter_mut().zip(&self.bins) {
            let magnitude = (start..end)
                .map(|bin| (real[bin] * real[bin] + imag[bin] * imag[bin]).sqrt())
                .fold(0.0_f32, f32::max);
            // Amplitude of a full-scale sine is 1.0 (0 dBFS).
            let amplitude = 2.0 * magnitude / self.window_sum;
            let decibels = 20.0 * amplitude.max(1e-9).log10();
            *level = ((decibels - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0);
        }
        Frame {
            bands,
            peak: peak.min(1.0),
            active: true,
        }
    }
}

/// Rolling mono buffer fed with interleaved stereo float frames.
struct Window {
    samples: VecDeque<f32>,
    since_frame: usize,
    /// Loudest sample of either channel since the last frame: every sample
    /// is seen once, so a single clipped sample is never missed (the mono
    /// mix the spectrum uses would halve a clip on one side).
    channel_peak: f32,
}

impl Window {
    fn new() -> Self {
        Self {
            samples: VecDeque::from(vec![0.0; WINDOW]),
            since_frame: 0,
            channel_peak: 0.0,
        }
    }

    /// Adds one stereo frame; returns true when a new analysis frame is due.
    fn push(&mut self, left: f32, right: f32) -> bool {
        self.samples.pop_front();
        self.samples.push_back((left + right) * 0.5);
        self.channel_peak = self.channel_peak.max(left.abs()).max(right.abs());
        self.since_frame += 1;
        if self.since_frame >= HOP {
            self.since_frame = 0;
            true
        } else {
            false
        }
    }

    fn contiguous(&mut self) -> &[f32] {
        self.samples.make_contiguous()
    }

    fn take_peak(&mut self) -> f32 {
        std::mem::take(&mut self.channel_peak)
    }
}

unsafe extern "C" {
    fn prctl(option: c_int, arg2: c_ulong, arg3: c_ulong, arg4: c_ulong, arg5: c_ulong) -> c_int;
}
const PR_SET_PDEATHSIG: c_int = 1;
const SIGKILL: c_ulong = 9;

/// A running `pw-record` on the Game sink monitor and its reader thread.
struct Capture {
    child: Child,
    reader: Option<JoinHandle<()>>,
}

impl Capture {
    fn start(on_frame: impl FnMut(Frame) + Send + 'static) -> Result<Self, String> {
        let game = crate::pipewire::game_sink_name(&crate::pipewire::pw_dump()?)?;
        let mut command = Command::new("pw-record");
        command
            .args([
                "--raw",
                "--format",
                "f32",
                "--rate",
                &SAMPLE_RATE.to_string(),
                "--channels",
                &CHANNELS.to_string(),
                "--latency",
                "20ms",
                "-P",
                // A passive capture of the sink's monitor: it never keeps the
                // device awake and is never routed into the playback graph.
                "{ stream.capture.sink = true node.passive = true node.name = \"jambalinux-soniccore-meter\" node.description = \"JamBaLinux Sonic Core meter\" }",
                "--target",
                &game,
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // The recorder must not outlive the meter, even if it is killed.
        unsafe {
            command.pre_exec(|| {
                prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0);
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("could not start pw-record: {error}"))?;
        let stdout = child.stdout.take().ok_or("pw-record has no output pipe")?;
        let reader = thread::spawn(move || read_frames(stdout, on_frame));
        Ok(Self {
            child,
            reader: Some(reader),
        })
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Reads interleaved stereo f32 from `input` and reports a frame every
/// [`HOP`] samples until the stream ends.
fn read_frames(mut input: impl Read, mut on_frame: impl FnMut(Frame)) {
    let analyzer = Analyzer::new();
    let mut window = Window::new();
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut carry = Vec::<u8>::new();
    const FRAME_BYTES: usize = CHANNELS * 4;
    loop {
        let count = match input.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(count) => count,
        };
        carry.extend_from_slice(&buffer[..count]);
        let whole = carry.len() / FRAME_BYTES * FRAME_BYTES;
        for frame in carry[..whole].as_chunks::<FRAME_BYTES>().0 {
            let left = f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
            let right = f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
            if window.push(left, right) {
                let mut frame = analyzer.analyze(window.contiguous());
                // Above 1.0 (0 dBFS) the sound clips on its way out.
                frame.peak = window.take_peak();
                on_frame(frame);
            }
        }
        carry.drain(..whole);
    }
}

/// `soniccore meter --format json [--frames N]`
pub fn print_frames(limit: Option<usize>) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut capture = Capture::start(move |frame| {
        let _ = sender.send(frame);
    })?;
    let mut printed = 0;
    let stdout = std::io::stdout();
    while limit.is_none_or(|limit| printed < limit) {
        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(frame) => {
                let mut out = stdout.lock();
                writeln!(out, "{}", frame.to_json()).map_err(|error| error.to_string())?;
                out.flush().map_err(|error| error.to_string())?;
                printed += 1;
            }
            Err(_) if !capture.running() => {
                return Err("pw-record stopped; is the Quantum Game sink available?".into());
            }
            Err(_) => {}
        }
    }
    Ok(())
}

struct Shared {
    frame: Frame,
    last_request: Instant,
}

struct MeterService {
    shared: Arc<Mutex<Shared>>,
}

#[zbus::interface(name = "org.jambalinux.soniccore.Meter")]
impl MeterService {
    /// Latest frame: band levels (0..1), peak (0..1) and whether sound is
    /// playing. Each call keeps the capture running for two more seconds.
    fn frame(&self) -> (Vec<f64>, f64, bool) {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        shared.last_request = Instant::now();
        (
            shared
                .frame
                .bands
                .iter()
                .map(|&level| f64::from(level))
                .collect(),
            f64::from(shared.frame.peak),
            shared.frame.active,
        )
    }
}

/// `soniccore meter --dbus`: on-demand session service for the widget.
pub fn serve_dbus() -> Result<(), String> {
    let shared = Arc::new(Mutex::new(Shared {
        frame: Frame::default(),
        last_request: Instant::now(),
    }));
    let _connection = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.name(BUS_NAME))
        .and_then(|builder| {
            builder.serve_at(
                OBJECT_PATH,
                MeterService {
                    shared: shared.clone(),
                },
            )
        })
        .and_then(|builder| builder.build())
        .map_err(|error| format!("could not serve {BUS_NAME}: {error}"))?;

    let mut capture: Option<Capture> = None;
    let mut retry_after = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(100));
        let idle = shared
            .lock()
            .map(|shared| shared.last_request.elapsed())
            .unwrap_or(EXIT_IDLE);
        if idle >= EXIT_IDLE {
            return Ok(());
        }
        let wanted = idle < CAPTURE_IDLE;
        if capture.as_mut().is_some_and(|capture| !capture.running()) {
            capture = None;
        }
        match (wanted, capture.is_some()) {
            (true, false) if Instant::now() >= retry_after => {
                let sink = shared.clone();
                match Capture::start(move |frame| {
                    if let Ok(mut shared) = sink.lock() {
                        shared.frame = frame;
                    }
                }) {
                    Ok(started) => capture = Some(started),
                    Err(error) => {
                        eprintln!("meter: {error}");
                        retry_after = Instant::now() + Duration::from_secs(2);
                    }
                }
            }
            (false, true) => {
                capture = None;
                if let Ok(mut shared) = shared.lock() {
                    shared.frame = Frame::default();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f32, amplitude: f32) -> Vec<f32> {
        (0..WINDOW)
            .map(|index| {
                amplitude
                    * (2.0 * std::f32::consts::PI * frequency * index as f32 / SAMPLE_RATE as f32)
                        .sin()
            })
            .collect()
    }

    fn loudest_band(frame: &Frame) -> usize {
        frame
            .bands
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map(|(band, _)| band)
            .unwrap()
    }

    fn band_of(frequency: f32) -> usize {
        let ratio = HIGHEST_HZ / LOWEST_HZ;
        ((frequency / LOWEST_HZ).ln() / ratio.ln() * BANDS as f32).floor() as usize
    }

    #[test]
    fn a_tone_lights_the_band_that_contains_it() {
        let analyzer = Analyzer::new();
        for frequency in [60.0, 440.0, 1_000.0, 5_000.0, 12_000.0] {
            let frame = analyzer.analyze(&sine(frequency, 0.5));
            assert!(frame.active);
            let loudest = loudest_band(&frame);
            assert!(
                loudest.abs_diff(band_of(frequency)) <= 1,
                "{frequency} Hz peaked in band {loudest}, expected {}",
                band_of(frequency)
            );
            assert!(
                frame.bands[loudest] > 0.9,
                "-6 dBFS is near the top of the scale"
            );
            assert!((frame.peak - 0.5).abs() < 0.01);
        }
    }

    #[test]
    fn quieter_tones_read_lower_and_silence_is_inactive() {
        let analyzer = Analyzer::new();
        let loud = analyzer.analyze(&sine(1_000.0, 0.1));
        let quiet = analyzer.analyze(&sine(1_000.0, 0.01));
        let band = loudest_band(&loud);
        // 20 dB less is a third of the 60 dB scale.
        assert!((loud.bands[band] - quiet.bands[band] - 20.0 / 60.0).abs() < 0.05);

        let silence = analyzer.analyze(&vec![0.0; WINDOW]);
        assert!(!silence.active);
        assert!(silence.bands.iter().all(|&level| level == 0.0));
    }

    #[test]
    fn a_clip_on_one_channel_shows_in_the_frame_peak() {
        let mut bytes = Vec::new();
        for index in 0..(HOP * 2) {
            // Quiet left, and one right sample past full scale.
            let right = if index == HOP + 7 { 1.4_f32 } else { 0.1 };
            bytes.extend_from_slice(&0.1_f32.to_le_bytes());
            bytes.extend_from_slice(&right.to_le_bytes());
        }
        let mut frames = Vec::new();
        read_frames(std::io::Cursor::new(bytes), |frame| frames.push(frame));
        assert_eq!(frames.len(), 2);
        assert!((frames[0].peak - 0.1).abs() < 1e-6);
        assert!((frames[1].peak - 1.4).abs() < 1e-6);
    }

    #[test]
    fn the_reader_emits_one_frame_per_hop_from_raw_stereo() {
        let samples = sine(1_000.0, 0.5);
        let mut bytes = Vec::new();
        for _ in 0..3 {
            for &sample in &samples {
                bytes.extend_from_slice(&sample.to_le_bytes());
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let mut frames = Vec::new();
        read_frames(std::io::Cursor::new(bytes), |frame| frames.push(frame));
        assert_eq!(frames.len(), 3 * WINDOW / HOP);
        assert!(frames.last().unwrap().active);
        assert!(frames.last().unwrap().to_json().starts_with("{\"bands\":["));
    }
}
