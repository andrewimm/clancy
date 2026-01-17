use anyhow::{Context, Result};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config;
use crate::project::{Project, NOTE_CATEGORIES};

/// Task summary for conversation continuity
struct TaskSummary {
    number: u32,
    prompt: String,
    summary: String,
}

/// REPL session state
struct Session {
    project: Project,
    task_history: Vec<TaskSummary>,
    working_dir: PathBuf,
}

impl Session {
    fn new(project: Project) -> Result<Self> {
        let working_dir = std::env::current_dir()?;
        Ok(Self {
            project,
            task_history: Vec::new(),
            working_dir,
        })
    }

    /// Compiles all notes into .claude/context.md
    fn compile_context(&self) -> Result<()> {
        let claude_dir = self.working_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir)?;

        let context_path = claude_dir.join("context.md");
        let mut content = String::new();

        // Header
        content.push_str("<!-- CLANCY CONTEXT — AUTO-GENERATED -->\n");
        content.push_str(&format!(
            "<!-- Project: {} | Task: {} -->\n\n",
            self.project.metadata.name,
            self.task_history.len() + 1
        ));

        // Session context with prior task summaries
        if !self.task_history.is_empty() {
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
            content.push_str("\n");
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

        std::fs::write(&context_path, &content)
            .with_context(|| format!("Failed to write context file: {:?}", context_path))?;

        Ok(())
    }

    /// Runs a task via claude -p
    fn run_task(&mut self, prompt: &str) -> Result<()> {
        // Compile context before task
        self.compile_context()?;

        let task_num = self.task_history.len() as u32 + 1;
        println!("\n[Task {}] Running...\n", task_num);

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

        // Record task (basic summary for now - will be enhanced in Phase 3)
        let summary = self.generate_basic_summary(prompt);
        self.task_history.push(TaskSummary {
            number: task_num,
            prompt: truncate_string(prompt, 60),
            summary,
        });

        // Update project stats
        self.project.record_task()?;

        // Save task log
        self.save_task_log(task_num, prompt, &captured_output)?;

        println!("[Task {} complete]\n", task_num);
        Ok(())
    }

    /// Generates a basic summary (placeholder for Phase 3 extraction)
    fn generate_basic_summary(&self, prompt: &str) -> String {
        // For Phase 1, just use a truncated version of the prompt
        // Phase 3 will use Claude API for proper extraction
        truncate_string(prompt, 80)
    }

    /// Saves the task log to disk
    fn save_task_log(&self, task_num: u32, prompt: &str, output: &str) -> Result<()> {
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
            "output": output,
        });

        let content = serde_json::to_string_pretty(&log)?;
        std::fs::write(&path, content)?;

        Ok(())
    }

    /// Handles REPL commands (those starting with /)
    fn handle_command(&mut self, cmd: &str) -> Result<bool> {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let command = parts.first().map(|s| *s).unwrap_or("");

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
        println!(
            r#"
## Clancy REPL Commands

  <task description>   Run a task via Claude
  /status              Show current notes summary
  /notes [category]    Edit notes (architecture|decisions|failures|plan)
  /history             Show task history this session
  /done or /quit       Exit the session
  /help                Show this help
"#
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
    println!("Injecting context from notes...\n");

    let mut session = Session::new(project)?;
    session.compile_context()?;

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
}
