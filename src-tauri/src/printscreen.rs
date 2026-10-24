//! Windows' Print Screen key.
//!
//! Pressing Print Screen does not reach an application's shortcut registration
//! while the Snipping Tool owns the key: Windows intercepts it first, so a hotkey
//! bound to `PrintScreen` simply never fires. Turning the Snipping Tool's handler
//! off is a single per-user value in the registry — no elevation, no sign-out —
//! and that is the whole of what this module touches.
//!
//! Whatever the value was before is recorded first and written back afterwards,
//! so switching the option off returns the key exactly as it was found rather than
//! to whatever the app assumes the default must be.

/// Holds `PrintScreenKeyForSnippingEnabled` among the other keyboard settings.
const KEY: &str = r"Control Panel\Keyboard";
/// `0` gives Print Screen back to applications; `1`, or an absent value, gives it
/// to the Snipping Tool.
const VALUE: &str = "PrintScreenKeyForSnippingEnabled";

/// The shortcut spelling the hotkey layer parses for the key itself.
pub const SHORTCUT: &str = "PrintScreen";

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD, RegDeleteKeyValueW, RegGetValueW,
        RegSetKeyValueW,
    };
    use windows::core::{Error as WinError, PCWSTR};

    use crate::error::{AppError, Result};

    use super::{KEY, VALUE};

    /// The stored value: `None` when it is absent, which Windows reads as "the
    /// Snipping Tool handles Print Screen".
    pub fn current() -> Result<Option<u32>> {
        let key = wide(KEY);
        let name = wide(VALUE);
        let mut data = 0u32;
        let mut bytes = std::mem::size_of::<u32>() as u32;
        // SAFETY: both strings are NUL-terminated and outlive the call, and the
        // buffer is exactly the size the API is told it has.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut data as *mut u32 as *mut core::ffi::c_void),
                Some(&mut bytes),
            )
        };
        match status {
            ERROR_SUCCESS => Ok(Some(data)),
            ERROR_FILE_NOT_FOUND => Ok(None),
            other => Err(AppError::msg(format!(
                "could not read the Print Screen setting: {}",
                message(other)
            ))),
        }
    }

    pub fn write(value: Option<u32>) -> Result<()> {
        let key = wide(KEY);
        let name = wide(VALUE);
        match value {
            Some(value) => {
                // SAFETY: the strings are NUL-terminated and outlive the call, and
                // the pointer names a `u32` whose length matches `REG_DWORD`.
                let status = unsafe {
                    RegSetKeyValueW(
                        HKEY_CURRENT_USER,
                        PCWSTR(key.as_ptr()),
                        PCWSTR(name.as_ptr()),
                        REG_DWORD.0,
                        Some(&value as *const u32 as *const core::ffi::c_void),
                        std::mem::size_of::<u32>() as u32,
                    )
                };
                if status != ERROR_SUCCESS {
                    return Err(AppError::msg(format!(
                        "could not hand Print Screen over: {}",
                        message(status)
                    )));
                }
            }
            None => {
                // SAFETY: the same NUL-terminated strings. Deleting a value that is
                // already absent reaches the state we want, so its error is moot.
                unsafe {
                    let _ = RegDeleteKeyValueW(
                        HKEY_CURRENT_USER,
                        PCWSTR(key.as_ptr()),
                        PCWSTR(name.as_ptr()),
                    );
                }
            }
        }
        Ok(())
    }

    fn message(status: WIN32_ERROR) -> String {
        WinError::from_hresult(windows::core::HRESULT::from_win32(status.0)).to_string()
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::error::Result;

    pub fn current() -> Result<Option<u32>> {
        Ok(None)
    }

    pub fn write(_value: Option<u32>) -> Result<()> {
        Ok(())
    }
}

/// Takes the key away from the Snipping Tool, returning what was there before so
/// [`release`] can put it back.
///
/// `Ok(Some(0))` means the key was already free and nothing was written.
pub fn claim() -> crate::error::Result<Option<u32>> {
    let previous = imp::current()?;
    if previous == Some(0) {
        return Ok(previous);
    }
    imp::write(Some(0))?;
    Ok(previous)
}

/// Gives Print Screen back to whatever had it before [`claim`] ran.
pub fn release(previous: Option<u32>) -> crate::error::Result<()> {
    if previous == Some(0) {
        return Ok(());
    }
    imp::write(previous)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claim is a no-op on a key that is already free, so releasing must write
    /// nothing: otherwise the app would flip a setting it never changed.
    #[test]
    fn a_key_that_was_already_free_is_left_alone() {
        assert!(release(Some(0)).is_ok());
    }

    /// The registry plumbing, against the real key. Off by default because it
    /// changes a Windows setting, however briefly:
    /// `cargo test print_screen_round_trips -- --ignored`.
    #[test]
    #[ignore = "writes to the live registry"]
    fn print_screen_round_trips_through_the_registry() {
        let before = imp::current().expect("the value is readable");
        println!("before: {before:?}");

        let claimed = claim().expect("the key can be taken");
        assert_eq!(claimed, before, "claim reports what was there");
        assert_eq!(
            imp::current().expect("readable after claim"),
            Some(0),
            "the Snipping Tool no longer owns the key"
        );

        release(before).expect("the key can be given back");
        assert_eq!(
            imp::current().expect("readable after release"),
            before,
            "the original value is back"
        );
    }

    /// The spelling the hotkey layer parses has to be accepted, because an
    /// unparseable accelerator is skipped rather than reported, and the key would
    /// silently never fire.
    #[test]
    fn the_shortcut_spelling_parses_as_an_accelerator() {
        assert_eq!(SHORTCUT, "PrintScreen");
        assert!(
            "PrintScreen"
                .parse::<tauri_plugin_global_shortcut::Shortcut>()
                .is_ok()
        );
    }
}
