//! Optional feedback sounds for copy outcomes.
//!
//! Sounds are fire-and-forget: playback is asynchronous and a missing or broken
//! file falls back to the Windows tone, so audio can never fail a copy.

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
    /// The Windows tone for the outcome.
    Tone,
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
        "" => Sound::Tone,
        path => Sound::File(path),
    }
}

/// Plays the configured sound for `outcome`, if the user enabled one.
#[cfg(windows)]
pub fn play(settings: &Settings, outcome: Outcome) {
    match choice(settings, outcome) {
        Sound::Off => {}
        Sound::Tone => win::beep(outcome),
        Sound::File(path) => {
            if !win::play_file(path) {
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
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONHAND, MB_OK, MESSAGEBOX_STYLE};

    use super::Outcome;

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

    /// The Windows tone used when no custom file is configured or playable.
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
    fn a_blank_file_uses_the_windows_tone() {
        let mut settings = Settings::default();
        settings.copy_sound = true;
        settings.copy_sound_file = "   ".into();
        assert_eq!(choice(&settings, Outcome::Copied), Sound::Tone);
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
    fn the_success_sound_is_on_by_default_and_failure_is_not() {
        let settings = Settings::default();
        assert_eq!(choice(&settings, Outcome::Copied), Sound::Tone);
        assert_eq!(choice(&settings, Outcome::Failed), Sound::Off);
    }
}
