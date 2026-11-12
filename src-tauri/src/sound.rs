//! Generated alarm tones, so there are no audio files to ship or load.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Ordered from quietest to most urgent.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum Sound {
    #[default]
    Off,
    /// Two soft rising notes.
    Chime,
    /// Three short beeps.
    Alert,
    /// Three fast bursts of high beeps: hard to sleep through.
    Urgent,
}

/// (frequency in Hz, length in ms). 0 Hz is a pause.
fn notes(s: Sound) -> Vec<(f32, u64)> {
    match s {
        Sound::Off => vec![],
        Sound::Chime => vec![(880.0, 180), (0.0, 60), (1318.5, 320)],
        Sound::Alert => [(988.0, 150), (0.0, 110)].repeat(3),
        Sound::Urgent => [
            (1568.0, 120),
            (0.0, 50),
            (1568.0, 120),
            (0.0, 50),
            (1568.0, 120),
            (0.0, 350),
        ]
        .repeat(5),
    }
}

/// The tone playing now (0 = none), so repeated alerts don't stack up while a more
/// urgent tone still gets through.
static PLAYING: AtomicU8 = AtomicU8::new(0);
/// The last playback failure, shown in the popup so a broken speaker isn't silent.
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Clears the playing flag even if playback panics.
struct Playing;
impl Drop for Playing {
    fn drop(&mut self) {
        PLAYING.store(0, Ordering::SeqCst);
    }
}

/// Plays a tone on the default output device without blocking. A tone already playing
/// is only joined by a more urgent one.
pub fn play(s: Sound) {
    let level = s as u8;
    if s == Sound::Off || PLAYING.fetch_max(level, Ordering::SeqCst) >= level {
        return;
    }
    std::thread::spawn(move || {
        let _guard = Playing;
        let result = play_blocking(s).map_err(|e| e.to_string());
        if let Err(e) = &result {
            eprintln!("cgm-glance: could not play the alarm sound: {e}");
        }
        *LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()) = result.err();
    });
}

/// Why the last alarm sound failed, if it did.
pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn play_blocking(s: Sound) -> Result<(), rodio::StreamError> {
    use rodio::source::{SineWave, Source, Zero};
    let mut stream = rodio::OutputStreamBuilder::open_default_stream()?;
    stream.log_on_drop(false);
    let sink = rodio::Sink::connect_new(stream.mixer());
    for (freq, ms) in notes(s) {
        let d = Duration::from_millis(ms);
        if freq == 0.0 {
            sink.append(Zero::new(1, 44_100).take_duration(d));
        } else {
            sink.append(
                SineWave::new(freq)
                    .take_duration(d)
                    .fade_in(Duration::from_millis(8))
                    .amplify(if s == Sound::Urgent { 0.6 } else { 0.35 }),
            );
        }
    }
    sink.sleep_until_end();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_are_short() {
        for s in [Sound::Chime, Sound::Alert, Sound::Urgent] {
            let ms: u64 = notes(s).iter().map(|n| n.1).sum();
            assert!(ms > 0 && ms < 5_000, "{s:?} lasts {ms} ms");
        }
        assert!(notes(Sound::Off).is_empty());
    }

    /// Plays every tone on this computer's speakers: `cargo test sound -- --ignored`.
    #[test]
    #[ignore]
    fn listen() {
        for s in [Sound::Chime, Sound::Alert, Sound::Urgent] {
            play_blocking(s).expect("audio output");
        }
    }
}
