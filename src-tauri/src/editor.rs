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
    let mut parts = split_command_line(editor);
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
    split_command_line(editor).first().map(PathBuf::from)
}

/// Quotes a path so it survives [`split_command_line`] as one argument.
///
/// The file picker hands back a bare path, and an editor under `C:\Program Files`
/// has a space in it, so the setting has to be stored with quotes or it would be
/// read as two arguments.
pub fn quote_path(path: &str) -> String {
    if path.contains(' ') && !path.starts_with('"') {
        format!("\"{path}\"")
    } else {
        path.to_string()
    }
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
    fn picked_paths_with_spaces_are_quoted_once() {
        assert_eq!(
            quote_path(r"C:\Program Files\ShotCraft\shotcraft.exe"),
            "\"C:\\Program Files\\ShotCraft\\shotcraft.exe\""
        );
        assert_eq!(quote_path(r"C:\Tools\paint.exe"), r"C:\Tools\paint.exe");
        // Already quoted by the user: left alone.
        assert_eq!(quote_path(r#""C:\a b\x.exe""#), r#""C:\a b\x.exe""#);
    }
}
