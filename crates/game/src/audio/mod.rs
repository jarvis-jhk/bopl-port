//! Audio: SDL2 output device driven by our own [`Mixer`], or (headless)
//! the same mixer rendered offline into a WAV file.
//!
//! The game talks to [`Audio`] only; commands go over a channel to the SDL
//! audio thread, or straight into the offline mixer.

mod decode;
mod mixer;
mod wav;

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::Result;

pub use decode::decode_ogg_file;
pub use mixer::{Bus, Command, Mixer, PlayParams, SoundData, VoiceId};
pub use wav::WavWriter;

/// Output sample rate requested from the device and used offline.
pub const SAMPLE_RATE: u32 = 48000;

struct DeviceCallback {
    mixer: Mixer,
    rx: Receiver<Command>,
}

impl sdl2::audio::AudioCallback for DeviceCallback {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        while let Ok(cmd) = self.rx.try_recv() {
            self.mixer.apply(cmd);
        }
        self.mixer.mix(out);
    }
}

enum Output {
    /// No audio device: commands are dropped.
    Null,
    Device {
        tx: Sender<Command>,
        _device: sdl2::audio::AudioDevice<DeviceCallback>,
    },
    Offline {
        mixer: Mixer,
        wav: Option<WavWriter>,
        /// Fractional frames carried between `advance_offline` calls.
        carry: f64,
        buf: Vec<f32>,
    },
}

/// The game's audio front end.
#[allow(dead_code)] // Part of the API the game proper (M3+) uses.
pub struct Audio {
    out: Output,
    next_id: u64,
    sample_rate: u32,
    description: String,
}

#[allow(dead_code)] // Part of the API the game proper (M3+) uses.
impl Audio {
    /// Opens the default SDL playback device (stereo f32).
    pub fn open_device(sdl_audio: &sdl2::AudioSubsystem) -> Result<Self> {
        let desired = sdl2::audio::AudioSpecDesired {
            freq: Some(SAMPLE_RATE as i32),
            channels: Some(2),
            samples: Some(512),
        };
        let (tx, rx) = mpsc::channel();
        let device = sdl_audio
            .open_playback(None, &desired, |spec| DeviceCallback {
                mixer: Mixer::new(spec.freq as u32),
                rx,
            })
            .map_err(|e| anyhow::anyhow!("opening audio device: {e}"))?;
        let spec = *device.spec();
        if spec.channels != 2 {
            anyhow::bail!(
                "audio device opened with {} channels, need stereo",
                spec.channels
            );
        }
        device.resume();
        let description = format!(
            "SDL audio ({}), {} Hz, {} ch, {} frames",
            sdl_audio.current_audio_driver(),
            spec.freq,
            spec.channels,
            spec.samples
        );
        log::info!("{description}");
        Ok(Self {
            out: Output::Device {
                tx,
                _device: device,
            },
            next_id: 1,
            sample_rate: spec.freq as u32,
            description,
        })
    }

    /// No sound at all (no playback device): commands are dropped, so no
    /// voices pile up.
    pub fn null() -> Self {
        Self {
            out: Output::Null,
            next_id: 1,
            sample_rate: SAMPLE_RATE,
            description: "none (no audio device)".into(),
        }
    }

    /// A real playback device.
    pub fn is_device(&self) -> bool {
        matches!(self.out, Output::Device { .. })
    }

    /// An offline mixer; with a path, everything mixed is written to a WAV.
    pub fn offline(wav_path: Option<&std::path::Path>) -> Result<Self> {
        let wav = wav_path
            .map(|p| WavWriter::create(p, SAMPLE_RATE))
            .transpose()?;
        let description = match wav_path {
            Some(p) => format!("offline mixer -> {}", p.display()),
            None => "offline mixer (discarded)".into(),
        };
        Ok(Self {
            out: Output::Offline {
                mixer: Mixer::new(SAMPLE_RATE),
                wav,
                carry: 0.0,
                buf: Vec::new(),
            },
            next_id: 1,
            sample_rate: SAMPLE_RATE,
            description,
        })
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn send(&mut self, cmd: Command) {
        match &mut self.out {
            Output::Device { tx, .. } => {
                let _ = tx.send(cmd);
            }
            Output::Offline { mixer, .. } => mixer.apply(cmd),
            Output::Null => {}
        }
    }

    pub fn play(&mut self, sound: &Arc<SoundData>, params: PlayParams) -> VoiceId {
        let id = VoiceId(self.next_id);
        self.next_id += 1;
        self.send(Command::Play {
            id,
            sound: sound.clone(),
            params,
        });
        id
    }

    pub fn stop(&mut self, id: VoiceId) {
        self.send(Command::Stop(id));
    }

    pub fn set_volume(&mut self, id: VoiceId, volume: f32) {
        self.send(Command::SetVolume(id, volume));
    }

    pub fn set_pitch(&mut self, id: VoiceId, pitch: f32) {
        self.send(Command::SetPitch(id, pitch));
    }

    pub fn set_pan(&mut self, id: VoiceId, pan: f32) {
        self.send(Command::SetPan(id, pan));
    }

    pub fn set_bus_volume(&mut self, bus: Bus, volume: f32) {
        self.send(Command::SetBusVolume(bus, volume));
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.send(Command::SetMasterVolume(volume));
    }

    pub fn stop_all(&mut self) {
        self.send(Command::StopAll);
    }

    /// Offline only: mixes `seconds` of audio (appending to the WAV). No-op
    /// for a real device, which pulls audio itself.
    pub fn advance_offline(&mut self, seconds: f64) -> Result<()> {
        if let Output::Offline {
            mixer,
            wav,
            carry,
            buf,
        } = &mut self.out
        {
            let exact = seconds * mixer.sample_rate() as f64 + *carry;
            let frames = exact.floor();
            *carry = exact - frames;
            buf.resize(frames as usize * 2, 0.0);
            mixer.mix(buf);
            if let Some(w) = wav {
                w.write(buf)?;
            }
        }
        Ok(())
    }

    /// Flushes the WAV file, if any.
    pub fn finish(self) -> Result<()> {
        if let Output::Offline { wav: Some(w), .. } = self.out {
            w.finish()?;
        }
        Ok(())
    }
}
