use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::error::AppError;

pub fn start_gui() -> Result<Child, AppError> {
    Command::new("cc-switch")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| AppError::GuiUnavailable(format!("cannot start cc-switch: {error}")))
}

pub fn is_gui_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|pid| pid.parse::<u32>().is_ok())
        })
        .any(|entry| {
            std::fs::read_link(entry.path().join("exe"))
                .is_ok_and(|executable| is_cc_switch_executable(&executable))
        })
}

fn is_cc_switch_executable(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("cc-switch")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_name_is_exact() {
        assert!(is_cc_switch_executable(Path::new("/usr/bin/cc-switch")));
        assert!(!is_cc_switch_executable(Path::new(
            "/usr/bin/cc-switch-cli"
        )));
    }
}
