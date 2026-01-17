# Clancy: Claude Code Session Harness

A CLI wrapper that adds cross-session memory to Claude Code, inspired by the Confucius Code Agent architecture.

## Core Concept

Clancy runs a REPL that dispatches tasks to Claude Code in non-interactive mode (`claude -p`). After each task completes, Clancy extracts structured notes and injects them into the next invocation. This gives us clean session boundaries, automatic note extraction, and controlled context growth.

```
┌─────────────────────────────────────────────────────────────────┐
│                         CLANCY REPL                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  $ clancy start my-feature                                      │
│                                                                 │
│  my-feature> fix the auth bug in user_create handler            │
│       │                                                         │
│       ▼                                                         │
│  ┌─────────────────────┐                                        │
│  │ 1. Compile context  │ ← notes from ~/.config/clancy/         │
│  │ 2. Build prompt     │   prior conversation (if any)          │
│  │ 3. Run claude -p    │ → captures full output                 │
│  │ 4. Extract notes    │   via Claude API                       │
│  │ 5. Update store     │ → ~/.config/clancy/projects/my-feature/│
│  └─────────────────────┘                                        │
│       │                                                         │
│       ▼                                                         │
│  my-feature> now add tests for it                               │
│       │                                                         │
│       ▼                                                         │
│  ┌─────────────────────┐                                        │
│  │ [same cycle]        │ ← but now includes notes from task 1   │
│  └─────────────────────┘                                        │
│       │                                                         │
│       ▼                                                         │
│  my-feature> done                                               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Key Advantages

1. **Clean task boundaries** — each `claude -p` invocation is one discrete unit of work
2. **Automatic note extraction** — runs after every task, not just at session end
3. **Context refresh** — updated notes injected before each task
4. **No user discipline required** — no need to remember `/clear` or exit
5. **Conversation continuity** — Clancy can optionally include prior task summaries

## Directory Structure

### Central Storage (`~/.config/clancy/`)

```
~/.config/clancy/
├── config.toml
└── projects/
    └── my-feature/
        ├── project.toml              # metadata
        ├── tasks/
        │   ├── 001-fix-auth-bug.json     # task prompt + full output
        │   └── 002-add-tests.json
        └── notes/
            ├── architecture.md       # patterns, structure, conventions
            ├── decisions.md          # choices made and rationale
            ├── failures.md           # things that didn't work and why
            └── plan.md               # current state, next steps, todos
```

### Per-Repo Files

```
your-repo/
├── CLAUDE.md                         # checked in, static instructions + import
└── .claude/
    └── context.md                    # gitignored, generated before each task
```

**CLAUDE.md** (checked in):
```markdown
# Project Instructions

... your existing static content ...

@.claude/context.md
```

**.gitignore** addition:
```
.claude/
```

## CLI Interface

```bash
# Start a session — enters the Clancy REPL
clancy start <project-name>

# Inside the REPL:
#   <task description>     Run a task
#   /status                Show current notes summary
#   /notes [category]      View/edit notes
#   /history               Show task history this session
#   /continue              Resume with full prior conversation context
#   /compact               Summarize conversation, start fresh
#   /done or /quit         Exit the session

# Outside the REPL — management commands
clancy list                           # List all projects
clancy status [project-name]          # Show project status and notes
clancy notes <project> [category]     # View/edit notes directly
clancy link <child> <parent>          # Link for note inheritance
clancy archive <project-name>         # Archive completed project
```

## Task Execution Flow

When you enter a task in the REPL:

```
┌──────────────────────────────────────────────────────────────────┐
│ 1. COMPILE CONTEXT                                               │
│    - Load notes from project store                               │
│    - Include inherited notes from parent projects                │
│    - Add conversation history (configurable depth)               │
│    - Write to .claude/context.md                                 │
├──────────────────────────────────────────────────────────────────┤
│ 2. BUILD PROMPT                                                  │
│    - User's task description                                     │
│    - Optional: summary of prior tasks this session               │
├──────────────────────────────────────────────────────────────────┤
│ 3. EXECUTE                                                       │
│    claude -p "<prompt>" --output-format stream-json              │
│    - Stream output to terminal (user sees progress)              │
│    - Capture full output to task log                             │
├──────────────────────────────────────────────────────────────────┤
│ 4. EXTRACT NOTES                                                 │
│    - Send transcript to Claude API                               │
│    - Parse structured note updates                               │
│    - Merge into project notes                                    │
├──────────────────────────────────────────────────────────────────┤
│ 5. UPDATE STATE                                                  │
│    - Save task log                                               │
│    - Update project metadata                                     │
│    - Ready for next task                                         │
└──────────────────────────────────────────────────────────────────┘
```

## Conversation Continuity

A key design decision: how much prior context to include between tasks?

**Options (configurable):**

1. **Fresh each task** — only notes, no conversation history
   - Pros: Maximum context budget for current task
   - Cons: Loses "we just discussed X" continuity

2. **Rolling summary** — compressed summary of prior tasks
   - Pros: Maintains continuity without bloat
   - Cons: May lose details

3. **Full history** (with `/continue`) — include complete prior conversation
   - Pros: Perfect continuity for multi-step work
   - Cons: Context grows, may hit limits

**Default behavior:**
- Include a one-paragraph summary of each prior task this session
- User can `/continue` for full context when needed
- User can `/compact` to compress and start fresh

## Context Injection

Before each task, compile notes into `.claude/context.md`:

```markdown
<!-- CLANCY CONTEXT — AUTO-GENERATED -->
<!-- Project: my-feature | Task: 3 -->

## Session Context

This is task 3 of an ongoing session. Prior tasks:
1. Fixed auth bug in user_create handler — modified src/handlers/user.rs
2. Added integration test — created tests/api/user_create_test.rs

## Architectural Context

[contents of architecture.md, including inherited from parent]

## Key Decisions

[contents of decisions.md, most recent first]

## Known Pitfalls

[contents of failures.md — critical for avoiding repeated mistakes]

## Current Plan

[contents of plan.md]

---
When you complete work or encounter a problem, state it clearly for continuity.
```

## Configuration (`~/.config/clancy/config.toml`)

```toml
[claude]
api_key_env = "ANTHROPIC_API_KEY"    # env var containing API key
model = "claude-sonnet-4-20250514"   # model for note extraction

[extraction]
max_transcript_tokens = 100000       # truncate very long task outputs
include_tool_outputs = true          # include file contents, command outputs

[context]
max_context_tokens = 12000           # cap on compiled context size
include_parent_notes = true          # inherit from linked parent projects
conversation_mode = "summary"        # fresh | summary | full

[repl]
editor = "nvim"                      # for /notes command
prompt_style = "project"             # shows "my-feature> "
```

## Project Metadata (`project.toml`)

```toml
name = "my-feature"
created = "2025-01-15T09:00:00Z"
last_task = "2025-01-15T14:30:00Z"
parent = "auth-layer"                 # optional, for note inheritance
branch = "feature/my-feature"         # informational
status = "active"                     # active | archived

[stats]
total_sessions = 3
total_tasks = 12
```

## Note Extraction

After each task, send the output to Claude for note extraction:

```markdown
You are extracting structured notes from a coding task transcript.
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
{{EXISTING_ARCHITECTURE}}
</architecture>

<decisions>
{{EXISTING_DECISIONS}}
</decisions>

<failures>
{{EXISTING_FAILURES}}
</failures>

<plan>
{{EXISTING_PLAN}}
</plan>

---

## Task Transcript

<transcript>
{{TRANSCRIPT}}
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
[full replacement content]
```

## Implementation Phases

### Phase 1: Basic REPL + Context Injection (2-3 hours)
- Rust CLI with basic REPL loop
- Project/notes directory management
- Context compilation and injection to `.claude/context.md`
- Shell out to `claude -p` and stream output
- Manual note editing with `/notes`

This gets you the core loop working without API integration.

### Phase 2: Transcript Capture (1-2 hours)
- Capture `--output-format stream-json` output
- Parse into structured task logs
- Store in project tasks/ directory

### Phase 3: Automated Note Extraction (2-3 hours)
- Claude API client integration
- Note extraction after each task
- Automatic merging into note files
- Full automation complete

### Phase 4: Polish (as needed)
- Conversation continuity modes (`/continue`, `/compact`)
- Project linking for branch stacks
- Token budget management / context summarization
- Better error handling and recovery

## Example Session

```
$ clancy start my-feature
Loading project: my-feature (3 prior sessions, 8 tasks)
Injecting context from notes...

my-feature> implement the password reset endpoint following our existing patterns

[claude -p runs, output streams to terminal]
[... claude does the work ...]

Extracting notes...
  ✓ architecture: added 2 items
  ✓ decisions: added 1 item  
  ✓ failures: no updates
  ✓ plan: updated

my-feature> /status

## Current Plan
Password reset endpoint implemented. Next:
- [ ] Add rate limiting
- [ ] Write integration tests
- [ ] Update API docs

## Recent Decisions
- [2025-01-15] Using existing email service rather than new provider

my-feature> add rate limiting to the new endpoint

[claude -p runs with updated context...]

my-feature> /done
Session complete. 2 tasks, notes updated.
```

## Open Questions

1. **Streaming output**: Need to verify `claude -p --output-format stream-json` streams to stdout while also giving us the full transcript. May need to tee or buffer.

2. **Error recovery**: If `claude -p` fails mid-task or note extraction fails, how do we recover? Probably save raw output first, extract notes as a separate retry-able step.

3. **Task granularity**: Is one `claude -p` invocation always one "task", or should user be able to group multiple invocations? Start simple (1:1), evolve if needed.

4. **Interactive fallback**: Some tasks benefit from mid-task feedback. Consider a `/interactive` command that drops into regular `claude` for one task, then returns to the REPL.
