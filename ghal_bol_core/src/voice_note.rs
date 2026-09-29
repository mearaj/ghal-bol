//! Voice-note capture and playback. A second tap on the same note stops it.
//!
//! PCM is mono 48 kHz, then packed with [`crate::voice_msg_v1`] (≤ 120 s).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample};

use crate::call_media::SAMPLE_RATE_HZ;
use crate::voice_msg_v1::{self, VOICE_MAX_DURATION_MS, decode_opus_blob_to_pcm, encode_pcm_to_opus_blob};

struct Recording {
    stop: mpsc::Sender<()>,
    thread: thread::JoinHandle<()>,
    pcm: Arc<Mutex<Vec<i16>>>,
    started: Instant,
}

fn recording_slot() -> &'static Mutex<Option<Recording>> {
    static SLOT: OnceLock<Mutex<Option<Recording>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

pub fn voice_note_recording() -> bool {
    recording_slot().lock().ok().is_some_and(|g| g.is_some())
}

/// Start a note, or stop the current one and return Opus plus duration.
pub fn voice_note_toggle() -> Result<Option<(u32, Vec<u8>)>, String> {
    let mut slot = recording_slot()
        .lock()
        .map_err(|_| "voice note lock".to_string())?;
    if let Some(active) = slot.take() {
        let _ = active.stop.send(());
        let _ = active.thread.join();
        let pcm = active
            .pcm
            .lock()
            .map_err(|_| "pcm lock".to_string())?
            .clone();
        let elapsed = active.started.elapsed().as_millis() as u32;
        let duration_ms = voice_msg_v1::duration_ms_from_pcm_len(pcm.len()).min(elapsed.max(1));
        if duration_ms == 0 || pcm.is_empty() {
            return Err("recording is empty".into());
        }
        let opus = encode_pcm_to_opus_blob(&pcm)?;
        return Ok(Some((duration_ms.max(1), opus)));
    }
    let pcm = Arc::new(Mutex::new(Vec::<i16>::new()));
    let pcm_thread = Arc::clone(&pcm);
    let (stop_tx, stop_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread = thread::spawn(move || {
        match start_input(pcm_thread) {
            Ok(stream) => {
                if let Err(e) = stream.play() {
                    let _ = ready_tx.send(Err(format!("mic start: {e}")));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                let _ = stop_rx.recv();
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        }
    });
    match ready_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = thread.join();
            return Err(e);
        }
        Err(_) => return Err("microphone thread stopped".into()),
    }
    *slot = Some(Recording {
        stop: stop_tx,
        thread,
        pcm,
        started: Instant::now(),
    });
    Ok(None)
}

fn play_stop() -> &'static AtomicBool {
    static STOP: AtomicBool = AtomicBool::new(false);
    &STOP
}

fn playing_path() -> &'static Mutex<Option<String>> {
    static PATH: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    PATH.get_or_init(|| Mutex::new(None))
}

pub fn play_opus_file(path: &str) -> Result<bool, String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("missing voice note".into());
    }
    if let Ok(mut current) = playing_path().lock() {
        if current.as_deref() == Some(path.as_str()) {
            play_stop().store(true, Ordering::Relaxed);
            *current = None;
            return Ok(false);
        }
        *current = Some(path.clone());
    }
    play_stop().store(false, Ordering::Relaxed);
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let pcm = decode_opus_blob_to_pcm(&bytes)?;
    let samples = pcm.len();
    thread::spawn(move || {
        if let Ok(stream) = start_output(pcm) {
            let _ = stream.play();
            let frames = samples / SAMPLE_RATE_HZ.max(1) as usize;
            for _ in 0..(frames.saturating_mul(10).saturating_add(2)) {
                if play_stop().load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(100));
            }
            drop(stream);
        }
        if let Ok(mut current) = playing_path().lock() {
            if current.as_deref() == Some(path.as_str()) {
                *current = None;
            }
        }
    });
    Ok(true)
}

fn start_input(pcm: Arc<Mutex<Vec<i16>>>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no microphone".to_string())?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("mic config: {e}"))?;
    let cfg = supported.config();
    let rate = cfg.sample_rate.0;
    let channels = cfg.channels.max(1);
    let max_samples = (u64::from(VOICE_MAX_DURATION_MS) * u64::from(SAMPLE_RATE_HZ) / 1000) as usize;
    match supported.sample_format() {
        cpal::SampleFormat::F32 => input_typed::<f32>(&device, &cfg, rate, channels, pcm, max_samples),
        cpal::SampleFormat::I16 => input_typed::<i16>(&device, &cfg, rate, channels, pcm, max_samples),
        cpal::SampleFormat::U16 => input_typed::<u16>(&device, &cfg, rate, channels, pcm, max_samples),
        other => Err(format!("unsupported mic format {other:?}")),
    }
}

fn input_typed<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    rate: u32,
    channels: u16,
    pcm: Arc<Mutex<Vec<i16>>>,
    max_samples: usize,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: FromSample<T>,
{
    let channels = channels.max(1) as usize;
    let mut pos = 0.0f64;
    let step = f64::from(rate) / f64::from(SAMPLE_RATE_HZ);
    device
        .build_input_stream(
            cfg,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut guard = match pcm.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                if guard.len() >= max_samples {
                    return;
                }
                let frames = data.len() / channels;
                for i in 0..frames {
                    let mut sum = 0.0f32;
                    for c in 0..channels {
                        sum += f32::from_sample(data[i * channels + c]);
                    }
                    let sample = sum / channels as f32;
                    pos += 1.0;
                    if pos >= step {
                        pos -= step;
                        let v = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        guard.push(v);
                        if guard.len() >= max_samples {
                            break;
                        }
                    }
                }
            },
            |e| eprintln!("ghal_bol voice note mic: {e}"),
            None,
        )
        .map_err(|e| format!("mic stream: {e}"))
}

fn start_output(pcm: Vec<i16>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no speaker".to_string())?;
    let supported = device
        .default_output_config()
        .map_err(|e| format!("speaker config: {e}"))?;
    let cfg = supported.config();
    let rate = cfg.sample_rate.0;
    let channels = cfg.channels.max(1) as usize;
    match supported.sample_format() {
        cpal::SampleFormat::F32 => output_typed::<f32>(&device, &cfg, rate, channels, pcm),
        cpal::SampleFormat::I16 => output_typed::<i16>(&device, &cfg, rate, channels, pcm),
        cpal::SampleFormat::U16 => output_typed::<u16>(&device, &cfg, rate, channels, pcm),
        other => Err(format!("unsupported speaker format {other:?}")),
    }
}

fn output_typed<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    rate: u32,
    channels: usize,
    pcm: Vec<i16>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + FromSample<f32>,
{
    let mut index = 0.0f64;
    let step = f64::from(SAMPLE_RATE_HZ) / f64::from(rate.max(1));
    device
        .build_output_stream(
            cfg,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    let sample = if (index as usize) < pcm.len() {
                        pcm[index as usize] as f32 / i16::MAX as f32
                    } else {
                        0.0
                    };
                    index += step;
                    let out = T::from_sample(sample);
                    for slot in frame.iter_mut() {
                        *slot = out;
                    }
                }
            },
            |e| eprintln!("ghal_bol voice note speaker: {e}"),
            None,
        )
        .map_err(|e| format!("speaker stream: {e}"))
}
