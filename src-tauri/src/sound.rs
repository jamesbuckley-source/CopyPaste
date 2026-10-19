//! Optional feedback sounds for copy outcomes.
//!
//! Sounds are fire-and-forget: playback is asynchronous and a missing or broken
//! file falls back to the bundled default, so audio can never fail a copy.

use crate::settings::Settings;

/// Which copy outcome a sound belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Copied,
    Failed,
}

/// What a copy should sound like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sound<'a> {
    /// The user turned this sound off.
    Off,
    /// The WAV that ships with the app.
    Default,
    /// A WAV the user chose.
    File(&'a str),
}

/// Reads the settings for `outcome`; kept pure so it can be tested without audio.
fn choice(settings: &Settings, outcome: Outcome) -> Sound<'_> {
    let (enabled, file) = match outcome {
        Outcome::Copied => (settings.copy_sound, settings.copy_sound_file.as_str()),
        Outcome::Failed => (settings.copy_fail_sound, settings.copy_fail_sound_file.as_str()),
    };
    if !enabled {
        return Sound::Off;
    }
    match file.trim() {
        "" => Sound::Default,
        path => Sound::File(path),
    }
}

/// Plays the configured sound for `outcome`, if the user enabled one.
#[cfg(windows)]
pub fn play(settings: &Settings, outcome: Outcome) {
    match choice(settings, outcome) {
        Sound::Off => {}
        Sound::Default => {
            if !win::play_default(outcome) {
                win::beep(outcome);
            }
        }
        Sound::File(path) => {
            // A custom file that vanished since it was chosen should still make
            // the noise the user expects, so fall back to the bundled sound.
            if !win::play_file(path) && !win::play_default(outcome) {
                win::beep(outcome);
            }
        }
    }
}

/// Non-Windows builds have no audio playback wired up.
#[cfg(not(windows))]
pub fn play(_settings: &Settings, _outcome: Outcome) {}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{
        PlaySoundW, SND_ASYNC, SND_FILENAME, SND_MEMORY, SND_NODEFAULT,
    };
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONHAND, MB_OK, MESSAGEBOX_STYLE};

    use super::Outcome;

    /// Embedded so a fresh install has its own sound without shipping loose files.
    const COPY_WAV: &[u8] = include_bytes!("../sounds/copy.wav");
    const FAIL_WAV: &[u8] = include_bytes!("../sounds/fail.wav");

    /// The sound that stands in for a custom file, or for none at all.
    pub(super) fn default_wav(outcome: Outcome) -> &'static [u8] {
        match outcome {
            Outcome::Copied => COPY_WAV,
            Outcome::Failed => FAIL_WAV,
        }
    }

    /// Starts the bundled WAV in the background; false means it could not play.
    pub fn play_default(outcome: Outcome) -> bool {
        // `PlaySoundW` reads the WAV straight from memory, which is why the bytes
        // above are compiled in: the buffer has to outlive asynchronous playback.
        let wav = default_wav(outcome);
        unsafe {
            PlaySoundW(
                PCWSTR(wav.as_ptr() as *const u16),
                None,
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            )
            .as_bool()
        }
    }

    /// Starts a WAV in the background; false means the file could not be played.
    pub fn play_file(path: &str) -> bool {
        if !std::path::Path::new(path).is_file() {
            return false;
        }
        let wide: Vec<u16> = std::ffi::OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            PlaySoundW(
                PCWSTR(wide.as_ptr()),
                None,
                SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
            )
            .as_bool()
        }
    }

    /// Last resort when neither the chosen file nor the bundled WAV can play.
    pub fn beep(outcome: Outcome) {
        let sound: MESSAGEBOX_STYLE = match outcome {
            Outcome::Copied => MB_OK,
            Outcome::Failed => MB_ICONHAND,
        };
        unsafe { _ = MessageBeep(sound) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_sound_is_silent() {
        let mut settings = Settings::default();
        settings.copy_sound = false;
        settings.copy_sound_file = "C:\\clips\\tick.wav".into();
        assert_eq!(choice(&settings, Outcome::Copied), Sound::Off);

        settings.copy_fail_sound = false;
        assert_eq!(choice(&settings, Outcome::Failed), Sound::Off);
    }

    #[test]
    fn a_blank_file_uses_the_bundled_default() {
        let mut settings = Settings::default();
        settings.copy_sound = true;
        settings.copy_sound_file = "   ".into();
        assert_eq!(choice(&settings, Outcome::Copied), Sound::Default);
    }

    #[test]
    fn a_chosen_file_is_played() {
        let mut settings = Settings::default();
        settings.copy_fail_sound = true;
        settings.copy_fail_sound_file = "  C:\\clips\\nope.wav  ".into();
        assert_eq!(
            choice(&settings, Outcome::Failed),
            Sound::File("C:\\clips\\nope.wav")
        );
    }

    #[test]
    fn both_outcomes_play_the_bundled_sounds_by_default() {
        let settings = Settings::default();
        assert_eq!(choice(&settings, Outcome::Copied), Sound::Default);
        assert_eq!(choice(&settings, Outcome::Failed), Sound::Default);
    }

    #[test]
    #[cfg(windows)]
    fn the_bundled_sounds_are_valid_wavs() {
        for outcome in [Outcome::Copied, Outcome::Failed] {
            let wav = win::default_wav(outcome);
            assert!(wav.len() > 44, "a bundled WAV is missing its audio data");
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(&wav[8..12], b"WAVE");
        }
    }
}
