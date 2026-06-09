//! Platform microphone capture + speaker playback for native call voice.
//!
//! * Desktop (Linux/macOS/Windows): [`CpalAudioBackend`] uses `cpal`. The device
//!   may run at any sample rate / channel count, so we down-mix to mono and
//!   linearly resample to/from the engine's 48 kHz mono 20 ms frames.
//! * Android (until P6) and headless builds: [`SilenceAudioBackend`] keeps the
//!   20 ms clock alive (silent capture, discarded playout) so the rest of the
//!   pipeline — transport, jitter buffer, stats — can be exercised end to end.
//!
//! All cpal streams live on a dedicated OS thread that owns them for their whole
//! lifetime (cpal `Stream` is not `Send` on every backend); audio crosses thread
//! boundaries only through `Send` channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;

use super::session::{AudioBackend, AudioStreams};
use super::FRAME_SAMPLES;

/// Engine sample rate (mono). The codec and jitter buffer all assume this.
const ENGINE_RATE: u32 = super::SAMPLE_RATE_HZ;

/// Pick the cpal backend where audio I/O is wired up, else the silent fallback.
pub fn default_audio_backend() -> Box<dyn AudioBackend> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        Box::new(CpalAudioBackend::new())
    }
    #[cfg(target_os = "android")]
    {
        // cpal/Oboe needs the JavaVM + Context first (see `set_android_audio_ready`).
        // Until the `:p2p` JNI init runs, fall back to silence so we never panic.
        if android_audio_ready() {
            Box::new(CpalAudioBackend::new())
        } else {
            log_audio_warn(
                "android audio not initialized (initAndroidAudio not called) — silent".to_string(),
            );
            Box::new(SilenceAudioBackend::new())
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    )))]
    {
        Box::new(SilenceAudioBackend::new())
    }
}

/// Android only: set once the JNI side has handed cpal/Oboe the JavaVM + Context.
#[cfg(target_os = "android")]
static ANDROID_AUDIO_READY: AtomicBool = AtomicBool::new(false);

/// Called from the `:p2p` JNI init after `ndk_context::initialize_android_context`.
#[cfg(target_os = "android")]
pub fn set_android_audio_ready() {
    ANDROID_AUDIO_READY.store(true, Ordering::Relaxed);
    log_audio_info("android audio ready (cpal/Oboe enabled)".to_string());
}

#[cfg(target_os = "android")]
fn android_audio_ready() -> bool {
    ANDROID_AUDIO_READY.load(Ordering::Relaxed)
}

#[cfg(target_os = "android")]
pub(crate) fn is_android_audio_ready() -> bool {
    android_audio_ready()
}

/// Streaming linear resampler (mono). Good enough for voice PoC quality; the
/// common desktop case (44.1 kHz ↔ 48 kHz) stays near unity ratio.
struct LinearResampler {
    /// Output spacing measured in input samples (`in_rate / out_rate`).
    step: f64,
    /// Current output position within the current input interval `[0, 1)`.
    t: f64,
    prev: f32,
    have_prev: bool,
}

impl LinearResampler {
    fn new(in_rate: u32, out_rate: u32) -> Self {
        let step = in_rate.max(1) as f64 / out_rate.max(1) as f64;
        Self {
            step,
            t: 0.0,
            prev: 0.0,
            have_prev: false,
        }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &s in input {
            if !self.have_prev {
                self.prev = s;
                self.have_prev = true;
                continue;
            }
            while self.t < 1.0 {
                let v = self.prev + (s - self.prev) * (self.t as f32);
                out.push(v);
                self.t += self.step;
            }
            while self.t >= 1.0 {
                self.t -= 1.0;
            }
            self.prev = s;
        }
    }
}

fn f32_to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// Silent backend: emits zero frames every 20 ms and drops playout. Keeps the
/// session clock alive so the transport/jitter/stats path is fully exercised.
/// Used where cpal has no usable I/O: Android before JNI audio init and any
/// platform without a cpal host (desktop always uses [`CpalAudioBackend`]).
#[cfg(any(test, not(any(target_os = "linux", target_os = "macos", target_os = "windows"))))]
pub struct SilenceAudioBackend {
    stop: Arc<AtomicBool>,
}

#[cfg(any(test, not(any(target_os = "linux", target_os = "macos", target_os = "windows"))))]
impl Default for SilenceAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, not(any(target_os = "linux", target_os = "macos", target_os = "windows"))))]
impl SilenceAudioBackend {
    pub fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[cfg(any(test, not(any(target_os = "linux", target_os = "macos", target_os = "windows"))))]
impl AudioBackend for SilenceAudioBackend {
    fn start(&mut self) -> Result<AudioStreams, String> {
        let (cap_tx, capture_rx) = mpsc::channel::<Vec<i16>>(64);
        let (playout_tx, mut playout_rx) = mpsc::channel::<Vec<i16>>(256);
        let stop = Arc::clone(&self.stop);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                tick.tick().await;
                if cap_tx.try_send(vec![0i16; FRAME_SAMPLES]).is_err()
                    && cap_tx.is_closed()
                {
                    break;
                }
            }
        });
        tokio::spawn(async move { while playout_rx.recv().await.is_some() {} });
        Ok(AudioStreams {
            capture_rx,
            playout_tx,
        })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "android"
))]
pub use cpal_backend::CpalAudioBackend;

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "android"
))]
mod cpal_backend {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample};

    /// Cap the playout backlog at ~0.5 s so a stalled callback cannot grow memory.
    fn playout_cap(rate: u32, channels: u16) -> usize {
        (rate as usize * channels as usize) / 2
    }

    pub struct CpalAudioBackend {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Default for CpalAudioBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    impl CpalAudioBackend {
        pub fn new() -> Self {
            Self {
                stop: Arc::new(AtomicBool::new(false)),
                thread: None,
            }
        }
    }

    impl AudioBackend for CpalAudioBackend {
        fn start(&mut self) -> Result<AudioStreams, String> {
            let (cap_tx, capture_rx) = mpsc::channel::<Vec<i16>>(64);
            let (playout_tx, playout_rx) = mpsc::channel::<Vec<i16>>(256);
            let stop = Arc::clone(&self.stop);

            // The audio thread builds + owns both cpal streams (not Send) and
            // signals readiness/error back through a oneshot-style channel.
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let thread = std::thread::Builder::new()
                .name("ghalbol-audio".into())
                .spawn(move || {
                    audio_thread_main(stop, cap_tx, playout_rx, ready_tx);
                })
                .map_err(|e| format!("spawn audio thread: {e}"))?;

            match ready_rx.recv_timeout(Duration::from_secs(3)) {
                Ok(Ok(())) => {
                    self.thread = Some(thread);
                    Ok(AudioStreams {
                        capture_rx,
                        playout_tx,
                    })
                }
                Ok(Err(e)) => Err(e),
                Err(_) => Err("audio device init timed out".to_string()),
            }
        }

        fn stop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    impl Drop for CpalAudioBackend {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn audio_thread_main(
        stop: Arc<AtomicBool>,
        cap_tx: mpsc::Sender<Vec<i16>>,
        mut playout_rx: mpsc::Receiver<Vec<i16>>,
        ready_tx: std::sync::mpsc::Sender<Result<(), String>>,
    ) {
        let host = cpal::default_host();

        let in_dev = host.default_input_device();
        let out_dev = match host.default_output_device() {
            Some(d) => d,
            None => {
                let _ = ready_tx.send(Err("no default output device".to_string()));
                return;
            }
        };

        // ---- Output (speaker) ----
        let out_supported = match out_dev.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("output config: {e}")));
                return;
            }
        };
        let out_rate = out_supported.sample_rate().0;
        let out_channels = out_supported.channels();
        let out_fmt = out_supported.sample_format();
        let out_cfg: cpal::StreamConfig = out_supported.config();

        let playout_buf: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let cap = playout_cap(out_rate, out_channels);

        let out_stream = match build_output_stream(
            &out_dev,
            &out_cfg,
            out_fmt,
            Arc::clone(&playout_buf),
        ) {
            Ok(s) => s,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };

        // ---- Input (mic) — optional; a call still works one-way if absent ----
        let in_stream = match &in_dev {
            Some(dev) => match dev.default_input_config() {
                Ok(supported) => {
                    let in_rate = supported.sample_rate().0;
                    let in_channels = supported.channels();
                    let in_fmt = supported.sample_format();
                    let in_cfg: cpal::StreamConfig = supported.config();
                    match build_input_stream(
                        dev,
                        &in_cfg,
                        in_fmt,
                        in_rate,
                        in_channels,
                        cap_tx.clone(),
                    ) {
                        Ok(s) => Some(s),
                        Err(e) => {
                            super::log_audio_warn(format!("mic disabled: {e}"));
                            None
                        }
                    }
                }
                Err(e) => {
                    super::log_audio_warn(format!("mic config failed: {e}"));
                    None
                }
            },
            None => {
                super::log_audio_warn("no default input device — call is playback-only".to_string());
                None
            }
        };

        if let Err(e) = out_stream.play() {
            let _ = ready_tx.send(Err(format!("start output: {e}")));
            return;
        }
        if let Some(s) = &in_stream {
            if let Err(e) = s.play() {
                super::log_audio_warn(format!("start mic: {e}"));
            }
        }

        super::log_audio_info(format!(
            "audio devices up: out {out_rate}Hz x{out_channels}, mic {}",
            if in_stream.is_some() { "on" } else { "off" }
        ));
        let _ = ready_tx.send(Ok(()));

        // Bridge engine playout (mono 48 kHz) → device buffer (out_rate x channels).
        let mut resampler = LinearResampler::new(ENGINE_RATE, out_rate);
        let mut mono_out: Vec<f32> = Vec::with_capacity(2048);
        while !stop.load(Ordering::Relaxed) {
            match playout_rx.try_recv() {
                Ok(frame) => {
                    let fin: Vec<f32> = frame.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    mono_out.clear();
                    resampler.process(&fin, &mut mono_out);
                    if let Ok(mut q) = playout_buf.lock() {
                        for &m in &mono_out {
                            for _ in 0..out_channels {
                                q.push_back(m);
                            }
                        }
                        while q.len() > cap {
                            q.pop_front();
                        }
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        drop(in_stream);
        drop(out_stream);
        super::log_audio_info("audio devices stopped".to_string());
    }

    fn build_output_stream(
        dev: &cpal::Device,
        cfg: &cpal::StreamConfig,
        fmt: cpal::SampleFormat,
        buf: Arc<Mutex<VecDeque<f32>>>,
    ) -> Result<cpal::Stream, String> {
        match fmt {
            cpal::SampleFormat::F32 => build_output_typed::<f32>(dev, cfg, buf),
            cpal::SampleFormat::I16 => build_output_typed::<i16>(dev, cfg, buf),
            cpal::SampleFormat::U16 => build_output_typed::<u16>(dev, cfg, buf),
            other => Err(format!("unsupported output format {other:?}")),
        }
    }

    fn build_output_typed<T>(
        dev: &cpal::Device,
        cfg: &cpal::StreamConfig,
        buf: Arc<Mutex<VecDeque<f32>>>,
    ) -> Result<cpal::Stream, String>
    where
        T: cpal::SizedSample + FromSample<f32>,
    {
        let err_fn = |e| super::log_audio_warn(format!("output stream error: {e}"));
        dev.build_output_stream(
            cfg,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                if let Ok(mut q) = buf.try_lock() {
                    for slot in data.iter_mut() {
                        let v = q.pop_front().unwrap_or(0.0);
                        *slot = T::from_sample(v);
                    }
                } else {
                    for slot in data.iter_mut() {
                        *slot = T::from_sample(0.0f32);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("build output stream: {e}"))
    }

    fn build_input_stream(
        dev: &cpal::Device,
        cfg: &cpal::StreamConfig,
        fmt: cpal::SampleFormat,
        in_rate: u32,
        in_channels: u16,
        cap_tx: mpsc::Sender<Vec<i16>>,
    ) -> Result<cpal::Stream, String> {
        match fmt {
            cpal::SampleFormat::F32 => {
                build_input_typed::<f32>(dev, cfg, in_rate, in_channels, cap_tx)
            }
            cpal::SampleFormat::I16 => {
                build_input_typed::<i16>(dev, cfg, in_rate, in_channels, cap_tx)
            }
            cpal::SampleFormat::U16 => {
                build_input_typed::<u16>(dev, cfg, in_rate, in_channels, cap_tx)
            }
            other => Err(format!("unsupported input format {other:?}")),
        }
    }

    fn build_input_typed<T>(
        dev: &cpal::Device,
        cfg: &cpal::StreamConfig,
        in_rate: u32,
        in_channels: u16,
        cap_tx: mpsc::Sender<Vec<i16>>,
    ) -> Result<cpal::Stream, String>
    where
        T: cpal::SizedSample,
        f32: FromSample<T>,
    {
        let err_fn = |e| super::log_audio_warn(format!("input stream error: {e}"));
        let mut resampler = LinearResampler::new(in_rate, ENGINE_RATE);
        let mut mono_in: Vec<f32> = Vec::with_capacity(2048);
        let mut resampled: Vec<f32> = Vec::with_capacity(2048);
        let mut frame: Vec<i16> = Vec::with_capacity(FRAME_SAMPLES);
        let channels = in_channels.max(1) as usize;
        dev.build_input_stream(
            cfg,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // Down-mix interleaved device frames to mono f32.
                mono_in.clear();
                for chunk in data.chunks(channels) {
                    let mut sum = 0.0f32;
                    for &s in chunk {
                        sum += f32::from_sample(s);
                    }
                    mono_in.push(sum / channels as f32);
                }
                resampled.clear();
                resampler.process(&mono_in, &mut resampled);
                for v in &resampled {
                    frame.push(f32_to_i16(*v));
                    if frame.len() == FRAME_SAMPLES {
                        let _ = cap_tx.try_send(std::mem::take(&mut frame));
                        frame.reserve(FRAME_SAMPLES);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("build input stream: {e}"))
    }
}

fn log_audio_info(msg: String) {
    crate::p2p::native_log::info("call_audio", &msg);
}

fn log_audio_warn(msg: String) {
    crate::p2p::native_log::warn("call_audio", &msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_unity_passthrough() {
        let mut r = LinearResampler::new(48_000, 48_000);
        let input: Vec<f32> = (0..100).map(|i| (i as f32) / 100.0).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        // Unity ratio drops only the first priming sample.
        assert!(out.len() >= input.len() - 2);
    }

    #[test]
    fn resampler_upsample_produces_more() {
        let mut r = LinearResampler::new(24_000, 48_000);
        let input: Vec<f32> = (0..100).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert!(out.len() > input.len(), "upsample should grow sample count");
    }

    #[tokio::test]
    async fn silence_backend_emits_frames() {
        let mut b = SilenceAudioBackend::new();
        let mut s = b.start().unwrap();
        let f = tokio::time::timeout(Duration::from_millis(200), s.capture_rx.recv())
            .await
            .expect("capture frame within 200ms")
            .expect("frame present");
        assert_eq!(f.len(), FRAME_SAMPLES);
        b.stop();
    }
}
