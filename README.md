# Clancy

A CLI wrapper that adds cross-session memory to Claude Code. Clancy runs a REPL that dispatches tasks to `claude -p`, automatically extracts structured notes after each task, and injects them into the next invocation.

## Features

- **Automatic note extraction** — Claude analyzes each task and updates project notes
- **Context injection** — Notes injected before each task via `.claude/context.md`
- **Conversation continuity** — Choose between fresh, summary, or full conversation modes
- **Project linking** — Inherit notes from parent projects (useful for branch stacks)
- **Token budgeting** — Automatic context truncation when approaching limits

## Installation

```bash
# Clone and build
git clone https://github.com/yourname/clancy
cd clancy
cargo build --release

# Add to PATH
cp target/release/clancy ~/.local/bin/
```

Requires:
- [Claude Code CLI](https://claude.ai/code) installed and authenticated
- `ANTHROPIC_API_KEY` environment variable (for note extraction)

## Quick Start

### 1. Set up your repo

```bash
cd your-project

# Add to .gitignore
echo ".claude/" >> .gitignore

# Add import to CLAUDE.md (create if needed)
echo "@.claude/context.md" >> CLAUDE.md
```

### 2. Start a session

```bash
clancy start my-feature
```

### 3. Work

```
my-feature> implement user authentication with JWT
[Task 1] Injecting context (~450 tokens)...

(Claude does the work...)

[Task 1 complete in 45.2s ($0.0234)]
Extracting notes... updated: architecture, decisions, plan

my-feature> add tests for the auth middleware
[Task 2] Injecting context (~1,200 tokens)...

...

my-feature> /done
Session complete. 2 tasks, notes updated.
```

## Usage Examples

### Greenfield Project

```bash
mkdir my-app && cd my-app
git init
echo ".claude/" >> .gitignore
echo "@.claude/context.md" > CLAUDE.md

clancy start my-app
```
```
my-app> scaffold a REST API with Express and TypeScript
my-app> add a /users endpoint with CRUD operations
my-app> add input validation with zod
my-app> /status   # see current plan and decisions
my-app> /done
```

### Brownfield Codebase

```bash
cd ~/code/existing-project
echo ".claude/" >> .gitignore
# Add @.claude/context.md to your existing CLAUDE.md

clancy start existing-project
```
```
existing-project> explore the codebase and summarize the architecture
existing-project> fix the race condition in src/worker.ts
existing-project> /notes architecture   # review/edit extracted notes
existing-project> /done
```

### Working with a PLAN.md

If you have a PRD or implementation plan, use `/auto` to run through all phases automatically:

```
my-project> /auto PLAN.md

Found 4 phases in PLAN.md:

  1. Setup
  2. Core Implementation
  3. Testing
  4. Polish

Press Enter to start, or Ctrl+C to cancel...

============================================================
Phase 1/4: Setup
============================================================

[Task 1] Injecting context (~450 tokens)...

(Claude implements Phase 1...)

[Task 1 complete in 45.2s ($0.0234)]
Extracting notes... updated: architecture, plan

Phase 1 complete. Press Enter for next phase, or 'q' to stop...
```

**PLAN.md format:**

```markdown
# My Implementation Plan

## Phase 1: Setup
Set up the project structure with Cargo.toml and basic CLI.

## Phase 2: Core Implementation
Implement the main business logic.

## Phase 3: Testing
Add unit and integration tests.
```

Clancy parses `## Phase N: Title` or `## N. Title` headers and uses the following paragraph as the task prompt.

You can also reference the plan in CLAUDE.md for manual work:

```markdown
# CLAUDE.md

@PLAN.md
@.claude/context.md
```

Then work through phases manually:

```
my-project> implement Phase 1 from PLAN.md
```

### Long Session with Context Management

```
my-feature> implement the payment flow
my-feature> add Stripe integration
my-feature> /continue              # switch to full conversation mode
my-feature> now add error handling  # has full context from previous tasks
my-feature> /compact               # summarize and free up context
my-feature> add webhook handlers   # starts fresh but keeps summary
```

### Branch Stack with Linked Projects

```bash
# Parent project with shared architecture knowledge
clancy start auth-layer
# ... do work, /done

# Child project inherits parent's notes
clancy start feature-x
clancy link feature-x auth-layer

clancy start feature-x
# Context now includes auth-layer's architecture notes
```

## CLI Commands

```bash
clancy start <project>           # Start REPL session
clancy list                      # List all projects
clancy status <project>          # Show project status and notes
clancy notes <project> [cat]     # Edit notes (architecture|decisions|failures|plan)
clancy archive <project>         # Archive a project
clancy link <child> <parent>     # Link for note inheritance
clancy unlink <project>          # Remove parent link
```

## REPL Commands

| Command | Description |
|---------|-------------|
| `<task>` | Run a task via Claude |
| `/status` | Show current plan and recent decisions |
| `/notes [category]` | Edit notes in your editor |
| `/history` | Show tasks this session |
| `/auto [file]` | Run all phases from PLAN.md (or specified file) |
| `/continue` | Full conversation mode (complete prior context) |
| `/compact` | Summarize history, start fresh |
| `/fresh` | Only notes, no session history |
| `/summary` | Default mode (task summaries) |
| `/done`, `/quit` | Exit session |
| `/help` | Show help |

## Configuration

Config file: `~/.config/clancy/config.toml`

```toml
[claude]
api_key_env = "ANTHROPIC_API_KEY"      # env var for API key
model = "claude-sonnet-4-20250514"     # model for note extraction

[context]
max_context_tokens = 12000             # truncate context above this
conversation_mode = "summary"          # fresh | summary | full
include_parent_notes = true            # inherit from linked projects

[repl]
editor = "vim"                         # for /notes command
```

### Using Vercel AI Gateway

To route API calls through [Vercel AI Gateway](https://vercel.com/docs/ai-gateway), set the `base_url` in your config:

```toml
[claude]
api_key_env = "ANTHROPIC_API_KEY"      # or your Vercel-provided key
base_url = "https://ai-gateway.vercel.sh"
```

You can also store your API key in a `.env` file in your project directory:

```bash
ANTHROPIC_API_KEY=your-api-key-here
```

Clancy loads `.env` automatically at startup.

## How It Works

```
┌─────────────────────────────────────────────────────────────┐
│  you> fix the auth bug                                      │
├─────────────────────────────────────────────────────────────┤
│  1. Compile notes → .claude/context.md                      │
│  2. Run: claude -p "fix the auth bug" --output-format json  │
│  3. Stream output to terminal                               │
│  4. Parse transcript, save to tasks/001-fix-the-auth.json   │
│  5. Send transcript to Claude API for note extraction       │
│  6. Merge extracted notes into project files                │
├─────────────────────────────────────────────────────────────┤
│  you> (next task has updated context)                       │
└─────────────────────────────────────────────────────────────┘
```

### Note Categories

| Category | Purpose | Update Style |
|----------|---------|--------------|
| `architecture.md` | Patterns, conventions, structure | Append |
| `decisions.md` | Choices made with rationale | Append |
| `failures.md` | What didn't work and why | Append |
| `plan.md` | Current state, next steps | Replace |

## Project Data

```
~/.config/clancy/
├── config.toml
└── projects/
    └── my-feature/
        ├── project.toml           # metadata
        ├── notes/
        │   ├── architecture.md
        │   ├── decisions.md
        │   ├── failures.md
        │   └── plan.md
        └── tasks/
            ├── 001-fix-auth-bug.json
            └── 002-add-tests.json
```

## License

MIT
