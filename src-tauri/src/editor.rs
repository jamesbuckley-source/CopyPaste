//! Turning the `imageEditor` setting into a command that opens a file.
//!
//! The setting holds a whole command line rather than a bare path, because plenty
//! of editors need to be told *how* to open a file: ShotCraft wants
//! `open-annotate --filepath`, a code editor often wants a line or a flag. The
//! image path goes wherever `{file}` appears, and is appended when it does not, so
//! the plain `mspaint.exe` case still needs nothing typed.

use crate::error::{AppError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Stands in for the image's path inside the configured command line.
pub const FILE_PLACEHOLDER: &str = "{file}";

/// Builds the command that opens `file` in the configured editor.
pub fn command_for(editor: &str, file: &Path) -> Result<Command> {
    let mut parts = split_editor_command(editor);
    if parts.is_empty() {
        return Err(AppError::msg("no image editor is set"));
    }
    let program = parts.remove(0);
    let file = file.to_string_lossy().to_string();

    let mut command = Command::new(&program);
    let mut placed = false;
    for arg in &parts {
        if arg.contains(FILE_PLACEHOLDER) {
            command.arg(arg.replace(FILE_PLACEHOLDER, &file));
            placed = true;
        } else {
            command.arg(arg);
        }
    }
    if !placed {
        command.arg(&file);
    }
    Ok(command)
}

/// The program a command line starts with, for messages that name it.
pub fn program_of(editor: &str) -> Option<PathBuf> {
    split_editor_command(editor).first().map(PathBuf::from)
}

/// Splits the configured command, repairing a program path nobody quoted.
///
/// Typing or pasting `C:\Program Files\App\app.exe --flag` without quotes leaves
/// the program as `C:\Program`, which is not a program at all. When the first
/// token is not a file but a longer run of the line is, that run was the program
/// and the rest are its arguments. A run that ends in a program extension counts
/// even when nothing is there, so a mistyped path is reported in full rather than
/// as its first word. Quoted lines are left alone: the splitter already honoured
/// them, so a miss there is a genuine mistake worth reporting.
pub fn split_editor_command(command: &str) -> Vec<String> {
    let parts = split_command_line(command);
    let Some(first) = parts.first() else {
        return parts;
    };
    if parts.len() == 1 || first.starts_with('"') || Path::new(first).is_file() {
        return parts;
    }

    let runs: Vec<&str> = command
        .char_indices()
        .filter(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| &command[..index])
        .collect();
    let found = runs
        .iter()
        .find(|run| Path::new(run).is_file())
        .or_else(|| runs.iter().find(|run| has_program_extension(run)));
    let Some(program) = found else {
        return parts;
    };

    let mut repaired = vec![program.to_string()];
    repaired.extend(split_command_line(&command[program.len()..]));
    repaired
}

/// Whether a run of the command line ends like a program rather than an argument.
fn has_program_extension(run: &str) -> bool {
    let lowered = run.to_ascii_lowercase();
    [".exe", ".com", ".bat", ".cmd"]
        .iter()
        .any(|ext| lowered.ends_with(ext))
}

/// Splits a command line the way the C runtime hands arguments to a program.
///
/// Quoted runs stay together, so a program under `C:\Program Files` is one token;
/// `""` inside quotes is a literal quote, and backslashes before a quote follow
/// the usual 2n/2n+1 rule. This is `CommandLineToArgvW`'s behaviour minus its
/// special case for the first token, which only matters for programs whose own
/// path contains quotes.
pub fn split_command_line(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut backslashes = 0usize;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                backslashes += 1;
                continue;
            }
            '"' => {
                // Half the backslashes are literal; an odd one escapes the quote.
                let escaped = backslashes % 2 == 1;
                current.push_str(&"\\".repeat(backslashes / 2));
                backslashes = 0;
                started = true;
                if escaped {
                    current.push('"');
                } else if quoted && chars.peek() == Some(&'"') {
                    // A doubled quote inside a quoted run is one literal quote.
                    chars.next();
                    current.push('"');
                } else {
                    quoted = !quoted;
                }
                continue;
            }
            _ => {}
        }

        current.push_str(&"\\".repeat(backslashes));
        backslashes = 0;

        if !quoted && ch.is_whitespace() {
            if started {
                args.push(std::mem::take(&mut current));
                started = false;
            }
            continue;
        }
        current.push(ch);
        started = true;
    }

    current.push_str(&"\\".repeat(backslashes));
    if started {
        args.push(current);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_program_is_one_argument() {
        assert_eq!(split_command_line("mspaint.exe"), ["mspaint.exe"]);
        assert_eq!(split_command_line("   "), Vec::<String>::new());
        assert_eq!(split_command_line(""), Vec::<String>::new());
    }

    #[test]
    fn a_quoted_program_keeps_its_spaces() {
        assert_eq!(
            split_command_line(r#""C:\Program Files\ShotCraft\shotcraft.exe" open-annotate"#),
            [
                r"C:\Program Files\ShotCraft\shotcraft.exe",
                "open-annotate"
            ]
        );
    }

    #[test]
    fn arguments_after_the_program_are_kept_in_order() {
        assert_eq!(
            split_command_line(r#"tool.exe open-annotate --filepath "{file}""#),
            [
                "tool.exe".to_string(),
                "open-annotate".to_string(),
                "--filepath".to_string(),
                "{file}".to_string()
            ]
        );
    }

    #[test]
    fn quotes_and_backslashes_follow_the_c_runtime_rules() {
        // An escaped quote inside a quoted run is a literal quote.
        assert_eq!(
            split_command_line(r#""a\"b" c"#),
            ["a\"b".to_string(), "c".to_string()]
        );
        // A path that ends in a backslash keeps it, because the quote is escaped.
        let trailing = format!("{}{}{}", '"', r"C:\shots\\", '"');
        assert_eq!(split_command_line(&trailing), [r"C:\shots\"]);
        // Doubled quotes inside a quoted run collapse to one.
        assert_eq!(
            split_command_line(r#"say "a""b""#),
            ["say".to_string(), "a\"b".to_string()]
        );
        // Unquoted backslashes are literal.
        assert_eq!(split_command_line(r"C:\a\b"), [r"C:\a\b"]);
    }

    #[test]
    fn the_file_path_is_appended_when_the_command_does_not_place_it() {
        let command = command_for(r#""C:\Program Files\ShotCraft\shotcraft.exe" open-annotate"#, Path::new(r"C:\shots\one.png")).unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, ["open-annotate", r"C:\shots\one.png"]);
        assert_eq!(
            command.get_program().to_string_lossy(),
            r"C:\Program Files\ShotCraft\shotcraft.exe"
        );
    }

    #[test]
    fn the_placeholder_decides_where_the_path_goes() {
        let command = command_for(
            r#""C:\Program Files\ShotCraft\shotcraft.exe" open-annotate --filepath {file}"#,
            Path::new(r"C:\my shots\one.png"),
        )
        .unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        // The path with a space in it stays a single argument: no shell is involved.
        assert_eq!(
            args,
            ["open-annotate", "--filepath", r"C:\my shots\one.png"]
        );
    }

    #[test]
    fn an_empty_setting_is_an_error_rather_than_a_launch() {
        assert!(command_for("", Path::new(r"C:\a.png")).is_err());
        assert!(command_for("   ", Path::new(r"C:\a.png")).is_err());
    }

    #[test]
    fn an_unquoted_program_path_is_repaired() {
        // What an older build stored, and what typing a path without quotes gives.
        let command = command_for(
            r"C:\Program Files\ShotCraft\shotcraft.exe open-annotate --filepath {file}",
            Path::new(r"C:\shots\one.png"),
        )
        .unwrap();
        assert_eq!(
            command.get_program().to_string_lossy(),
            r"C:\Program Files\ShotCraft\shotcraft.exe",
            "the whole path is the program, not just its first word"
        );
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, ["open-annotate", "--filepath", r"C:\shots\one.png"]);
    }

    #[test]
    fn a_missing_program_is_left_for_the_error_to_explain() {
        // Nothing on disk matches a longer prefix, so the split stands and the
        // launch fails with the program name in the message.
        let parts = split_editor_command(r"C:\nowhere\app.exe --flag");
        assert_eq!(parts, [r"C:\nowhere\app.exe", "--flag"]);
        assert_eq!(
            program_of(r"C:\Program Files\Nope\nope.exe --flag"),
            Some(PathBuf::from(r"C:\Program Files\Nope\nope.exe"))
        );
    }

    #[test]
    fn a_program_on_the_path_is_not_glued_to_its_arguments() {
        let parts = split_editor_command(r"mspaint.exe C:\shots\one.png");
        assert_eq!(parts, ["mspaint.exe", r"C:\shots\one.png"]);
    }
}
