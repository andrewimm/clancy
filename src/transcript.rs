//! Transcript parsing for Claude stream-json output
//!
//! Parses newline-delimited JSON from `claude -p --output-format stream-json`
//! into structured transcript data.

use serde::{Deserialize, Serialize};

/// A complete parsed transcript from a Claude task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    /// System initialization info (model, version, etc.)
    pub init: Option<SystemInit>,
    /// Ordered list of conversation messages
    pub messages: Vec<Message>,
    /// Final result of the task
    pub result: Option<TaskResult>,
}

/// System initialization message from Claude
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInit {
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub claude_code_version: Option<String>,
    pub cwd: Option<String>,
}

/// A message in the conversation (assistant response or tool usage)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    /// Text response from the assistant
    #[serde(rename = "text")]
    Text { text: String },
    /// Tool invocation by the assistant
    #[serde(rename = "tool_use")]
    ToolUse {
        tool_name: String,
        tool_id: String,
        input: serde_json::Value,
    },
    /// Result from a tool invocation
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_id: String,
        output: String,
        is_error: bool,
    },
}

/// Final result of a task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub success: bool,
    pub result_text: Option<String>,
    pub duration_ms: Option<u64>,
    pub total_cost_usd: Option<f64>,
    pub usage: Option<TokenUsage>,
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
}

impl Transcript {
    /// Parse newline-delimited JSON output into a structured transcript
    pub fn parse(output: &str) -> Self {
        let mut transcript = Transcript {
            init: None,
            messages: Vec::new(),
            result: None,
        };

        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Try to parse each line as JSON
            let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };

            // Get the message type
            let Some(msg_type) = json.get("type").and_then(|t| t.as_str()) else {
                continue;
            };

            match msg_type {
                "system" => {
                    if json.get("subtype").and_then(|s| s.as_str()) == Some("init") {
                        transcript.init = Some(SystemInit {
                            model: json.get("model").and_then(|v| v.as_str()).map(String::from),
                            session_id: json
                                .get("session_id")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            claude_code_version: json
                                .get("claude_code_version")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            cwd: json.get("cwd").and_then(|v| v.as_str()).map(String::from),
                        });
                    }
                }
                "assistant" => {
                    // Extract content from assistant messages
                    if let Some(content) = json.get("message").and_then(|m| m.get("content")) {
                        if let Some(arr) = content.as_array() {
                            for item in arr {
                                if let Some(item_type) = item.get("type").and_then(|t| t.as_str()) {
                                    match item_type {
                                        "text" => {
                                            if let Some(text) =
                                                item.get("text").and_then(|t| t.as_str())
                                            {
                                                transcript.messages.push(Message::Text {
                                                    text: text.to_string(),
                                                });
                                            }
                                        }
                                        "tool_use" => {
                                            let tool_name = item
                                                .get("name")
                                                .and_then(|n| n.as_str())
                                                .unwrap_or("unknown")
                                                .to_string();
                                            let tool_id = item
                                                .get("id")
                                                .and_then(|i| i.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let input = item
                                                .get("input")
                                                .cloned()
                                                .unwrap_or(serde_json::Value::Null);
                                            transcript.messages.push(Message::ToolUse {
                                                tool_name,
                                                tool_id,
                                                input,
                                            });
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                "user" => {
                    // Extract tool results from user messages
                    if let Some(content) = json.get("message").and_then(|m| m.get("content")) {
                        if let Some(arr) = content.as_array() {
                            for item in arr {
                                if item.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                                {
                                    let tool_id = item
                                        .get("tool_use_id")
                                        .and_then(|i| i.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let output = item
                                        .get("content")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let is_error = item
                                        .get("is_error")
                                        .and_then(|e| e.as_bool())
                                        .unwrap_or(false);
                                    transcript.messages.push(Message::ToolResult {
                                        tool_id,
                                        output,
                                        is_error,
                                    });
                                }
                            }
                        }
                    }
                }
                "result" => {
                    let success = json.get("subtype").and_then(|s| s.as_str()) == Some("success");
                    let result_text = json
                        .get("result")
                        .and_then(|r| r.as_str())
                        .map(String::from);
                    let duration_ms = json.get("duration_ms").and_then(|d| d.as_u64());
                    let total_cost_usd = json.get("total_cost_usd").and_then(|c| c.as_f64());

                    let usage = json.get("usage").map(|u| TokenUsage {
                        input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        cache_read_tokens: u
                            .get("cache_read_input_tokens")
                            .and_then(|v| v.as_u64()),
                        cache_creation_tokens: u
                            .get("cache_creation_input_tokens")
                            .and_then(|v| v.as_u64()),
                    });

                    transcript.result = Some(TaskResult {
                        success,
                        result_text,
                        duration_ms,
                        total_cost_usd,
                        usage,
                    });
                }
                _ => {}
            }
        }

        transcript
    }

    /// Generate a summary of the transcript suitable for context injection
    pub fn generate_summary(&self) -> String {
        let mut summary = String::new();

        // If we have a final result text, use that as the primary summary
        if let Some(ref result) = self.result {
            if let Some(ref text) = result.result_text {
                // Truncate long results
                if text.len() > 200 {
                    summary.push_str(&text[..200]);
                    summary.push_str("...");
                } else {
                    summary.push_str(text);
                }
            }
        }

        // If no result text, try to summarize from messages
        if summary.is_empty() {
            for msg in &self.messages {
                if let Message::Text { text } = msg {
                    // Take the first text response as summary
                    if text.len() > 200 {
                        summary.push_str(&text[..200]);
                        summary.push_str("...");
                    } else {
                        summary.push_str(text);
                    }
                    break;
                }
            }
        }

        if summary.is_empty() {
            summary.push_str("(no summary available)");
        }

        summary
    }

    /// Get a list of tools used in this transcript
    pub fn tools_used(&self) -> Vec<String> {
        self.messages
            .iter()
            .filter_map(|msg| {
                if let Message::ToolUse { tool_name, .. } = msg {
                    Some(tool_name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get total cost in USD, if available
    pub fn total_cost(&self) -> Option<f64> {
        self.result.as_ref().and_then(|r| r.total_cost_usd)
    }

    /// Get duration in milliseconds, if available
    pub fn duration_ms(&self) -> Option<u64> {
        self.result.as_ref().and_then(|r| r.duration_ms)
    }

    /// Check if the task completed successfully
    pub fn succeeded(&self) -> bool {
        self.result.as_ref().map(|r| r.success).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_transcript() {
        let output = r#"{"type":"system","subtype":"init","model":"claude-opus-4-5-20251101","session_id":"abc123","claude_code_version":"2.1.12","cwd":"/test"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Hello, I can help with that."}]}}
{"type":"result","subtype":"success","result":"Done","duration_ms":1500,"total_cost_usd":0.01,"usage":{"input_tokens":100,"output_tokens":50}}"#;

        let transcript = Transcript::parse(output);

        // Check init
        assert!(transcript.init.is_some());
        let init = transcript.init.unwrap();
        assert_eq!(init.model, Some("claude-opus-4-5-20251101".to_string()));
        assert_eq!(init.session_id, Some("abc123".to_string()));

        // Check messages
        assert_eq!(transcript.messages.len(), 1);
        if let Message::Text { text } = &transcript.messages[0] {
            assert_eq!(text, "Hello, I can help with that.");
        } else {
            panic!("Expected text message");
        }

        // Check result
        assert!(transcript.result.is_some());
        let result = transcript.result.unwrap();
        assert!(result.success);
        assert_eq!(result.result_text, Some("Done".to_string()));
        assert_eq!(result.duration_ms, Some(1500));
        assert_eq!(result.total_cost_usd, Some(0.01));
    }

    #[test]
    fn test_parse_with_tool_use() {
        let output = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","id":"tool_123","input":{"file_path":"/test.txt"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool_123","content":"file contents here"}]}}
{"type":"result","subtype":"success","result":"Read the file"}"#;

        let transcript = Transcript::parse(output);

        assert_eq!(transcript.messages.len(), 2);

        // Check tool use
        if let Message::ToolUse {
            tool_name, tool_id, ..
        } = &transcript.messages[0]
        {
            assert_eq!(tool_name, "Read");
            assert_eq!(tool_id, "tool_123");
        } else {
            panic!("Expected tool use message");
        }

        // Check tool result
        if let Message::ToolResult {
            tool_id, output, ..
        } = &transcript.messages[1]
        {
            assert_eq!(tool_id, "tool_123");
            assert_eq!(output, "file contents here");
        } else {
            panic!("Expected tool result message");
        }

        assert_eq!(transcript.tools_used(), vec!["Read"]);
    }

    #[test]
    fn test_generate_summary() {
        let output =
            r#"{"type":"result","subtype":"success","result":"Fixed the authentication bug"}"#;

        let transcript = Transcript::parse(output);
        assert_eq!(
            transcript.generate_summary(),
            "Fixed the authentication bug"
        );
    }

    #[test]
    fn test_empty_output() {
        let transcript = Transcript::parse("");

        assert!(transcript.init.is_none());
        assert!(transcript.messages.is_empty());
        assert!(transcript.result.is_none());
        assert!(!transcript.succeeded());
    }

    #[test]
    fn test_malformed_json_lines_skipped() {
        let output = r#"not json
{"type":"result","subtype":"success","result":"Done"}
also not json"#;

        let transcript = Transcript::parse(output);

        // Should still parse the valid line
        assert!(transcript.result.is_some());
        assert!(transcript.succeeded());
    }
}
