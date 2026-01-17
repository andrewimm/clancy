use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config;

/// Project metadata stored in project.toml
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub name: String,
    pub created: DateTime<Utc>,
    pub last_task: Option<DateTime<Utc>>,
    /// Optional parent project for note inheritance
    pub parent: Option<String>,
    /// Git branch (informational)
    pub branch: Option<String>,
    /// Project status: active | archived
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub stats: ProjectStats,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProjectStats {
    pub total_sessions: u32,
    pub total_tasks: u32,
}

fn default_status() -> String {
    "active".to_string()
}

/// Note categories
pub const NOTE_CATEGORIES: &[&str] = &["architecture", "decisions", "failures", "plan"];

/// Represents a project with its directory and metadata
pub struct Project {
    pub metadata: ProjectMetadata,
    pub path: PathBuf,
}

impl Project {
    /// Opens an existing project or creates a new one
    pub fn open_or_create(name: &str) -> Result<Self> {
        config::ensure_config_dir()?;
        let project_path = config::projects_dir()?.join(name);

        if project_path.exists() {
            Self::open(name)
        } else {
            Self::create(name)
        }
    }

    /// Opens an existing project
    pub fn open(name: &str) -> Result<Self> {
        let project_path = config::projects_dir()?.join(name);
        if !project_path.exists() {
            bail!("Project '{}' not found", name);
        }

        let metadata_path = project_path.join("project.toml");
        let metadata = if metadata_path.exists() {
            let content = std::fs::read_to_string(&metadata_path)
                .with_context(|| format!("Failed to read project metadata: {:?}", metadata_path))?;
            toml::from_str(&content).with_context(|| "Failed to parse project metadata")?
        } else {
            // Metadata file missing, create default
            ProjectMetadata {
                name: name.to_string(),
                created: Utc::now(),
                last_task: None,
                parent: None,
                branch: None,
                status: "active".to_string(),
                stats: ProjectStats::default(),
            }
        };

        Ok(Self {
            metadata,
            path: project_path,
        })
    }

    /// Creates a new project
    pub fn create(name: &str) -> Result<Self> {
        config::ensure_config_dir()?;
        let project_path = config::projects_dir()?.join(name);

        if project_path.exists() {
            bail!("Project '{}' already exists", name);
        }

        // Create directory structure
        std::fs::create_dir_all(&project_path)
            .with_context(|| format!("Failed to create project directory: {:?}", project_path))?;
        std::fs::create_dir_all(project_path.join("tasks"))
            .context("Failed to create tasks directory")?;
        std::fs::create_dir_all(project_path.join("notes"))
            .context("Failed to create notes directory")?;

        // Create metadata
        let metadata = ProjectMetadata {
            name: name.to_string(),
            created: Utc::now(),
            last_task: None,
            parent: None,
            branch: None,
            status: "active".to_string(),
            stats: ProjectStats::default(),
        };

        let project = Self {
            metadata,
            path: project_path,
        };

        // Initialize empty note files
        for category in NOTE_CATEGORIES {
            let note_path = project.notes_path(category);
            if !note_path.exists() {
                std::fs::write(&note_path, "")
                    .with_context(|| format!("Failed to create note file: {:?}", note_path))?;
            }
        }

        project.save_metadata()?;

        Ok(project)
    }

    /// Saves the project metadata
    pub fn save_metadata(&self) -> Result<()> {
        let metadata_path = self.path.join("project.toml");
        let content = toml::to_string_pretty(&self.metadata)
            .context("Failed to serialize project metadata")?;
        std::fs::write(&metadata_path, content)
            .with_context(|| format!("Failed to write project metadata: {:?}", metadata_path))?;
        Ok(())
    }

    /// Returns the path to a note file
    pub fn notes_path(&self, category: &str) -> PathBuf {
        self.path.join("notes").join(format!("{}.md", category))
    }

    /// Returns the path to the tasks directory
    pub fn tasks_path(&self) -> PathBuf {
        self.path.join("tasks")
    }

    /// Reads notes for a category
    pub fn read_notes(&self, category: &str) -> Result<String> {
        let path = self.notes_path(category);
        if path.exists() {
            std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read notes: {:?}", path))
        } else {
            Ok(String::new())
        }
    }

    /// Writes notes for a category
    pub fn write_notes(&self, category: &str, content: &str) -> Result<()> {
        let path = self.notes_path(category);
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write notes: {:?}", path))?;
        Ok(())
    }

    /// Appends to notes for a category (except plan which is replaced)
    pub fn append_notes(&self, category: &str, content: &str) -> Result<()> {
        if category == "plan" {
            // Plan is replaced, not appended
            self.write_notes(category, content)
        } else {
            let existing = self.read_notes(category)?;
            let new_content = if existing.is_empty() {
                content.to_string()
            } else {
                format!("{}\n{}", existing.trim_end(), content)
            };
            self.write_notes(category, &new_content)
        }
    }

    /// Updates the last_task timestamp and increments task count
    pub fn record_task(&mut self) -> Result<()> {
        self.metadata.last_task = Some(Utc::now());
        self.metadata.stats.total_tasks += 1;
        self.save_metadata()
    }

    /// Increments session count
    pub fn record_session_start(&mut self) -> Result<()> {
        self.metadata.stats.total_sessions += 1;
        self.save_metadata()
    }

    /// Returns the next task number
    pub fn next_task_number(&self) -> Result<u32> {
        let tasks_dir = self.tasks_path();
        if !tasks_dir.exists() {
            return Ok(1);
        }

        let mut max_num = 0;
        for entry in std::fs::read_dir(&tasks_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Task files are named like 001-description.json
            if let Some(num_str) = name_str.split('-').next() {
                if let Ok(num) = num_str.parse::<u32>() {
                    max_num = max_num.max(num);
                }
            }
        }

        Ok(max_num + 1)
    }
}

/// Lists all projects
pub fn list_projects() -> Result<()> {
    config::ensure_config_dir()?;
    let projects_dir = config::projects_dir()?;

    if !projects_dir.exists() {
        println!("No projects found.");
        return Ok(());
    }

    let mut projects: Vec<_> = std::fs::read_dir(&projects_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    // Sort by name
    projects.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    println!("Projects:\n");
    for entry in projects {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Try to load metadata for status info
        if let Ok(project) = Project::open(&name_str) {
            let status_marker = if project.metadata.status == "archived" {
                " (archived)"
            } else {
                ""
            };
            let task_info = format!(
                "{} sessions, {} tasks",
                project.metadata.stats.total_sessions, project.metadata.stats.total_tasks
            );
            println!("  {}{} - {}", name_str, status_marker, task_info);
        } else {
            println!("  {}", name_str);
        }
    }

    Ok(())
}

/// Shows project status
pub fn show_status(project_name: Option<&str>) -> Result<()> {
    let name = project_name.ok_or_else(|| anyhow::anyhow!("Project name required"))?;
    let project = Project::open(name)?;

    println!("Project: {}", project.metadata.name);
    println!("Status: {}", project.metadata.status);
    println!(
        "Created: {}",
        project.metadata.created.format("%Y-%m-%d %H:%M")
    );
    if let Some(last) = project.metadata.last_task {
        println!("Last task: {}", last.format("%Y-%m-%d %H:%M"));
    }
    println!(
        "Stats: {} sessions, {} tasks",
        project.metadata.stats.total_sessions, project.metadata.stats.total_tasks
    );

    // Show plan if it exists
    let plan = project.read_notes("plan")?;
    if !plan.trim().is_empty() {
        println!("\n## Current Plan\n");
        println!("{}", plan);
    }

    // Show recent decisions
    let decisions = project.read_notes("decisions")?;
    if !decisions.trim().is_empty() {
        let lines: Vec<&str> = decisions.lines().collect();
        let recent: Vec<&str> = lines.iter().rev().take(5).copied().collect();
        if !recent.is_empty() {
            println!("\n## Recent Decisions\n");
            for line in recent.iter().rev() {
                println!("{}", line);
            }
        }
    }

    Ok(())
}

/// Opens editor for notes
pub fn edit_notes(project_name: &str, category: Option<&str>) -> Result<()> {
    let project = Project::open(project_name)?;
    let config = config::load_config()?;

    let path = if let Some(cat) = category {
        if !NOTE_CATEGORIES.contains(&cat) {
            bail!(
                "Invalid category '{}'. Valid: {}",
                cat,
                NOTE_CATEGORIES.join(", ")
            );
        }
        project.notes_path(cat)
    } else {
        // Open notes directory
        project.path.join("notes")
    };

    let editor = &config.repl.editor;
    let status = std::process::Command::new(editor)
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to open editor: {}", editor))?;

    if !status.success() {
        bail!("Editor exited with error");
    }

    Ok(())
}

/// Archives a project
pub fn archive_project(project_name: &str) -> Result<()> {
    let mut project = Project::open(project_name)?;
    project.metadata.status = "archived".to_string();
    project.save_metadata()?;
    println!("Project '{}' archived.", project_name);
    Ok(())
}

/// Links a child project to a parent for note inheritance
pub fn link_projects(child_name: &str, parent_name: &str) -> Result<()> {
    // Verify parent exists
    let _parent = Project::open(parent_name)
        .with_context(|| format!("Parent project '{}' not found", parent_name))?;

    // Update child's parent reference
    let mut child = Project::open(child_name)
        .with_context(|| format!("Child project '{}' not found", child_name))?;

    // Check for circular references
    if child_name == parent_name {
        bail!("Cannot link a project to itself");
    }

    // Check if parent has this child as an ancestor (would create cycle)
    let mut current = Some(parent_name.to_string());
    while let Some(ref name) = current {
        if name == child_name {
            bail!(
                "Cannot link: would create circular reference ({} -> ... -> {})",
                child_name,
                parent_name
            );
        }
        if let Ok(p) = Project::open(name) {
            current = p.metadata.parent;
        } else {
            break;
        }
    }

    child.metadata.parent = Some(parent_name.to_string());
    child.save_metadata()?;

    println!(
        "Linked '{}' -> '{}'. Child will inherit parent's architecture notes.",
        child_name, parent_name
    );
    Ok(())
}

/// Unlinks a project from its parent
pub fn unlink_project(project_name: &str) -> Result<()> {
    let mut project = Project::open(project_name)?;

    if project.metadata.parent.is_none() {
        println!("Project '{}' has no parent link.", project_name);
        return Ok(());
    }

    let parent_name = project.metadata.parent.take();
    project.save_metadata()?;

    println!(
        "Unlinked '{}' from '{}'.",
        project_name,
        parent_name.unwrap_or_default()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_categories_exist() {
        assert!(NOTE_CATEGORIES.contains(&"architecture"));
        assert!(NOTE_CATEGORIES.contains(&"decisions"));
        assert!(NOTE_CATEGORIES.contains(&"failures"));
        assert!(NOTE_CATEGORIES.contains(&"plan"));
    }

    #[test]
    fn test_project_metadata_serialization() {
        let metadata = ProjectMetadata {
            name: "test".to_string(),
            created: Utc::now(),
            last_task: None,
            parent: None,
            branch: Some("main".to_string()),
            status: "active".to_string(),
            stats: ProjectStats::default(),
        };

        let serialized = toml::to_string_pretty(&metadata).unwrap();
        let deserialized: ProjectMetadata = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.name, "test");
    }
}
