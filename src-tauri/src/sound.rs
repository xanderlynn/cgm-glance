//! Generated alarm tones, so there are no audio files to ship or load.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
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
            (1568.0, 110),
            (0.0, 50),
            (1568.0, 110),
            (0.0, 50),
            (1568.0, 110),
            (0.0, 420),
        ]
        .repeat(3),
    }
}

/// True while a tone is playing, so repeated alerts don't stack up.
static PLAYING: AtomicBool = AtomicBool::new(false);

/// Plays a tone on the default output device without blocking.
pub fn play(s: Sound) {
    if s == Sound::Off || PLAYING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = play_blocking(s) {
            eprintln!("cgm-glance: could not play the alarm sound: {e}");
        }
        PLAYING.store(false, Ordering::SeqCst);
    });
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
                    .amplify(0.35),
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
            assert!(ms > 0 && ms < 3_000, "{s:?} lasts {ms} ms");
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
