use std::str::FromStr;

use anyhow::Result;
use rmcp::{transport::stdio, ServiceExt};

#[derive(Clone, Debug)]
pub enum McpCommand {
    AutoVisualiser,
    ComputerController,
    Memory,
    Tutorial,
}

impl FromStr for McpCommand {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace(' ', "").as_str() {
            "autovisualiser" => Ok(McpCommand::AutoVisualiser),
            "computercontroller" => Ok(McpCommand::ComputerController),
            "memory" => Ok(McpCommand::Memory),
            "tutorial" => Ok(McpCommand::Tutorial),
            _ => Err(format!("Invalid command: {}", s)),
        }
    }
}

impl McpCommand {
    /// Every server compiled into the binary. Nothing seeds these into the
    /// config file, so this list is what lets a dialog offer them at all.
    pub const ALL: [McpCommand; 4] = [
        McpCommand::AutoVisualiser,
        McpCommand::ComputerController,
        McpCommand::Memory,
        McpCommand::Tutorial,
    ];

    pub fn name(&self) -> &str {
        match self {
            McpCommand::AutoVisualiser => "autovisualiser",
            McpCommand::ComputerController => "computercontroller",
            McpCommand::Memory => "memory",
            McpCommand::Tutorial => "tutorial",
        }
    }

    pub fn title(&self) -> &str {
        match self {
            McpCommand::AutoVisualiser => "Auto Visualiser",
            McpCommand::ComputerController => "Computer Controller",
            McpCommand::Memory => "Memory",
            McpCommand::Tutorial => "Tutorial",
        }
    }

    pub fn description(&self) -> &str {
        match self {
            McpCommand::AutoVisualiser => "Data visualisation and UI generation tools",
            McpCommand::ComputerController => {
                "Controls for webscraping, file caching, and automations"
            }
            McpCommand::Memory => "Tools to save and retrieve durable memories",
            McpCommand::Tutorial => "Access interactive tutorials and guides",
        }
    }
}

pub async fn serve<S>(server: S) -> Result<()>
where
    S: rmcp::ServerHandler,
{
    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    })?;

    service.waiting().await?;

    Ok(())
}
