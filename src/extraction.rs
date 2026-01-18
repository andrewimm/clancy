//! Automated note extraction using Claude API
//!
//! After each task, sends the transcript to Claude for analysis and
//! extracts structured notes to maintain context across sessions.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{load_config, Config};
use crate::project::Project;
use crate::transcript::Transcript;

/// Result of note extraction
#[derive(Debug, Default)]
pub struct ExtractionResult {
    pub architecture: Option<String>,
    pub decisions: Option<String>,
    pub failures: Option<String>,
    pub plan: Option<String>,
}

impl ExtractionResult {
    /// Returns true if any notes were extracted
    pub fn has_updates(&self) -> bool {
        self.architecture.is_some()
            || self.decisions.is_some()
            || self.failures.is_some()
            || self.plan.is_some()
    }

    /// Returns a summary of what was updated
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.architecture.is_some() {
            parts.push("architecture");
        }
        if self.decisions.is_some() {
            parts.push("decisions");
        }
        if self.failures.is_some() {
            parts.push("failures");
        }
        if self.plan.is_some() {
            parts.push("plan");
        }
        if parts.is_empty() {
            "no updates".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Claude API message format
#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ApiMessage>,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

/// Extracts notes from a task transcript using Claude API
pub async fn extract_notes(
    project: &Project,
    transcript: &Transcript,
    prompt: &str,
) -> Result<ExtractionResult> {
    let config = load_config()?;

    // Get API key from environment
    let api_key = std::env::var(&config.claude.api_key_env).with_context(|| {
        format!(
            "API key not found. Set {} environment variable.",
            config.claude.api_key_env
        )
    })?;

    // Build the extraction prompt
    let extraction_prompt = build_extraction_prompt(project, transcript, prompt)?;

    // Call Claude API
    let response_text = call_claude_api(&api_key, &config, &extraction_prompt).await?;

    // Parse the response
    parse_extraction_response(&response_text)
}

/// Builds the note extraction prompt with current notes and transcript
fn build_extraction_prompt(
    project: &Project,
    transcript: &Transcript,
    task_prompt: &str,
) -> Result<String> {
    let architecture = project.read_notes("architecture")?;
    let decisions = project.read_notes("decisions")?;
    let failures = project.read_notes("failures")?;
    let plan = project.read_notes("plan")?;

    // Format transcript for inclusion
    let transcript_text = format_transcript_for_extraction(transcript, task_prompt);

    Ok(format!(
        r#"You are extracting structured notes from a coding task transcript.
The developer will use these notes to maintain context across tasks and sessions.

Analyze the transcript and produce updates to four note categories.
For each category, output ONLY new information not already present in existing notes.
If nothing new was learned for a category, output "NO_UPDATES".

## Categories

### ARCHITECTURE
Patterns, conventions, and structural knowledge about the codebase.
Examples: "Uses repository pattern", "Handlers follow extract-validate-execute",
"Tests use TestDb harness from tests/common/".

### DECISIONS
Choices made during this task with rationale.
Format: "- [YYYY-MM-DD] Chose X over Y because Z"
Include rejected alternatives when discussed.

### FAILURES
Things that didn't work, error messages encountered, dead ends.
Format: "- Don't try X — causes Y because Z"
This is critical for avoiding repeated mistakes.

### PLAN
Current state of the work, immediate next steps, open questions.
This REPLACES (not appends to) the previous plan.
Format as a brief status + bullet list of TODOs.

---

## Existing Notes

<architecture>
{architecture}
</architecture>

<decisions>
{decisions}
</decisions>

<failures>
{failures}
</failures>

<plan>
{plan}
</plan>

---

## Task Transcript

<transcript>
{transcript_text}
</transcript>

---

Output format (use exactly these headers):

### ARCHITECTURE
[new items only, or NO_UPDATES]

### DECISIONS
[new items only, or NO_UPDATES]

### FAILURES
[new items only, or NO_UPDATES]

### PLAN
[full replacement content]"#,
        architecture = if architecture.is_empty() {
            "(empty)"
        } else {
            &architecture
        },
        decisions = if decisions.is_empty() {
            "(empty)"
        } else {
            &decisions
        },
        failures = if failures.is_empty() {
            "(empty)"
        } else {
            &failures
        },
        plan = if plan.is_empty() { "(empty)" } else { &plan },
        transcript_text = transcript_text,
    ))
}

/// Formats the transcript for inclusion in the extraction prompt
fn format_transcript_for_extraction(transcript: &Transcript, task_prompt: &str) -> String {
    let mut output = String::new();

    // Include the original task prompt
    output.push_str(&format!("Task: {}\n\n", task_prompt));

    // Include model info if available
    if let Some(ref init) = transcript.init {
        if let Some(ref model) = init.model {
            output.push_str(&format!("Model: {}\n", model));
        }
    }

    output.push_str("---\n\n");

    // Include messages
    for msg in &transcript.messages {
        match msg {
            crate::transcript::Message::Text { text } => {
                output.push_str("Assistant:\n");
                output.push_str(text);
                output.push_str("\n\n");
            }
            crate::transcript::Message::ToolUse {
                tool_name, input, ..
            } => {
                output.push_str(&format!("Tool: {}\n", tool_name));
                // Include relevant input for context (truncate if too long)
                let input_str = serde_json::to_string_pretty(input).unwrap_or_default();
                if input_str.len() < 500 {
                    output.push_str(&format!("Input: {}\n", input_str));
                }
                output.push('\n');
            }
            crate::transcript::Message::ToolResult {
                output: result,
                is_error,
                ..
            } => {
                if *is_error {
                    output.push_str(&format!("Error: {}\n\n", truncate(result, 500)));
                } else {
                    // Only include short tool results
                    if result.len() < 200 {
                        output.push_str(&format!("Result: {}\n\n", result));
                    }
                }
            }
        }
    }

    // Include final result
    if let Some(ref result) = transcript.result {
        if let Some(ref text) = result.result_text {
            output.push_str("---\n\n");
            output.push_str(&format!("Final result: {}\n", text));
        }
        if !result.success {
            output.push_str("(Task failed)\n");
        }
    }

    output
}

/// Calls the Claude API with the extraction prompt
async fn call_claude_api(api_key: &str, config: &Config, prompt: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("Failed to create HTTP client")?;

    let request = ApiRequest {
        model: config.claude.model.clone(),
        max_tokens: 2048,
        messages: vec![ApiMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .context("Failed to connect to Claude API (check network connection)")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        // Provide helpful error messages for common issues
        let hint = match status.as_u16() {
            401 => " (check your API key)",
            429 => " (rate limited, try again later)",
            500..=599 => " (API server error, try again later)",
            _ => "",
        };

        bail!("Claude API error ({}){}: {}", status, hint, body);
    }

    let api_response: ApiResponse = response
        .json()
        .await
        .context("Failed to parse Claude API response")?;

    // Extract text from response
    let text = api_response
        .content
        .iter()
        .filter(|c| c.content_type == "text")
        .filter_map(|c| c.text.as_deref())
        .collect::<Vec<_>>()
        .join("");

    if text.is_empty() {
        bail!("Claude API returned empty response");
    }

    Ok(text)
}

/// Parses the extraction response into structured notes
fn parse_extraction_response(response: &str) -> Result<ExtractionResult> {
    let mut result = ExtractionResult::default();

    // Find each section by header
    let sections = [
        ("### ARCHITECTURE", "architecture"),
        ("### DECISIONS", "decisions"),
        ("### FAILURES", "failures"),
        ("### PLAN", "plan"),
    ];

    for (i, (header, name)) in sections.iter().enumerate() {
        if let Some(start) = response.find(header) {
            let content_start = start + header.len();

            // Find the end (next header or end of string)
            let end = sections
                .iter()
                .skip(i + 1)
                .filter_map(|(h, _)| response[content_start..].find(h))
                .map(|pos| content_start + pos)
                .next()
                .unwrap_or(response.len());

            let content = response[content_start..end].trim();

            // Check if there are updates
            if !content.is_empty()
                && content.to_uppercase() != "NO_UPDATES"
                && !content.starts_with("NO_UPDATES")
            {
                match *name {
                    "architecture" => result.architecture = Some(content.to_string()),
                    "decisions" => result.decisions = Some(content.to_string()),
                    "failures" => result.failures = Some(content.to_string()),
                    "plan" => result.plan = Some(content.to_string()),
                    _ => {}
                }
            }
        }
    }

    Ok(result)
}

/// Applies extraction results to project notes
pub fn apply_extraction(project: &Project, extraction: &ExtractionResult) -> Result<()> {
    // Architecture, decisions, and failures are appended
    if let Some(ref content) = extraction.architecture {
        project.append_notes("architecture", content)?;
    }
    if let Some(ref content) = extraction.decisions {
        project.append_notes("decisions", content)?;
    }
    if let Some(ref content) = extraction.failures {
        project.append_notes("failures", content)?;
    }

    // Plan is replaced entirely
    if let Some(ref content) = extraction.plan {
        project.write_notes("plan", content)?;
    }

    Ok(())
}

/// Truncates a string to a maximum length
fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_extraction_response() {
        let response = r#"
### ARCHITECTURE
- Uses async/await pattern for API calls
- Configuration loaded from TOML files

### DECISIONS
NO_UPDATES

### FAILURES
- Don't use blocking HTTP client in async context

### PLAN
Completed Phase 3 implementation.
- [ ] Add integration tests
- [ ] Update documentation
"#;

        let result = parse_extraction_response(response).unwrap();

        assert!(result.architecture.is_some());
        assert!(result.architecture.unwrap().contains("async/await"));

        assert!(result.decisions.is_none()); // NO_UPDATES

        assert!(result.failures.is_some());
        assert!(result.failures.unwrap().contains("blocking HTTP"));

        assert!(result.plan.is_some());
        assert!(result.plan.unwrap().contains("Phase 3"));
    }

    #[test]
    fn test_parse_all_no_updates() {
        let response = r#"
### ARCHITECTURE
NO_UPDATES

### DECISIONS
NO_UPDATES

### FAILURES
NO_UPDATES

### PLAN
NO_UPDATES
"#;

        let result = parse_extraction_response(response).unwrap();
        assert!(!result.has_updates());
    }

    #[test]
    fn test_extraction_result_summary() {
        let mut result = ExtractionResult::default();
        assert_eq!(result.summary(), "no updates");

        result.architecture = Some("test".to_string());
        result.plan = Some("test".to_string());
        assert_eq!(result.summary(), "architecture, plan");
    }
}
