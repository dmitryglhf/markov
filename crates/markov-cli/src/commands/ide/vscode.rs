use super::paths::{BaseDirs, Os};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// VS Code has no ACP client of its own, so the settings we write stay inert
/// until this third-party extension is present.
pub const EXTENSION_ID: &str = "formulahendry.acp-client";

pub fn extension_installed(dirs: &BaseDirs) -> bool {
    let prefix = format!("{EXTENSION_ID}-");
    let Ok(entries) = std::fs::read_dir(dirs.home.join(".vscode").join("extensions")) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with(prefix.as_str())
    })
}

/// The `code` launcher, which is not on PATH unless the user added it from the
/// command palette.
pub fn find_cli(dirs: &BaseDirs, os: Os) -> Option<PathBuf> {
    let bundled = match os {
        Os::Macos => vec![
            PathBuf::from("/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"),
            dirs.home
                .join("Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"),
        ],
        Os::Linux => vec![
            PathBuf::from("/usr/bin/code"),
            PathBuf::from("/usr/share/code/bin/code"),
            PathBuf::from("/snap/bin/code"),
        ],
        Os::Windows => vec![
            dirs.windows_local
                .join("Programs")
                .join("Microsoft VS Code")
                .join("bin")
                .join("code.cmd"),
            PathBuf::from(r"C:\Program Files\Microsoft VS Code\bin\code.cmd"),
        ],
    };

    bundled
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| find_on_path(os))
}

fn find_on_path(os: Os) -> Option<PathBuf> {
    let name = if os == Os::Windows {
        "code.cmd"
    } else {
        "code"
    };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(name))
                .collect::<Vec<_>>()
        })
        .find(|candidate| candidate.is_file())
}

pub fn install_extension(cli: &Path, vsix: Option<&Path>) -> Result<()> {
    let target = match vsix {
        Some(path) => path.as_os_str().to_os_string(),
        None => EXTENSION_ID.into(),
    };

    let status = Command::new(cli)
        .arg("--install-extension")
        .arg(&target)
        .arg("--force")
        .status()
        .with_context(|| format!("could not run {}", cli.display()))?;

    if !status.success() {
        bail!(
            "{} --install-extension {} failed",
            cli.display(),
            target.to_string_lossy()
        );
    }
    Ok(())
}
