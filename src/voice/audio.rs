use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Records audio from the default input device into an in-memory buffer.
///
/// Call `start()` to begin capture and `stop()` to finalise. The recorded
/// samples are encoded as a WAV file in memory and returned from `stop()`.
pub struct AudioCapture {
    sample_rate: u32,
    channels: u16,
    buffer: Arc<Mutex<Vec<f32>>>,
    stream: Option<cpal::Stream>,
}

impl AudioCapture {
    pub fn new() -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no input device found"))?;

        let config = device.default_input_config()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();

        Ok(Self {
            sample_rate,
            channels,
            buffer: Arc::new(Mutex::new(Vec::new())),
            stream: None,
        })
    }

    /// Begin recording. Safe to call while already recording (no-op).
    pub fn start(&mut self) -> anyhow::Result<()> {
        if self.stream.is_some() {
            return Ok(());
        }

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no input device"))?;
        let config = device.default_input_config()?;

        let buf = Arc::clone(&self.buffer);

        let stream = device.build_input_stream(
            &config.into(),
            move |data: &[f32], _| {
                if let Ok(mut b) = buf.lock() {
                    b.extend_from_slice(data);
                }
            },
            |e| tracing::error!("audio stream error: {e}"),
            None,
        )?;
        stream.play()?;
        self.stream = Some(stream);
        Ok(())
    }

    /// Stop recording and return the captured audio encoded as WAV bytes.
    pub fn stop(&mut self) -> anyhow::Result<Vec<u8>> {
        self.stream.take(); // drops stream, stopping capture

        let samples = self
            .buffer
            .lock()
            .map(|mut b| std::mem::take(&mut *b))
            .unwrap_or_default();

        encode_wav(&samples, self.sample_rate, self.channels)
    }
}

impl Default for AudioCapture {
    fn default() -> Self {
        Self::new().expect("AudioCapture::default failed — check input device")
    }
}

/// Encode f32 samples to a PCM WAV file in memory using `hound`.
fn encode_wav(samples: &[f32], sample_rate: u32, channels: u16) -> anyhow::Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for &s in samples {
            writer.write_sample(s)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}
