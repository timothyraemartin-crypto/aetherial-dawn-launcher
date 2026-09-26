//! Quiet menu music (Timothy, 2026-09-26): Skyrim's main theme, played from
//! the player's own game archives with Windows' XAudio2 (which plays xWMA
//! natively). Nothing is copied out of the game folder. It fades in, loops,
//! and stops when the game starts or the player mutes it.

use std::path::PathBuf;
use std::sync::mpsc;

use crate::log;

/// How loud the music plays (XAudio2 volume, 1.0 = full).
pub const VOLUME: f32 = 0.12;

pub enum Cmd {
    Play(PathBuf),
    Stop,
}

pub struct Music {
    tx: std::sync::Mutex<Option<mpsc::Sender<Cmd>>>,
}

impl Music {
    pub fn new() -> Self {
        Self { tx: std::sync::Mutex::new(None) }
    }

    fn send(&self, c: Cmd) {
        let mut tx = self.tx.lock().unwrap();
        if tx.is_none() {
            let (t, r) = mpsc::channel();
            std::thread::spawn(move || player(r));
            *tx = Some(t);
        }
        if tx.as_ref().unwrap().send(c).is_err() {
            *tx = None;
        }
    }

    pub fn play(&self, game_dir: PathBuf) {
        self.send(Cmd::Play(game_dir));
    }

    pub fn stop(&self) {
        if self.tx.lock().unwrap().is_some() {
            self.send(Cmd::Stop);
        }
    }
}

#[cfg(windows)]
fn player(rx: mpsc::Receiver<Cmd>) {
    use windows::Win32::Media::Audio::XAudio2::*;
    use windows::Win32::Media::Audio::{AudioCategory_GameMedia, WAVEFORMATEX};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    struct Playing {
        voice: IXAudio2SourceVoice,
        // Kept alive while XAudio2 reads them.
        _track: launcher_core::bsa::Xwma,
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut engine: Option<(IXAudio2, IXAudio2MasteringVoice)> = None;
    let mut playing: Option<Playing> = None;

    let fade = |v: &IXAudio2SourceVoice, from: f32, to: f32, ms: u32| {
        let steps = (ms / 50).max(1);
        for i in 1..=steps {
            let x = from + (to - from) * i as f32 / steps as f32;
            unsafe {
                let _ = v.SetVolume(x, XAUDIO2_COMMIT_NOW);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    };
    let stop = |p: Option<Playing>| {
        if let Some(p) = p {
            fade(&p.voice, VOLUME, 0.0, 1000);
            unsafe {
                let _ = p.voice.Stop(0, XAUDIO2_COMMIT_NOW);
                p.voice.DestroyVoice();
            }
        }
    };

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Stop => stop(playing.take()),
            Cmd::Play(dir) => {
                if playing.is_some() {
                    continue;
                }
                let track = match launcher_core::bsa::menu_music(&dir) {
                    Ok(t) => t,
                    Err(e) => {
                        log::line(&format!("music: no menu music ({e})"));
                        continue;
                    }
                };
                if engine.is_none() {
                    let mut x: Option<IXAudio2> = None;
                    let made = unsafe { XAudio2CreateWithVersionInfo(&mut x, 0, XAUDIO2_DEFAULT_PROCESSOR, 0x0A00_0000) };
                    let Some(x) = x.filter(|_| made.is_ok()) else {
                        log::line("music: Windows audio (XAudio2) isn't available");
                        continue;
                    };
                    let mut m: Option<IXAudio2MasteringVoice> = None;
                    let ok = unsafe { x.CreateMasteringVoice(&mut m, 0, 0, 0, windows::core::PCWSTR::null(), None, AudioCategory_GameMedia) };
                    let Some(m) = m.filter(|_| ok.is_ok()) else {
                        log::line("music: no sound device");
                        continue;
                    };
                    engine = Some((x, m));
                }
                let (x, _) = engine.as_ref().unwrap();
                let mut voice: Option<IXAudio2SourceVoice> = None;
                let fmt = track.format.as_ptr() as *const WAVEFORMATEX;
                let made = unsafe { x.CreateSourceVoice(&mut voice, fmt, 0, XAUDIO2_DEFAULT_FREQ_RATIO, None, None, None) };
                let Some(voice) = voice.filter(|_| made.is_ok()) else {
                    log::line("music: couldn't play Skyrim's music format");
                    continue;
                };
                let buffer = XAUDIO2_BUFFER {
                    Flags: XAUDIO2_END_OF_STREAM,
                    AudioBytes: track.data.len() as u32,
                    pAudioData: track.data.as_ptr(),
                    LoopCount: XAUDIO2_LOOP_INFINITE,
                    ..Default::default()
                };
                let wma = XAUDIO2_BUFFER_WMA { pDecodedPacketCumulativeBytes: track.dpds.as_ptr(), PacketCount: track.dpds.len() as u32 };
                let started = unsafe {
                    voice
                        .SetVolume(0.0, XAUDIO2_COMMIT_NOW)
                        .and_then(|_| voice.SubmitSourceBuffer(&buffer, Some(&wma)))
                        .and_then(|_| voice.Start(0, XAUDIO2_COMMIT_NOW))
                };
                if let Err(e) = started {
                    log::line(&format!("music: couldn't start ({e})"));
                    unsafe { voice.DestroyVoice() };
                    continue;
                }
                log::line("music: playing Skyrim's main theme quietly");
                fade(&voice, 0.0, VOLUME, 3000);
                playing = Some(Playing { voice, _track: track });
            }
        }
    }
    stop(playing.take());
}

#[cfg(not(windows))]
fn player(rx: mpsc::Receiver<Cmd>) {
    while rx.recv().is_ok() {}
}
