use anyhow::Result;
use etcetera::base_strategy::{Apple, BaseStrategy, Windows, Xdg};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Macos,
    Linux,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }
}

/// The directories every path below is derived from.
///
/// Held as plain data so the resolver stays a pure function: the tests can walk
/// all three platforms on whatever machine they happen to run on.
#[derive(Clone, Debug)]
pub struct BaseDirs {
    pub home: PathBuf,
    pub xdg_config: PathBuf,
    pub apple_data: PathBuf,
    /// %APPDATA%
    pub windows_config: PathBuf,
    /// %LOCALAPPDATA%, which etcetera calls the Windows cache directory.
    pub windows_local: PathBuf,
}

impl BaseDirs {
    /// Reads HOME, XDG_CONFIG_HOME and APPDATA through etcetera rather than
    /// rebuilding them from $HOME, which would miss redirected folders on
    /// domain-joined machines.
    pub fn detect() -> Result<Self> {
        let windows = Windows::new()?;
        Ok(Self {
            home: etcetera::home_dir()?,
            xdg_config: Xdg::new()?.config_dir(),
            apple_data: Apple::new()?.data_dir(),
            windows_config: windows.config_dir(),
            windows_local: windows.cache_dir(),
        })
    }
}

impl BaseDirs {
    /// Zed keeps a plain ~/.config on macOS rather than moving into Library.
    pub fn zed_settings(&self, os: Os) -> PathBuf {
        match os {
            Os::Macos => self.home.join(".config").join("zed").join("settings.json"),
            Os::Linux => self.xdg_config.join("zed").join("settings.json"),
            Os::Windows => self.windows_config.join("Zed").join("settings.json"),
        }
    }

    /// Our own file, so one entry configures CLion, IDEA and Rider at once.
    /// The Windows location is an assumption: the docs name only the home path.
    pub fn jetbrains_acp(&self, _os: Os) -> PathBuf {
        self.home.join(".jetbrains").join("acp.json")
    }

    pub fn vscode_settings(&self, os: Os) -> PathBuf {
        let base = match os {
            Os::Macos => self.apple_data.clone(),
            Os::Linux => self.xdg_config.clone(),
            Os::Windows => self.windows_config.clone(),
        };
        base.join("Code").join("User").join("settings.json")
    }

    /// Where install.sh and install.ps1 put the CLI. Deliberately not
    /// `current_exe`: on macOS that resolves through the symlink into
    /// Markov.app, a path that changes with every update.
    pub fn cli_binary(&self, os: Os) -> PathBuf {
        match os {
            Os::Windows => self.home.join("markov").join("markov.exe"),
            _ => self.home.join(".local").join("bin").join("markov"),
        }
    }

    /// Directories that mean the IDE has been run at least once.
    pub fn zed_marker(&self, os: Os) -> PathBuf {
        self.zed_settings(os)
            .parent()
            .expect("settings path has a parent")
            .to_path_buf()
    }

    pub fn vscode_marker(&self, os: Os) -> PathBuf {
        self.vscode_settings(os)
            .parent()
            .expect("settings path has a parent")
            .to_path_buf()
    }

    pub fn jetbrains_marker(&self, os: Os) -> PathBuf {
        match os {
            Os::Macos => self.apple_data.join("JetBrains"),
            Os::Linux => self.xdg_config.join("JetBrains"),
            Os::Windows => self.windows_config.join("JetBrains"),
        }
    }
}
