use super::paths::{BaseDirs, Os};
use clap::ValueEnum;
use jsonc_parser::cst::CstInputValue;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TargetSelector {
    Zed,
    #[value(alias = "jb")]
    Jetbrains,
    #[value(alias = "code")]
    Vscode,
    All,
}

impl TargetSelector {
    pub fn expand(self) -> Vec<&'static Target> {
        match self {
            TargetSelector::Zed => vec![&ZED],
            TargetSelector::Jetbrains => vec![&JETBRAINS],
            TargetSelector::Vscode => vec![&VSCODE],
            TargetSelector::All => vec![&ZED, &JETBRAINS, &VSCODE],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetId {
    Zed,
    Jetbrains,
    Vscode,
}

pub struct Target {
    pub id: TargetId,
    pub label: &'static str,
    /// Where the agent entries are nested. VS Code settings are a flat map, so
    /// its container is the single dotted key the ACP extension declares.
    pub container: &'static [&'static str],
}

pub static ZED: Target = Target {
    id: TargetId::Zed,
    label: "Zed",
    container: &["agent_servers"],
};

pub static JETBRAINS: Target = Target {
    id: TargetId::Jetbrains,
    label: "JetBrains IDEs",
    container: &["agent_servers"],
};

pub static VSCODE: Target = Target {
    id: TargetId::Vscode,
    label: "VS Code",
    container: &["acp.agents"],
};

impl Target {
    pub fn config_path(&self, dirs: &BaseDirs, os: Os) -> PathBuf {
        match self.id {
            TargetId::Zed => dirs.zed_settings(os),
            TargetId::Jetbrains => dirs.jetbrains_acp(os),
            TargetId::Vscode => dirs.vscode_settings(os),
        }
    }

    /// A directory the IDE creates on first run. Our JetBrains file is our own,
    /// so its absence says nothing — we look for the vendor directory instead.
    pub fn is_installed(&self, dirs: &BaseDirs, os: Os) -> bool {
        let marker = match self.id {
            TargetId::Zed => dirs.zed_marker(os),
            TargetId::Jetbrains => dirs.jetbrains_marker(os),
            TargetId::Vscode => dirs.vscode_marker(os),
        };
        marker.is_dir()
    }

    pub fn entry(&self, command: &str) -> CstInputValue {
        let string = |s: &str| CstInputValue::String(s.to_string());
        let args = CstInputValue::Array(vec![string("acp")]);
        match self.id {
            TargetId::Zed => CstInputValue::Object(vec![
                ("type".to_string(), string("custom")),
                ("command".to_string(), string(command)),
                ("args".to_string(), args),
            ]),
            TargetId::Jetbrains | TargetId::Vscode => CstInputValue::Object(vec![
                ("command".to_string(), string(command)),
                ("args".to_string(), args),
                ("env".to_string(), CstInputValue::Object(vec![])),
            ]),
        }
    }
}
