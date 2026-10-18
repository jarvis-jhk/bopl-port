//! The original's audio manager on top of our mixer: sounds by logical name
//! (`globals.json` `AudioManager.sounds`, clips from the import) and songs
//! (intro once, then the loop part scheduled to start when the intro ends).
//!
//! Rules from `docs/spec/audio.md`: one voice per sound name (playing it
//! again restarts it), effect volume = entry volume × SFX volume / 10, music
//! volume = music volume / 10 × 0.1875, engine volume 0.5 on top. The
//! settings act through the mixer's buses (SFX bus = SFX / 10, music bus =
//! the current music volume, master = engine volume × master / 10), so a
//! change applies at once to what is playing. Loops follow
//! `PlayOrContinueLoop`: wanted each tick or stopped (fading entries fade
//! in and out at their volume per second); music ducking follows
//! `FadeOutMusicTick`.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use bopl_import::GameData;
use serde_json::Value as J;

use crate::audio::{Audio, Bus, PlayParams, SoundData, VoiceId, decode_ogg_file};

/// Unity's global audio volume in the original.
pub const ENGINE_VOLUME: f32 = 0.5;
/// Factor of the music volume (audio.md 1).
pub const MUSIC_FACTOR: f32 = 0.1875;

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    clip: String,
    volume: f32,
    pitch: f32,
    looping: bool,
    /// `fadeLoop`: the loop fades in and out.
    fade: bool,
}

/// A looping entry that plays (`PlayOrContinueLoop`).
#[derive(Clone, Copy, Debug)]
struct LoopVoice {
    voice: VoiceId,
    /// Current voice volume (fading entries ramp it).
    volume: f32,
}

/// `AudioManager.MusicRecoveryPerSecond01`.
const MUSIC_RECOVERY: f32 = 0.3;

/// A song: intro clip, loop clip, and when the loop starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub name: String,
    intro: Option<String>,
    looped: Option<String>,
    /// Seconds after which the loop starts (0: the intro clip's length).
    intro_length: f32,
}

/// Sounds and music of the original, by name.
pub struct Sounds {
    data: Rc<GameData>,
    entries: HashMap<String, Entry>,
    pub songs: Vec<Song>,
    pub menu_song: Option<Song>,
    cache: HashMap<String, Arc<SoundData>>,
    /// name -> (voice, start time, clip length)
    playing: HashMap<String, (VoiceId, f64, f64)>,
    music: Vec<VoiceId>,
    /// Presentation clock (seconds), advanced by [`Sounds::advance`].
    now: f64,
    /// Current song: index into `songs` (`None`: menu song or silence),
    /// start time and intro + loop length.
    pub song: Option<usize>,
    song_started: f64,
    song_length: f64,
    rng: u32,
    /// Settings 0..10 (the original's sliders; master is ours, the
    /// original's master volume is fixed at 1).
    pub sfx_volume: f32,
    pub music_volume: f32,
    pub master_volume: f32,
    /// Looping entries playing, and the ones wanted this tick.
    loops: HashMap<String, LoopVoice>,
    wanted: BTreeSet<String>,
    /// Current music volume (0..music / 10; ducked by `FadeOutMusicTick`).
    current_music: f32,
    /// Strongest music fade asked for this tick.
    music_fade: f32,
    /// Bus / master volumes last sent to the mixer.
    sent: Option<[f32; 3]>,
}

fn clip_ref(j: &J) -> Option<String> {
    j["$ref"].as_str().map(str::to_string)
}

fn song(j: &J) -> Option<Song> {
    Some(Song {
        name: j["name"].as_str()?.to_string(),
        intro: clip_ref(&j["intro"]),
        looped: clip_ref(&j["songLoop"]),
        intro_length: j["introLength"].as_f64().unwrap_or(0.0) as f32,
    })
}

impl Sounds {
    pub fn new(data: Rc<GameData>) -> Self {
        let am = data
            .globals
            .components
            .get("AudioManager")
            .map(|c| c.fields.clone())
            .unwrap_or(J::Null);
        let mut entries = HashMap::new();
        for s in am["sounds"].as_array().into_iter().flatten() {
            let (Some(name), Some(clip)) = (s["name"].as_str(), clip_ref(&s["clip"])) else {
                continue;
            };
            entries.insert(
                name.to_string(),
                Entry {
                    clip,
                    volume: s["volume"].as_f64().unwrap_or(1.0) as f32,
                    pitch: s["pitch"].as_f64().unwrap_or(1.0) as f32,
                    looping: s["loop"].as_i64() == Some(1),
                    fade: s["fadeLoop"].as_i64() == Some(1),
                },
            );
        }
        let songs = am["songs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(song)
            .collect();
        let menu_song = song(&am["menuSong"]);
        Self {
            data,
            entries,
            songs,
            menu_song,
            cache: HashMap::new(),
            playing: HashMap::new(),
            music: Vec::new(),
            now: 0.0,
            song: None,
            song_started: 0.0,
            song_length: 0.0,
            rng: 0x2545_f491,
            sfx_volume: 10.0,
            music_volume: 10.0,
            master_volume: 10.0,
            loops: HashMap::new(),
            wanted: BTreeSet::new(),
            current_music: 1.0,
            music_fade: 0.0,
            sent: None,
        }
    }

    /// Sends the bus and master volumes to the mixer when they changed
    /// (settings, ducking).
    pub fn apply_volumes(&mut self, audio: &mut Audio) {
        let v = [
            (self.sfx_volume / 10.0).clamp(0.0, 1.0),
            self.current_music
                .min(self.music_volume / 10.0)
                .clamp(0.0, 1.0),
            ENGINE_VOLUME * (self.master_volume / 10.0).clamp(0.0, 1.0),
        ];
        if self.sent != Some(v) {
            audio.set_bus_volume(Bus::Sfx, v[0]);
            audio.set_bus_volume(Bus::Music, v[1]);
            audio.set_master_volume(v[2]);
            self.sent = Some(v);
        }
    }

    /// `PlayOrContinueLoop(name)`: the looping entry plays while it is
    /// asked for every tick.
    pub fn want_loop(&mut self, name: &str) {
        if !self.wanted.contains(name) {
            self.wanted.insert(name.to_string());
        }
    }

    /// `FadeOutMusicTick(strength)`: the music fades by `strength` per
    /// second while asked for every tick.
    pub fn fade_music(&mut self, strength: f32) {
        self.music_fade = self.music_fade.max(strength);
    }

    /// Stops every loop at once (leaving a match).
    pub fn stop_loops(&mut self, audio: &mut Audio) {
        for (_, l) in self.loops.drain() {
            audio.stop(l.voice);
        }
        self.wanted.clear();
    }

    /// Looping voices playing (tests, leak checks).
    pub fn loops_playing(&self) -> usize {
        self.loops.len()
    }

    /// End of a tick (`LateUpdate`): starts / continues / fades / stops
    /// the loops, ducks or recovers the music, applies the volumes.
    pub fn update(&mut self, audio: &mut Audio, dt: f32) {
        let wanted = std::mem::take(&mut self.wanted);
        let names: BTreeSet<String> = self
            .loops
            .keys()
            .cloned()
            .chain(wanted.iter().cloned())
            .collect();
        for name in names {
            let Some(e) = self.entries.get(&name).cloned() else {
                if wanted.contains(&name) {
                    log::warn!("no sound named {name:?}");
                }
                continue;
            };
            let want = wanted.contains(&name) && e.looping;
            match (self.loops.get(&name).copied(), want) {
                (None, true) => {
                    let Some(clip) = self.clip(&e.clip) else {
                        continue;
                    };
                    // A fading entry fades in from silence.
                    let volume = if e.fade { e.volume * dt } else { e.volume };
                    let voice = audio.play(
                        &clip,
                        PlayParams {
                            volume,
                            pitch: e.pitch,
                            looping: true,
                            bus: Bus::Sfx,
                            ..Default::default()
                        },
                    );
                    log::debug!("loop {name} starts");
                    self.loops.insert(name, LoopVoice { voice, volume });
                }
                (Some(mut l), true) => {
                    if e.fade && l.volume < e.volume {
                        l.volume = (l.volume + e.volume * dt).min(e.volume);
                        audio.set_volume(l.voice, l.volume);
                        self.loops.insert(name, l);
                    }
                }
                (Some(mut l), false) => {
                    if e.fade && l.volume > 0.0 {
                        l.volume = (l.volume - e.volume * dt).max(0.0);
                        audio.set_volume(l.voice, l.volume);
                        self.loops.insert(name, l);
                    } else {
                        log::debug!("loop {name} stops");
                        audio.stop(l.voice);
                        self.loops.remove(&name);
                    }
                }
                (None, false) => {}
            }
        }
        // Music: ducked while asked for, else recovering (audio.md 2).
        let full = (self.music_volume / 10.0).clamp(0.0, 1.0);
        if self.music_fade == 0.0 {
            self.current_music += (full - self.current_music) * (MUSIC_RECOVERY * dt).min(1.0);
            if (full - self.current_music).abs() < 1e-4 {
                self.current_music = full;
            }
        } else {
            self.current_music = (self.current_music - self.music_fade * dt).clamp(0.0, full);
            self.music_fade = 0.0;
        }
        self.current_music = self.current_music.min(full);
        self.apply_volumes(audio);
    }

    /// A new audio device (the old one went away): volumes again, the
    /// current song from its start, loops restart when asked for.
    pub fn reopened(&mut self, audio: &mut Audio) {
        self.sent = None;
        self.loops.clear();
        self.playing.clear();
        self.music.clear();
        self.apply_volumes(audio);
        let song = match self.song {
            Some(i) => self.match_song(i),
            None => self.menu_song.clone(),
        };
        if let Some(s) = song {
            let keep = self.song;
            self.play_song(audio, &s);
            self.song = keep;
        }
    }

    /// The music volume setting changed: the music follows at once.
    pub fn set_music_volume(&mut self, audio: &mut Audio, v: f32) {
        self.music_volume = v;
        self.current_music = (v / 10.0).clamp(0.0, 1.0);
        self.apply_volumes(audio);
    }

    /// Sound names (sorted).
    pub fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.entries.keys().map(String::as_str).collect();
        v.sort();
        v
    }

    fn clip(&mut self, id: &str) -> Option<Arc<SoundData>> {
        if let Some(c) = self.cache.get(id) {
            return Some(c.clone());
        }
        let path = self.data.audio_path(id)?;
        match decode_ogg_file(&path) {
            Ok(s) => {
                let s = Arc::new(s);
                self.cache.insert(id.to_string(), s.clone());
                Some(s)
            }
            Err(e) => {
                log::warn!("{e:#}");
                None
            }
        }
    }

    /// Plays the sound `name` (restarting it if it is playing). Unknown
    /// names only log, like the original.
    pub fn play(&mut self, audio: &mut Audio, name: &str) -> Option<VoiceId> {
        let Some(e) = self.entries.get(name).cloned() else {
            log::warn!("no sound named {name:?}");
            return None;
        };
        let clip = self.clip(&e.clip)?;
        if let Some((v, ..)) = self.playing.remove(name) {
            audio.stop(v);
        }
        let length = clip.duration() / e.pitch.max(0.01) as f64;
        let v = audio.play(
            &clip,
            PlayParams {
                volume: e.volume,
                pitch: e.pitch,
                looping: e.looping,
                bus: Bus::Sfx,
                ..Default::default()
            },
        );
        self.playing.insert(name.to_string(), (v, self.now, length));
        Some(v)
    }

    /// Advances the presentation clock (used to know which sounds still
    /// play and how long the song has played).
    pub fn advance(&mut self, dt: f64) {
        self.now += dt;
    }

    fn is_playing(&self, name: &str) -> bool {
        self.playing
            .get(name)
            .is_some_and(|(_, t, len)| self.now - t < *len)
    }

    fn next_random(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// `PlayRandomWithPrefix` / `PlayWithPrefixWithInterrupts` (audio.md 1).
    pub fn play_prefix(&mut self, audio: &mut Audio, prefix: &str) -> Option<VoiceId> {
        let mut names: Vec<String> = self
            .entries
            .iter()
            .filter(|(n, e)| n.starts_with(prefix) && !e.looping)
            .map(|(n, _)| n.clone())
            .collect();
        names.sort();
        if names.is_empty() {
            log::warn!("no sound with prefix {prefix:?}");
            return None;
        }
        let pick = if prefix == "easeInBlock" {
            // With interrupts: the one with the least playback progress.
            names
                .iter()
                .min_by(|a, b| {
                    // Stopped sources report progress 0.
                    let prog = |n: &String| {
                        self.playing.get(n).map_or(0.0, |(_, t, len)| {
                            let p = (self.now - t) / len.max(1e-6);
                            if p >= 1.0 { 0.0 } else { p }
                        })
                    };
                    prog(a).total_cmp(&prog(b))
                })
                .cloned()
        } else {
            let free: Vec<String> = names
                .iter()
                .filter(|n| !self.is_playing(n))
                .cloned()
                .collect();
            if free.is_empty() {
                None
            } else {
                // QUIRK: the upper bound excludes the last candidate.
                let k = free.len().saturating_sub(1).max(1);
                let i = (self.next_random() as usize) % k;
                Some(free[i].clone())
            }
        }?;
        self.play(audio, &pick)
    }

    /// Menu music (on entering menus from a match or at start).
    pub fn play_menu_music(&mut self, audio: &mut Audio) {
        if let Some(s) = self.menu_song.clone() {
            self.play_song(audio, &s);
        }
        self.song = None;
    }

    /// "Match started" (audio.md 2): from menus song 0; otherwise keep the
    /// current song unless it played longer than intro + one loop, then the
    /// next one.
    pub fn match_started(&mut self, audio: &mut Audio) {
        let next = match self.song {
            None => Some(0),
            Some(i) if self.now - self.song_started > self.song_length => Some(i + 1),
            Some(_) => None,
        };
        if let Some(i) = next
            && let Some(song) = self.match_song(i)
        {
            self.play_song(audio, &song);
            self.song = Some(i % self.songs.len().max(1));
        }
        self.set_music_pitch(audio, 1.0);
    }

    /// "Match ended": a draw stops the music and counts the song as done.
    pub fn match_ended(&mut self, audio: &mut Audio, draw: bool) {
        self.set_music_pitch(audio, 1.0);
        if draw {
            self.stop_music(audio);
            self.song_length = 0.0;
        }
    }

    /// Sudden death: music pitch + 0.1.
    pub fn set_music_pitch(&mut self, audio: &mut Audio, pitch: f32) {
        for v in &self.music {
            audio.set_pitch(*v, pitch);
        }
    }

    /// Starts a song from the beginning (stopping the current music).
    pub fn play_song(&mut self, audio: &mut Audio, song: &Song) {
        self.stop_music(audio);
        self.apply_volumes(audio);
        let volume = MUSIC_FACTOR;
        let intro = song.intro.as_deref().and_then(|id| self.clip(id));
        let looped = song.looped.as_deref().and_then(|id| self.clip(id));
        let mut delay = 0.0;
        self.song_started = self.now;
        if let Some(i) = &intro {
            self.music.push(audio.play(
                i,
                PlayParams {
                    volume,
                    bus: Bus::Music,
                    ..Default::default()
                },
            ));
            delay = if song.intro_length > 0.0 {
                song.intro_length as f64
            } else {
                i.duration()
            };
        }
        self.song_length = delay + looped.as_ref().map_or(0.0, |l| l.duration());
        if let Some(l) = &looped {
            self.music.push(audio.play(
                l,
                PlayParams {
                    volume,
                    looping: true,
                    bus: Bus::Music,
                    delay,
                    ..Default::default()
                },
            ));
        }
    }

    /// The match song with this index (0 = TheBopl, the first of a session).
    pub fn match_song(&self, i: usize) -> Option<Song> {
        if self.songs.is_empty() {
            return None;
        }
        Some(self.songs[i % self.songs.len()].clone())
    }

    pub fn stop_music(&mut self, audio: &mut Audio) {
        for v in self.music.drain(..) {
            audio.stop(v);
        }
    }
}
