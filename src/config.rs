use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Global Clancy configuration
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub claude: ClaudeConfig,
    #[serde(default)]
    pub extraction: ExtractionConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub repl: ReplConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaudeConfig {
    /// Environment variable containing the API key
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
    /// Model for note extraction
    #[serde(default = "default_model")]
    pub model: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractionConfig {
    /// Max tokens for transcript before truncation
    #[serde(default = "default_max_transcript_tokens")]
    pub max_transcript_tokens: usize,
    /// Include tool outputs in transcript
    #[serde(default = "default_true")]
    pub include_tool_outputs: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Max tokens for compiled context
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    /// Include notes from parent projects
    #[serde(default = "default_true")]
    pub include_parent_notes: bool,
    /// Conversation continuity mode: fresh | summary | full
    #[serde(default = "default_conversation_mode")]
    pub conversation_mode: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplConfig {
    /// Editor for /notes command
    #[serde(default = "default_editor")]
    pub editor: String,
    /// Prompt style: project | minimal
    #[serde(default = "default_prompt_style")]
    pub prompt_style: String,
}

fn default_api_key_env() -> String {
    "ANTHROPIC_API_KEY".to_string()
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

fn default_max_transcript_tokens() -> usize {
    100000
}

fn default_max_context_tokens() -> usize {
    12000
}

fn default_true() -> bool {
    true
}

fn default_conversation_mode() -> String {
    "summary".to_string()
}

fn default_editor() -> String {
    std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string())
}

fn default_prompt_style() -> String {
    "project".to_string()
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            api_key_env: default_api_key_env(),
            model: default_model(),
        }
    }
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            max_transcript_tokens: default_max_transcript_tokens(),
            include_tool_outputs: true,
        }
    }
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: default_max_context_tokens(),
            include_parent_notes: true,
            conversation_mode: default_conversation_mode(),
        }
    }
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            editor: default_editor(),
            prompt_style: default_prompt_style(),
        }
    }
}

/// Returns the Clancy config directory (~/.config/clancy/)
pub fn config_dir() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .context("Could not determine config directory")?
        .join("clancy");
    Ok(config_dir)
}

/// Returns the projects directory (~/.config/clancy/projects/)
pub fn projects_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("projects"))
}

/// Returns the config file path (~/.config/clancy/config.toml)
pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Ensures the config directory structure exists
pub fn ensure_config_dir() -> Result<()> {
    let config = config_dir()?;
    let projects = projects_dir()?;

    std::fs::create_dir_all(&config)
        .with_context(|| format!("Failed to create config directory: {:?}", config))?;
    std::fs::create_dir_all(&projects)
        .with_context(|| format!("Failed to create projects directory: {:?}", projects))?;

    Ok(())
}

/// Loads the config, creating default if it doesn't exist
pub fn load_config() -> Result<Config> {
    let config_path = config_file()?;

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
        let config: Config =
            toml::from_str(&content).with_context(|| "Failed to parse config file")?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = Config::default();
        assert_eq!(config.claude.api_key_env, "ANTHROPIC_API_KEY");
        assert_eq!(config.context.conversation_mode, "summary");
    }

    #[test]
    fn test_config_roundtrip() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.claude.model, config.claude.model);
    }
}
