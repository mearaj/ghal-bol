//! Echo cancellation for call audio. Sonora processes 10 ms frames. The call
//! engine uses 20 ms frames, so each frame is two chunks.

use sonora::config::EchoCanceller;
use sonora::{AudioProcessing, Config, StreamConfig};

use super::SAMPLE_RATE_HZ;

const CHUNK: usize = (SAMPLE_RATE_HZ as usize) / 100;

pub struct CallAec {
    apm: AudioProcessing,
    render: Vec<f32>,
    capture: Vec<f32>,
    render_out: Vec<f32>,
    capture_out: Vec<f32>,
    cleaned: Vec<i16>,
}

impl CallAec {
    pub fn new() -> Self {
        let stream = StreamConfig::new(SAMPLE_RATE_HZ, 1);
        let config = Config {
            echo_canceller: Some(EchoCanceller::default()),
            ..Config::default()
        };
        Self {
            apm: AudioProcessing::builder()
                .config(config)
                .capture_config(stream)
                .render_config(stream)
                .build(),
            render: vec![0.0; CHUNK],
            capture: vec![0.0; CHUNK],
            render_out: vec![0.0; CHUNK],
            capture_out: vec![0.0; CHUNK],
            cleaned: Vec::new(),
        }
    }

    /// Tell the canceller what the speaker just played.
    pub fn observe_playout(&mut self, pcm: &[i16]) {
        self.chunks(pcm, true);
    }

    /// Remove speaker echo from the microphone frame. Returns the original
    /// samples when the frame is not a whole number of 10 ms chunks.
    pub fn clean_capture(&mut self, pcm: &[i16]) -> Vec<i16> {
        if pcm.is_empty() || !pcm.len().is_multiple_of(CHUNK) {
            return pcm.to_vec();
        }
        self.cleaned.clear();
        self.cleaned.reserve(pcm.len());
        self.chunks(pcm, false);
        std::mem::take(&mut self.cleaned)
    }

    fn chunks(&mut self, pcm: &[i16], render: bool) {
        for chunk in pcm.chunks_exact(CHUNK) {
            for (dst, sample) in self.capture.iter_mut().zip(chunk) {
                *dst = *sample as f32 / 32768.0;
            }
            if render {
                self.render.copy_from_slice(&self.capture);
                let _ = self
                    .apm
                    .process_render_f32(&[&self.render], &mut [&mut self.render_out]);
            } else if self
                .apm
                .process_capture_f32(&[&self.capture], &mut [&mut self.capture_out])
                .is_ok()
            {
                for sample in &self.capture_out {
                    let scaled = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                    self.cleaned.push(scaled);
                }
            } else {
                self.cleaned.extend_from_slice(chunk);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(pcm: &[i16]) -> f64 {
        if pcm.is_empty() {
            return 0.0;
        }
        let energy: f64 = pcm.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (energy / pcm.len() as f64).sqrt()
    }

    #[test]
    fn echo_of_the_speaker_is_reduced() {
        let mut aec = CallAec::new();
        let tone: Vec<i16> = (0..CHUNK * 2)
            .map(|i| ((i as f32 * 0.07).sin() * 12000.0) as i16)
            .collect();
        let silence = vec![0i16; tone.len()];
        let mut last = tone.clone();
        let mut previous = silence.clone();
        for _ in 0..200 {
            aec.observe_playout(&tone);
            last = aec.clean_capture(&previous);
            previous = tone.clone();
        }
        assert!(
            rms(&last) < rms(&tone) * 0.5,
            "echo rms {} stayed near speaker rms {}",
            rms(&last),
            rms(&tone)
        );
    }

    #[test]
    fn silence_stays_quiet() {
        let mut aec = CallAec::new();
        let zeros = vec![0i16; CHUNK * 2];
        aec.observe_playout(&zeros);
        let out = aec.clean_capture(&zeros);
        assert!(rms(&out) < 50.0);
    }
}
