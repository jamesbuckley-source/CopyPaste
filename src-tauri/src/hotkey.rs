//! Global shortcut registration.
//!
//! The handler itself lives in the plugin builder (see `lib.rs`); this module only
//! owns reconciling the bindings with the user's configured shortcuts.

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::error::{AppError, Result};
use crate::settings::Settings;

/// The capture entry points that can each own a shortcut of their own.
///
/// Declaration order is the order the settings column lists them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureAction {
    Region,
    Window,
    Screen,
    Scrolling,
    Color,
}

impl CaptureAction {
    pub const ALL: [CaptureAction; 5] = [
        CaptureAction::Region,
        CaptureAction::Window,
        CaptureAction::Screen,
        CaptureAction::Scrolling,
        CaptureAction::Color,
    ];

    /// Index into [`Bindings::captures`].
    fn slot(self) -> usize {
        match self {
            CaptureAction::Region => 0,
            CaptureAction::Window => 1,
            CaptureAction::Screen => 2,
            CaptureAction::Scrolling => 3,
            CaptureAction::Color => 4,
        }
    }

    /// How a conflict names the action back to the user.
    pub fn label(self) -> &'static str {
        match self {
            CaptureAction::Region => "the region screenshot",
            CaptureAction::Window => "the window screenshot",
            CaptureAction::Screen => "the full-screen screenshot",
            CaptureAction::Scrolling => "the scrolling screenshot",
            CaptureAction::Color => "the colour picker",
        }
    }

    /// The stored combination for this action; empty when it has none.
    pub fn configured(self, settings: &Settings) -> &str {
        match self {
            CaptureAction::Region => &settings.capture_region_hotkey,
            CaptureAction::Window => &settings.capture_window_hotkey,
            CaptureAction::Screen => &settings.capture_screen_hotkey,
            CaptureAction::Scrolling => &settings.capture_scrolling_hotkey,
            CaptureAction::Color => &settings.pick_color_hotkey,
        }
    }
}

/// Every global shortcut the app holds, in the form the settings spell them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings {
    /// Summons the window; always set.
    pub summon: String,
    /// Toggles private mode. Best effort, because another app may own it.
    pub pause: String,
    /// One per [`CaptureAction`], indexed by [`CaptureAction::slot`]. Empty means
    /// the action has no shortcut at all.
    captures: [String; 5],
    /// Print Screen, when the user has asked the app to take the key over. It is
    /// spelled as a binding like any other so it is registered and released with
    /// the rest; what it *does* is decided from the default capture type.
    print_screen: bool,
}

impl Bindings {
    pub fn from_settings(settings: &Settings) -> Self {
        let mut captures: [String; 5] = Default::default();
        for action in CaptureAction::ALL {
            captures[action.slot()] = action.configured(settings).trim().to_string();
        }
        Bindings {
            summon: settings.hotkey.trim().to_string(),
            pause: settings.pause_hotkey.trim().to_string(),
            captures,
            print_screen: settings.print_screen_capture,
        }
    }

    /// True before anything has been registered, which is the state at startup.
    fn is_empty(&self) -> bool {
        self.summon.is_empty() && self.captures.iter().all(String::is_empty) && !self.print_screen
    }

    /// The same bindings without the opt-in capture shortcuts.
    ///
    /// Startup falls back to this when one of them cannot be registered: the
    /// summon key is the only way back into the app from the keyboard, so a stale
    /// or contended capture combination must not take it down too.
    pub fn without_capture_shortcuts(mut self) -> Self {
        self.captures = Default::default();
        self.print_screen = false;
        self
    }

    /// Parses every shortcut that must work, rejecting a malformed value and a
    /// combination two actions claim.
    ///
    /// Deliberately free of the OS: the same rule runs in tests and before the
    /// first registration, so a mistake is a message rather than a half-registered
    /// set of shortcuts.
    fn required(&self) -> Result<Vec<(&'static str, &str, Shortcut)>> {
        let mut parsed: Vec<(&'static str, &str, Shortcut)> = Vec::new();
        parsed.push((
            "the window",
            self.summon.as_str(),
            parse(&self.summon, "the window")?,
        ));

        for action in CaptureAction::ALL {
            let text = self.captures[action.slot()].as_str();
            // No shortcut asked for: registering an empty accelerator would fail.
            if text.is_empty() {
                continue;
            }
            let shortcut = parse(text, action.label())?;
            if let Some((owner, _, _)) = parsed.iter().find(|(_, _, taken)| *taken == shortcut) {
                return Err(AppError::msg(format!(
                    "`{text}` is already used by {owner}"
                )));
            }
            parsed.push((action.label(), text, shortcut));
        }

        if self.print_screen {
            let shortcut = parse(crate::printscreen::SHORTCUT, "Print Screen")?;
            // Print Screen is one key, so a user who also bound it to an action
            // has to choose which of the two it means.
            if let Some((owner, _, _)) = parsed.iter().find(|(_, _, taken)| *taken == shortcut) {
                return Err(AppError::msg(format!(
                    "Print Screen is already used by {owner}"
                )));
            }
            parsed.push(("Print Screen", crate::printscreen::SHORTCUT, shortcut));
        }

        Ok(parsed)
    }
}

fn parse(text: &str, owner: &'static str) -> Result<Shortcut> {
    text.parse::<Shortcut>()
        .map_err(|e| AppError::msg(format!("`{text}` is not a valid shortcut for {owner}: {e}")))
}

/// Applies `next`, putting `previous` back when the OS refuses a combination.
///
/// `register` is the only OS-facing step, so the restore decision can be tested
/// with a stub that rejects one combination.
fn switch(
    previous: &Bindings,
    next: &Bindings,
    mut register: impl FnMut(&Bindings) -> Result<()>,
) -> Result<()> {
    // Caught before the OS sees it, while the working set is still registered.
    next.required()?;

    if let Err(e) = register(next) {
        // Rebinding releases the old set first, so a refused candidate would
        // otherwise leave the user with no shortcuts at all.
        if !previous.is_empty()
            && let Err(restore) = register(previous)
        {
            log::error!("could not restore the previous shortcuts: {restore}");
        }
        return Err(e);
    }
    Ok(())
}

/// Rebinds the app's shortcuts to `next`, keeping `previous` alive if the system
/// refuses one of them.
pub fn apply<R: Runtime>(app: &AppHandle<R>, previous: &Bindings, next: &Bindings) -> Result<()> {
    switch(previous, next, |bindings| register(app, bindings))
}

fn register<R: Runtime>(app: &AppHandle<R>, bindings: &Bindings) -> Result<()> {
    let manager = app.global_shortcut();
    if let Err(e) = manager.unregister_all() {
        // Not fatal: the OS may simply have had nothing registered yet.
        log::debug!("no previous shortcut to release: {e}");
    }

    for (owner, text, shortcut) in bindings.required()? {
        manager.register(shortcut).map_err(|e| {
            AppError::msg(format!("`{text}` could not be registered for {owner}: {e}"))
        })?;
        log::info!("shortcut registered for {owner}: {text}");
    }

    // Best effort: a pause key another app owns only costs the keyboard toggle,
    // so it must not take the working shortcuts down with it.
    match bindings.pause.parse::<Shortcut>() {
        Ok(pause) => match manager.register(pause) {
            Ok(()) => log::info!("pause shortcut registered: {}", bindings.pause),
            Err(e) => log::warn!("`{}` could not be registered: {e}", bindings.pause),
        },
        Err(e) => log::warn!("`{}` is not a valid shortcut: {e}", bindings.pause),
    }

    Ok(())
}

/// The capture action a pressed shortcut belongs to, if any.
///
/// Comparing parsed shortcuts rather than strings lets the stored spelling
/// (`Ctrl+Shift+R`) match what the plugin reports (`control+shift+KeyR`).
pub fn capture_action(pressed: &Shortcut, settings: &Settings) -> Option<CaptureAction> {
    CaptureAction::ALL.into_iter().find(|action| {
        action
            .configured(settings)
            .parse::<Shortcut>()
            .is_ok_and(|configured| configured == *pressed)
    })
}

/// The capture a Print Screen press starts.
///
/// Whatever the header split button last remembered, so the key and the button
/// cannot disagree about what "the default capture" means.
pub fn default_capture(settings: &Settings) -> CaptureAction {
    match settings.capture_mode.as_str() {
        "window" => CaptureAction::Window,
        "screen" => CaptureAction::Screen,
        "scrolling" => CaptureAction::Scrolling,
        _ => CaptureAction::Region,
    }
}

/// True when the pressed shortcut is the Print Screen the app took over.
pub fn is_print_screen(pressed: &Shortcut, settings: &Settings) -> bool {
    settings.print_screen_capture
        && crate::printscreen::SHORTCUT
            .parse::<Shortcut>()
            .is_ok_and(|configured| configured == *pressed)
}

/// Releases every binding, used when the app shuts down.
pub fn release<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        log::debug!("could not release global shortcuts: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Bindings, CaptureAction, capture_action, default_capture, is_print_screen, switch,
    };

    use crate::error::AppError;
    use crate::settings::Settings;

    fn bindings(region: &str) -> Bindings {
        Bindings {
            summon: "Alt+V".into(),
            pause: "Ctrl+Alt+P".into(),
            captures: [
                region.into(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ],
            print_screen: false,
        }
    }

    #[test]
    fn an_unset_capture_shortcut_registers_nothing() {
        let bindings = Bindings::from_settings(&Settings::default());
        let required = bindings
            .required()
            .expect("the default summon key is valid");
        assert_eq!(
            required.len(),
            1,
            "only the summon key is registered on a fresh install"
        );
    }

    #[test]
    fn two_actions_cannot_share_a_combination() {
        let mut settings = Settings::default();
        settings.capture_region_hotkey = "Ctrl+Shift+R".into();
        // Same keys, different spelling: the parse decides, not the text.
        settings.capture_window_hotkey = "control+shift+keyr".into();
        let error = Bindings::from_settings(&settings)
            .required()
            .expect_err("a duplicate is caught before the OS sees it");
        assert!(
            error
                .to_string()
                .contains("already used by the region screenshot"),
            "the message names the owner: {error}"
        );
    }

    #[test]
    fn a_malformed_capture_shortcut_is_rejected() {
        let mut settings = Settings::default();
        settings.pick_color_hotkey = "Ctrl+NotAKey".into();
        let error = Bindings::from_settings(&settings)
            .required()
            .expect_err("an unparseable combination is an error, not a silent skip");
        assert!(error.to_string().contains("colour picker"), "{error}");
    }

    #[test]
    fn a_rejected_shortcut_puts_the_previous_set_back() {
        let previous = bindings("Ctrl+Shift+R");
        let next = bindings("Ctrl+Shift+K");
        let mut attempts: Vec<Bindings> = Vec::new();

        let result = switch(&previous, &next, |candidate| {
            attempts.push(candidate.clone());
            if *candidate == next {
                // What Windows does when another application owns the keys.
                Err(AppError::msg(
                    "`Ctrl+Shift+K` could not be registered: already in use",
                ))
            } else {
                Ok(())
            }
        });

        assert!(result.is_err());
        assert_eq!(
            attempts.last(),
            Some(&previous),
            "the working set is re-registered after the refusal"
        );
    }

    #[test]
    fn the_spelling_the_settings_column_writes_is_accepted() {
        // The recorder in the settings UI writes these shapes: modifier names,
        // a bare letter or digit, and a browser key code for the rest.
        let mut settings = Settings::default();
        settings.capture_region_hotkey = "Ctrl+Shift+R".into();
        settings.capture_window_hotkey = "Ctrl+Num1".into();
        settings.capture_screen_hotkey = "Alt+ArrowUp".into();
        settings.capture_scrolling_hotkey = "Ctrl+Alt+Shift+S".into();
        settings.pick_color_hotkey = "Ctrl+F5".into();

        assert_eq!(
            Bindings::from_settings(&settings).required().unwrap().len(),
            6,
            "the summon key plus all five capture shortcuts"
        );
    }

    /// Print Screen is a binding like the rest while the option is on, and gone
    /// the moment it is off.
    #[test]
    fn print_screen_is_bound_only_while_it_is_asked_for() {
        let mut settings = Settings::default();
        assert!(
            !Bindings::from_settings(&settings)
                .required()
                .unwrap()
                .iter()
                .any(|(owner, _, _)| *owner == "Print Screen"),
            "a fresh install does not take the key"
        );

        settings.print_screen_capture = true;
        let bindings = Bindings::from_settings(&settings);
        let bound = bindings.required().unwrap();
        assert!(
            bound
                .iter()
                .any(|(owner, text, _)| *owner == "Print Screen" && *text == "PrintScreen"),
            "asking for it registers exactly one Print Screen binding"
        );
    }

    /// One key cannot mean two things, and the message has to name the action it
    /// collided with or the user cannot fix it.
    #[test]
    fn print_screen_cannot_also_be_a_capture_shortcut() {
        let mut settings = Settings::default();
        settings.print_screen_capture = true;
        settings.capture_region_hotkey = "PrintScreen".into();

        let error = Bindings::from_settings(&settings)
            .required()
            .expect_err("the duplicate has to be refused");
        let message = error.to_string();
        assert!(
            message.contains("Print Screen"),
            "the refusal names Print Screen: {message}"
        );
    }

    #[test]
    fn print_screen_starts_the_remembered_capture_type() {
        let mut settings = Settings::default();
        settings.print_screen_capture = true;

        for (mode, expected) in [
            ("region", CaptureAction::Region),
            ("window", CaptureAction::Window),
            ("screen", CaptureAction::Screen),
            ("scrolling", CaptureAction::Scrolling),
            // The picker is not a capture type, so a nonsense value is region.
            ("nonsense", CaptureAction::Region),
        ] {
            settings.capture_mode = mode.into();
            assert_eq!(default_capture(&settings), expected, "capture_mode {mode}");
        }

        let pressed: tauri_plugin_global_shortcut::Shortcut =
            "printscreen".parse().expect("valid");
        assert!(is_print_screen(&pressed, &settings));
        settings.print_screen_capture = false;
        assert!(!is_print_screen(&pressed, &settings));
    }

    #[test]
    fn a_pressed_shortcut_maps_to_its_capture_action() {
        let mut settings = Settings::default();
        settings.capture_screen_hotkey = "Ctrl+Shift+1".into();
        settings.pick_color_hotkey = "Ctrl+Shift+2".into();

        let screen: tauri_plugin_global_shortcut::Shortcut =
            "ctrl+shift+digit1".parse().expect("valid");
        assert_eq!(
            capture_action(&screen, &settings),
            Some(CaptureAction::Screen)
        );

        let picker: tauri_plugin_global_shortcut::Shortcut =
            "ctrl+shift+digit2".parse().expect("valid");
        assert_eq!(
            capture_action(&picker, &settings),
            Some(CaptureAction::Color)
        );

        let summon: tauri_plugin_global_shortcut::Shortcut = "alt+keyv".parse().expect("valid");
        assert_eq!(
            capture_action(&summon, &settings),
            None,
            "the summon key is not a capture"
        );
    }
}
