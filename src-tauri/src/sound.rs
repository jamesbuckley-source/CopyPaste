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

/// Plays the configured sound for `outcome`, if the user enabled one.
#[cfg(windows)]
pub fn play(settings: &Settings, outcome: Outcome) {
    let (enabled, file) = match outcome {
        Outcome::Copied => (settings.copy_sound, settings.copy_sound_file.as_str()),
        Outcome::Failed => (settings.copy_fail_sound, settings.copy_fail_sound_file.as_str()),
    };
    if !enabled {
        return;
    }
    let file = file.trim();
    if file.is_empty() || !std::path::Path::new(file).is_file() || !play_file(file) {
        win::beep(outcome);
    }
}

/// Non-Windows builds have no audio playback wired up.
#[cfg(not(windows))]
pub fn play(_settings: &Settings, _outcome: Outcome) {}

#[cfg(windows)]
fn play_file(path: &str) -> bool {
    win::play_file(path)
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBeep, MB_ICONHAND, MB_OK};

    use super::Outcome;

    /// Starts a WAV in the background; false means the file could not be played.
    pub fn play_file(path: &str) -> bool {
        let wide: Vec<u16> = std::ffi::OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            PlaySoundW(PCWSTR(wide.as_ptr()), None, SND_FILENAME | SND_ASYNC | SND_NODEFAULT).is_ok()
        }
    }

    /// The Windows tone used when no custom file is configured.
    pub fn beep(outcome: Outcome) {
        let sound = match outcome {
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
    fn a_disabled_sound_plays_nothing() {
        let mut settings = Settings::default();
        settings.copy_sound = false;
        settings.copy_fail_sound = false;
        // `play` must return quietly instead of reaching for audio.
        play(&settings, Outcome::Copied);
        play(&settings, Outcome::Failed);
    }

    #[test]
    fn a_missing_file_falls_back_to_the_default_tone() {
        let mut settings = Settings::default();
        settings.copy_sound = true;
        settings.copy_sound_file = "C:\\does\\not\\exist.wav".to_string();
        // The fallback path is what a stale setting relies on.
        play(&settings, Outcome::Copied);
    }
}
