use anyhow::{Context, Result};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::{self, load_config};
use crate::extraction::{apply_extraction, extract_notes};
use crate::project::{Project, NOTE_CATEGORIES};
use crate::transcript::Transcript;

/// Conversation continuity mode
#[derive(Clone, Copy, PartialEq)]
enum ConversationMode {
    /// Fresh context each task (only notes, no history)
    Fresh,
    /// Include summaries of prior tasks (default)
    Summary,
    /// Include full conversation from prior tasks
    Full,
}

/// Task record for conversation continuity
struct TaskRecord {
    number: u32,
    prompt: String,
    summary: String,
    /// Full raw output for /continue mode
    raw_output: String,
}

/// REPL session state
struct Session {
    project: Project,
    task_history: Vec<TaskRecord>,
    working_dir: PathBuf,
    /// Current conversation mode
    conversation_mode: ConversationMode,
}

impl Session {
    fn new(project: Project) -> Result<Self> {
        let working_dir = std::env::current_dir()?;
        // Load conversation mode from config
        let config = load_config()?;
        let conversation_mode = match config.context.conversation_mode.as_str() {
            "fresh" => ConversationMode::Fresh,
            "full" => ConversationMode::Full,
            _ => ConversationMode::Summary,
        };
        Ok(Self {
            project,
            task_history: Vec::new(),
            working_dir,
            conversation_mode,
        })
    }

    /// Compiles all notes into .claude/context.md
    /// Returns estimated token count
    fn compile_context(&self) -> Result<usize> {
        let config = load_config()?;
        let claude_dir = self.working_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir)?;

        let context_path = claude_dir.join("context.md");
        let mut content = String::new();
        let max_tokens = config.context.max_context_tokens;

        // Header
        content.push_str("<!-- CLANCY CONTEXT — AUTO-GENERATED -->\n");
        content.push_str(&format!(
            "<!-- Project: {} | Task: {} -->\n\n",
            self.project.metadata.name,
            self.task_history.len() + 1
        ));

        // Session context based on conversation mode
        if !self.task_history.is_empty() {
            match self.conversation_mode {
                ConversationMode::Fresh => {
                    // No session history included
                }
                ConversationMode::Summary => {
                    content.push_str("## Session Context\n\n");
                    content.push_str(&format!(
                        "This is task {} of an ongoing session. Prior tasks:\n",
                        self.task_history.len() + 1
                    ));
                    for task in &self.task_history {
                        content.push_str(&format!(
                            "{}. {} — {}\n",
                            task.number, task.prompt, task.summary
                        ));
                    }
                    content.push('\n');
                }
                ConversationMode::Full => {
                    content.push_str("## Full Conversation History\n\n");
                    content.push_str(&format!(
                        "This is task {} of an ongoing session. Full prior conversation:\n\n",
                        self.task_history.len() + 1
                    ));
                    for task in &self.task_history {
                        content.push_str(&format!("### Task {}: {}\n\n", task.number, task.prompt));
                        // Include the full transcript, parsed for readability
                        let transcript = Transcript::parse(&task.raw_output);
                        for msg in &transcript.messages {
                            match msg {
                                crate::transcript::Message::Text { text } => {
                                    content.push_str(text);
                                    content.push_str("\n\n");
                                }
                                crate::transcript::Message::ToolUse { tool_name, .. } => {
                                    content.push_str(&format!("[Used tool: {}]\n\n", tool_name));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Include parent project notes if configured and parent exists
        if config.context.include_parent_notes {
            if let Some(ref parent_name) = self.project.metadata.parent {
                if let Ok(parent) = Project::open(parent_name) {
                    let parent_arch = parent.read_notes("architecture")?;
                    if !parent_arch.trim().is_empty() {
                        content
                            .push_str(&format!("## Inherited Context (from {})\n\n", parent_name));
                        content.push_str(&parent_arch);
                        content.push_str("\n\n");
                    }
                }
            }
        }

        // Architecture notes
        let arch = self.project.read_notes("architecture")?;
        if !arch.trim().is_empty() {
            content.push_str("## Architectural Context\n\n");
            content.push_str(&arch);
            content.push_str("\n\n");
        }

        // Decisions
        let decisions = self.project.read_notes("decisions")?;
        if !decisions.trim().is_empty() {
            content.push_str("## Key Decisions\n\n");
            content.push_str(&decisions);
            content.push_str("\n\n");
        }

        // Failures (critical for avoiding repeated mistakes)
        let failures = self.project.read_notes("failures")?;
        if !failures.trim().is_empty() {
            content.push_str("## Known Pitfalls\n\n");
            content.push_str(&failures);
            content.push_str("\n\n");
        }

        // Current plan
        let plan = self.project.read_notes("plan")?;
        if !plan.trim().is_empty() {
            content.push_str("## Current Plan\n\n");
            content.push_str(&plan);
            content.push_str("\n\n");
        }

        // Footer
        content.push_str("---\n");
        content.push_str(
            "When you complete work or encounter a problem, state it clearly for continuity.\n",
        );

        // Apply token budget (rough estimate: 4 chars per token)
        let estimated_tokens = content.len() / 4;
        if estimated_tokens > max_tokens {
            // Truncate content, keeping header and footer
            let max_chars = max_tokens * 4;
            if content.len() > max_chars {
                let truncated = &content[..max_chars];
                // Find last complete section
                if let Some(pos) = truncated.rfind("\n## ") {
                    content = format!(
                        "{}\n\n[Context truncated due to token limit]\n",
                        &content[..pos]
                    );
                }
            }
        }

        let final_tokens = content.len() / 4;

        std::fs::write(&context_path, &content)
            .with_context(|| format!("Failed to write context file: {:?}", context_path))?;

        Ok(final_tokens)
    }

    /// Runs a task via claude -p
    fn run_task(&mut self, prompt: &str) -> Result<()> {
        // Compile context before task
        let token_count = self.compile_context()?;

        let task_num = self.project.next_task_number()?;
        println!(
            "\n[Task {}] Injecting context (~{} tokens)...\n",
            task_num, token_count
        );

        // Build the command
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg(prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .current_dir(&self.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = cmd
            .spawn()
            .context("Failed to start claude. Is it installed and in PATH?")?;

        // Stream output while capturing for later
        let stdout = child.stdout.take().expect("Failed to capture stdout");
        let reader = BufReader::new(stdout);
        let mut captured_output = String::new();

        for line in reader.lines() {
            let line = line?;
            captured_output.push_str(&line);
            captured_output.push('\n');

            // Parse stream-json format and display relevant content
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                // Handle different message types
                if let Some(msg_type) = json.get("type").and_then(|t| t.as_str()) {
                    match msg_type {
                        "assistant" => {
                            if let Some(content) =
                                json.get("message").and_then(|m| m.get("content"))
                            {
                                if let Some(arr) = content.as_array() {
                                    for item in arr {
                                        if let Some(text) =
                                            item.get("text").and_then(|t| t.as_str())
                                        {
                                            print!("{}", text);
                                            std::io::stdout().flush()?;
                                        }
                                    }
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = json.get("delta") {
                                if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                    print!("{}", text);
                                    std::io::stdout().flush()?;
                                }
                            }
                        }
                        "result" => {
                            // Task completed
                            if let Some(result) = json.get("result").and_then(|r| r.as_str()) {
                                println!("\n{}", result);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let status = child.wait()?;
        println!();

        if !status.success() {
            println!("[Task failed with exit code: {:?}]", status.code());
        }

        // Parse the captured output into a structured transcript
        let transcript = Transcript::parse(&captured_output);

        // Generate summary from transcript (better than just truncating prompt)
        let summary = if transcript.succeeded() {
            let auto_summary = transcript.generate_summary();
            // Prefer transcript summary if meaningful, fall back to prompt
            if auto_summary.len() > 20 && auto_summary != "(no summary available)" {
                truncate_string(&auto_summary, 80)
            } else {
                self.generate_basic_summary(prompt)
            }
        } else {
            format!("(failed) {}", truncate_string(prompt, 70))
        };

        // Record task with full output for /continue mode
        self.task_history.push(TaskRecord {
            number: task_num,
            prompt: truncate_string(prompt, 60),
            summary,
            raw_output: captured_output.clone(),
        });

        // Update project stats
        self.project.record_task()?;

        // Save task log with parsed transcript
        self.save_task_log(task_num, prompt, &captured_output, &transcript)?;

        // Print task completion summary
        let cost_str = transcript
            .total_cost()
            .map(|c| format!(" (${:.4})", c))
            .unwrap_or_default();
        let duration_str = transcript
            .duration_ms()
            .map(|d| format!(" in {:.1}s", d as f64 / 1000.0))
            .unwrap_or_default();
        println!("[Task {} complete{}{}]", task_num, duration_str, cost_str);

        // Run note extraction
        self.run_extraction(&transcript, prompt);

        println!();
        Ok(())
    }

    /// Generates a basic summary (placeholder for Phase 3 extraction)
    fn generate_basic_summary(&self, prompt: &str) -> String {
        // For Phase 1, just use a truncated version of the prompt
        // Phase 3 will use Claude API for proper extraction
        truncate_string(prompt, 80)
    }

    /// Saves the task log to disk with parsed transcript
    fn save_task_log(
        &self,
        task_num: u32,
        prompt: &str,
        output: &str,
        transcript: &Transcript,
    ) -> Result<()> {
        let tasks_dir = self.project.tasks_path();
        std::fs::create_dir_all(&tasks_dir)?;

        // Create a sanitized filename from the prompt
        let slug = create_slug(prompt);
        let filename = format!("{:03}-{}.json", task_num, slug);
        let path = tasks_dir.join(filename);

        let log = serde_json::json!({
            "task_number": task_num,
            "prompt": prompt,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "success": transcript.succeeded(),
            "duration_ms": transcript.duration_ms(),
            "cost_usd": transcript.total_cost(),
            "tools_used": transcript.tools_used(),
            "summary": transcript.generate_summary(),
            "transcript": transcript,
            "raw_output": output,
        });

        let content = serde_json::to_string_pretty(&log)?;
        std::fs::write(&path, content)?;

        Ok(())
    }

    /// Runs note extraction on the transcript
    fn run_extraction(&self, transcript: &Transcript, prompt: &str) {
        print!("Extracting notes...");
        std::io::stdout().flush().ok();

        // Create a tokio runtime for the async extraction
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                println!(" error creating runtime: {}", e);
                return;
            }
        };

        // Run the async extraction
        let result = rt.block_on(extract_notes(&self.project, transcript, prompt));

        match result {
            Ok(extraction) => {
                if extraction.has_updates() {
                    // Apply the extracted notes
                    if let Err(e) = apply_extraction(&self.project, &extraction) {
                        println!(" error applying notes: {}", e);
                    } else {
                        println!(" updated: {}", extraction.summary());
                    }
                } else {
                    println!(" no updates");
                }
            }
            Err(e) => {
                // Don't fail the task if extraction fails
                println!(" error: {}", e);
            }
        }
    }

    /// Compacts the session history into a single summary
    fn run_compact(&mut self) {
        if self.task_history.is_empty() {
            println!("No tasks to compact.");
            return;
        }

        print!("Compacting {} tasks...", self.task_history.len());
        std::io::stdout().flush().ok();

        // Create a summary of all tasks
        let mut summary_parts: Vec<String> = Vec::new();
        for task in &self.task_history {
            summary_parts.push(format!(
                "- Task {}: {} → {}",
                task.number, task.prompt, task.summary
            ));
        }
        let combined_summary = summary_parts.join("\n");

        // Clear history but keep a single summary record
        let task_count = self.task_history.len();
        self.task_history.clear();
        self.task_history.push(TaskRecord {
            number: 0, // Special marker for compacted history
            prompt: format!("(compacted {} tasks)", task_count),
            summary: combined_summary,
            raw_output: String::new(),
        });

        // Switch to summary mode
        self.conversation_mode = ConversationMode::Summary;

        println!(" done. Session history compacted.");
    }

    /// Runs phases from a plan file automatically
    fn run_auto(&mut self, file: Option<&str>) -> Result<()> {
        let file_path = file.unwrap_or("PLAN.md");
        let path = self.working_dir.join(file_path);

        if !path.exists() {
            anyhow::bail!(
                "Plan file not found: {}\nUsage: /auto [file.md]  (defaults to PLAN.md)",
                path.display()
            );
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read plan file: {}", path.display()))?;

        let phases = parse_plan_phases(&content);

        if phases.is_empty() {
            anyhow::bail!(
                "No phases found in {}.\nExpected format:\n\n## Phase 1: Title\nDescription of what to do.\n\n## Phase 2: Title\n...",
                file_path
            );
        }

        println!("\nFound {} phases in {}:\n", phases.len(), file_path);
        for (i, phase) in phases.iter().enumerate() {
            println!("  {}. {}", i + 1, phase.title);
        }
        println!("\nPress Enter to start, or Ctrl+C to cancel...");

        // Wait for user confirmation
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        for (i, phase) in phases.iter().enumerate() {
            println!("\n{}", "=".repeat(60));
            println!("Phase {}/{}: {}", i + 1, phases.len(), phase.title);
            println!("{}\n", "=".repeat(60));

            // Build the task prompt
            let prompt = format!("{}\n\n{}", phase.title, phase.description);

            // Run the task
            if let Err(e) = self.run_task(&prompt) {
                println!("\nPhase {} failed: {}", i + 1, e);
                println!("Stopping auto mode. Use /history to see completed phases.");
                return Ok(());
            }

            // If there are more phases, ask to continue
            if i < phases.len() - 1 {
                println!(
                    "\nPhase {} complete. Press Enter for next phase, or 'q' to stop...",
                    i + 1
                );
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if input.trim().eq_ignore_ascii_case("q") {
                    println!("Stopped. {} of {} phases complete.", i + 1, phases.len());
                    return Ok(());
                }
            }
        }

        println!("\n{}", "=".repeat(60));
        println!("All {} phases complete!", phases.len());
        println!("{}\n", "=".repeat(60));

        Ok(())
    }

    /// Handles REPL commands (those starting with /)
    fn handle_command(&mut self, cmd: &str) -> Result<bool> {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let command = parts.first().copied().unwrap_or("");

        match command {
            "/done" | "/quit" | "/q" => {
                println!(
                    "Session complete. {} tasks, notes updated.",
                    self.task_history.len()
                );
                return Ok(true); // Signal to exit
            }
            "/status" => {
                self.show_status()?;
            }
            "/notes" => {
                let category = parts.get(1).copied();
                self.edit_notes(category)?;
            }
            "/history" => {
                self.show_history();
            }
            "/continue" => {
                self.conversation_mode = ConversationMode::Full;
                println!(
                    "Switched to full conversation mode. Next task will include complete prior context."
                );
            }
            "/compact" => {
                self.run_compact();
            }
            "/fresh" => {
                self.conversation_mode = ConversationMode::Fresh;
                println!("Switched to fresh mode. Next task will only include notes, no session history.");
            }
            "/summary" => {
                self.conversation_mode = ConversationMode::Summary;
                println!(
                    "Switched to summary mode (default). Next task will include task summaries."
                );
            }
            "/auto" => {
                let file = parts.get(1).copied();
                if let Err(e) = self.run_auto(file) {
                    println!("Auto error: {}", e);
                }
            }
            "/help" => {
                self.show_help();
            }
            _ => {
                println!(
                    "Unknown command: {}. Type /help for available commands.",
                    command
                );
            }
        }

        Ok(false)
    }

    fn show_status(&self) -> Result<()> {
        println!("\n## Project: {}", self.project.metadata.name);
        println!(
            "Session tasks: {} | Total tasks: {}",
            self.task_history.len(),
            self.project.metadata.stats.total_tasks
        );

        // Show plan
        let plan = self.project.read_notes("plan")?;
        if !plan.trim().is_empty() {
            println!("\n## Current Plan\n{}", plan);
        }

        // Show recent decisions
        let decisions = self.project.read_notes("decisions")?;
        if !decisions.trim().is_empty() {
            let lines: Vec<&str> = decisions.lines().take(5).collect();
            if !lines.is_empty() {
                println!("\n## Recent Decisions");
                for line in lines {
                    println!("{}", line);
                }
            }
        }

        println!();
        Ok(())
    }

    fn edit_notes(&self, category: Option<&str>) -> Result<()> {
        let config = config::load_config()?;
        let editor = &config.repl.editor;

        let path = if let Some(cat) = category {
            if !NOTE_CATEGORIES.contains(&cat) {
                println!(
                    "Invalid category '{}'. Valid: {}",
                    cat,
                    NOTE_CATEGORIES.join(", ")
                );
                return Ok(());
            }
            self.project.notes_path(cat)
        } else {
            self.project.path.join("notes")
        };

        let status = Command::new(editor)
            .arg(&path)
            .status()
            .with_context(|| format!("Failed to open editor: {}", editor))?;

        if !status.success() {
            println!("Editor exited with error");
        }

        Ok(())
    }

    fn show_history(&self) {
        if self.task_history.is_empty() {
            println!("No tasks this session.");
            return;
        }

        println!("\n## Task History\n");
        for task in &self.task_history {
            println!("{}. {} — {}", task.number, task.prompt, task.summary);
        }
        println!();
    }

    fn show_help(&self) {
        let mode_str = match self.conversation_mode {
            ConversationMode::Fresh => "fresh",
            ConversationMode::Summary => "summary",
            ConversationMode::Full => "full",
        };
        println!(
            r#"
## Clancy REPL Commands

  <task description>   Run a task via Claude
  /status              Show current notes summary
  /notes [category]    Edit notes (architecture|decisions|failures|plan)
  /history             Show task history this session
  /auto [file]         Run phases from PLAN.md (or specified file)

## Conversation Modes (current: {})

  /continue            Switch to full mode (include complete prior context)
  /compact             Summarize history and start fresh
  /fresh               Switch to fresh mode (only notes, no history)
  /summary             Switch to summary mode (default)

## Session

  /done or /quit       Exit the session
  /help                Show this help
"#,
            mode_str
        );
    }
}

/// Starts the REPL session for a project
pub fn start_session(project_name: &str) -> Result<()> {
    let mut project = Project::open_or_create(project_name)?;
    project.record_session_start()?;

    println!(
        "Loading project: {} ({} prior sessions, {} tasks)",
        project.metadata.name,
        project.metadata.stats.total_sessions,
        project.metadata.stats.total_tasks
    );

    let mut session = Session::new(project)?;
    let token_count = session.compile_context()?;
    println!("Injected context (~{} tokens)\n", token_count);

    // Set up readline
    let mut rl = DefaultEditor::new()?;
    let history_path = config::config_dir()?.join("history.txt");
    let _ = rl.load_history(&history_path);

    let prompt = format!("{}> ", project_name);

    loop {
        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                rl.add_history_entry(line)?;

                if line.starts_with('/') {
                    match session.handle_command(line) {
                        Ok(should_exit) => {
                            if should_exit {
                                break;
                            }
                        }
                        Err(e) => println!("Error: {}", e),
                    }
                } else {
                    // Run as a task
                    if let Err(e) = session.run_task(line) {
                        println!("Task error: {}", e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Use /done or /quit to exit");
            }
            Err(ReadlineError::Eof) => {
                println!("Session complete. {} tasks.", session.task_history.len());
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    // Save history
    let _ = rl.save_history(&history_path);

    Ok(())
}

/// Truncates a string to max length, adding ... if truncated
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// A phase parsed from a plan file
struct Phase {
    title: String,
    description: String,
}

/// Parses phases from a markdown plan file
/// Looks for ## headers with "Phase" or numbered sections
fn parse_plan_phases(content: &str) -> Vec<Phase> {
    let mut phases = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_desc = String::new();

    for line in content.lines() {
        // Check for phase header: ## Phase N: Title or ## N. Title or just ## Title
        if line.starts_with("## ") {
            // Save previous phase if exists
            if let Some(title) = current_title.take() {
                phases.push(Phase {
                    title,
                    description: current_desc.trim().to_string(),
                });
                current_desc.clear();
            }

            // Parse new phase title
            let header = line.trim_start_matches("## ").trim();
            // Skip non-phase headers like "## Configuration" or "## Notes"
            let is_phase = header.to_lowercase().contains("phase")
                || header
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false);

            if is_phase {
                // Clean up title: remove "Phase N:" prefix if present
                let title = header
                    .trim_start_matches(|c: char| {
                        c.is_ascii_digit() || c == '.' || c == ':' || c == ' '
                    })
                    .trim_start_matches("Phase")
                    .trim_start_matches(|c: char| {
                        c.is_ascii_digit() || c == '.' || c == ':' || c == ' '
                    })
                    .to_string();
                current_title = Some(if title.is_empty() {
                    header.to_string()
                } else {
                    title
                });
            }
        } else if current_title.is_some() && !line.starts_with('#') {
            // Accumulate description lines
            if !line.trim().is_empty() || !current_desc.is_empty() {
                current_desc.push_str(line);
                current_desc.push('\n');
            }
        }
    }

    // Don't forget the last phase
    if let Some(title) = current_title {
        phases.push(Phase {
            title,
            description: current_desc.trim().to_string(),
        });
    }

    phases
}

/// Creates a URL-safe slug from text
fn create_slug(text: &str) -> String {
    text.chars()
        .take(30)
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("hello", 10), "hello");
        assert_eq!(truncate_string("hello world", 8), "hello...");
    }

    #[test]
    fn test_create_slug() {
        assert_eq!(create_slug("Fix the auth bug"), "fix-the-auth-bug");
        assert_eq!(create_slug("Test!@#$%"), "test");
    }

    #[test]
    fn test_parse_plan_phases() {
        let content = r#"
# My Plan

Some intro text.

## Phase 1: Setup
Set up the project structure.
Create initial files.

## Phase 2: Core Implementation
Implement the main logic.

## Notes
This is not a phase.

## Phase 3: Polish
Add finishing touches.
"#;

        let phases = parse_plan_phases(content);
        assert_eq!(phases.len(), 3);

        assert_eq!(phases[0].title, "Setup");
        assert!(phases[0].description.contains("Set up the project"));

        assert_eq!(phases[1].title, "Core Implementation");
        assert!(phases[1].description.contains("main logic"));

        assert_eq!(phases[2].title, "Polish");
    }

    #[test]
    fn test_parse_plan_numbered_phases() {
        let content = r#"
## 1. First Step
Do the first thing.

## 2. Second Step
Do the second thing.
"#;

        let phases = parse_plan_phases(content);
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].title, "First Step");
        assert_eq!(phases[1].title, "Second Step");
    }
}
