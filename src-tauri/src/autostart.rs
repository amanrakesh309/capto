//! Optional "start at login" support.
//!
//! Implemented by writing a standard XDG autostart entry instead of pulling in
//! another plugin, so the behaviour is inspectable and easy to support.

use std::path::PathBuf;

pub const FILE_NAME: &str = "capto.desktop";

fn autostart_dir() -> Option<PathBuf> {
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config).join("autostart"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/autostart"))
}

pub fn desktop_path() -> Option<PathBuf> {
    autostart_dir().map(|dir| dir.join(FILE_NAME))
}

pub fn is_enabled() -> bool {
    desktop_path().map(|path| path.exists()).unwrap_or(false)
}

/// Quote an `Exec=` argument following the Desktop Entry specification.
pub fn quote_exec_argument(argument: &str) -> String {
    let needs_quotes = argument.is_empty()
        || argument.chars().any(|c| {
            matches!(
                c,
                ' ' | '\t' | '"' | '\'' | '\\' | '$' | '`' | ';' | '&' | '|'
            )
        });
    let escaped = argument.replace('\\', "\\\\").replace('"', "\\\"");
    if needs_quotes {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

pub fn desktop_entry(executable: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Capto\n\
         Comment=Lightweight continuous screenshot and snip tool\n\
         Exec={}\n\
         Icon=capto\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         Categories=Utility;\n",
        quote_exec_argument(executable)
    )
}

/// The binary an autostart entry should launch.
///
/// Inside an AppImage, `current_exe()` points into the temporary mount
/// (`/tmp/.mount_…`), which disappears on logout. `$APPIMAGE` is the stable path
/// the user actually started. A regular installation has no `$APPIMAGE`, so the
/// executable path is used as-is.
fn executable_path() -> Result<PathBuf, String> {
    match std::env::var_os("APPIMAGE") {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => std::env::current_exe()
            .map_err(|err| format!("could not locate the application binary: {err}")),
    }
}

/// Enable or disable the autostart entry. Errors are plain strings because they
/// are shown verbatim next to the checkbox.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let Some(path) = desktop_path() else {
        return Err("could not find the autostart folder (HOME is not set)".into());
    };
    if !enabled {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("could not remove {}: {err}", path.display())),
        };
    }
    let exe = executable_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
    }
    std::fs::write(&path, desktop_entry(&exe.display().to_string()))
        .map_err(|err| format!("could not write {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_contains_the_executable() {
        let entry = desktop_entry("/usr/bin/capto");
        assert!(entry.starts_with("[Desktop Entry]"));
        assert!(entry.contains("Exec=/usr/bin/capto"));
        assert!(entry.contains("Type=Application"));
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        let entry = desktop_entry("/home/aman/My Apps/capto tool");
        assert!(entry.contains("Exec=\"/home/aman/My Apps/capto tool\""));
    }

    #[test]
    fn quotes_are_escaped() {
        assert_eq!(quote_exec_argument("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_exec_argument("plain"), "plain");
    }
}
