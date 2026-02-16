# mc — project management for developers and AI agents

Manage tasks, meetings, research, contacts, and sprints with plain Markdown files. No database, no server, no account — just files in your repo that you can `git diff`, review in PRs, and edit with any tool. Works from your terminal and from AI editors via MCP.

[![Crates.io](https://img.shields.io/crates/v/mc.svg)](https://crates.io/crates/mc)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/RaaSaaR-org/mission-control/actions/workflows/ci.yml/badge.svg)](https://github.com/RaaSaaR-org/mission-control/actions)

## Quickstart

```bash
cargo install mc
cd my-project && mc init --embedded
mc -y new task "Ship the feature" --priority 2
mc task board
```

That's it. You now have a `.mc/` folder with task tracking in your project.

## Why mc?

- **Plain Markdown + YAML frontmatter** — `git diff` your project management
- **No database, no server, no account** — just files in your repo
- **Built for AI agents** — full MCP server, one command to connect
- **Embedded mode** — `.mc/` folder lives alongside your code
- **Kanban board, sprint planning, meeting notes, research tracking, contact management** — all from the terminal

## Two Ways to Use It

### Embedded Mode (recommended for most projects)

Add `mc` to an existing project. Creates a `.mc/` folder for tasks, meetings, research, and sprints (no customers, projects, or contacts).

```bash
cd my-project
mc init --embedded
```

```
my-project/
├── .mc/
│   ├── config.yml
│   ├── tasks/
│   ├── meetings/
│   ├── research/
│   └── sprints/
└── (your code)
```

### Standalone Mode (for portfolios and CRM)

Create a dedicated repository for managing customers, projects, and all related work.

```bash
mc init --name "My Company"
```

```
my-company/
├── config/config.yml
├── customers/
├── projects/
├── meetings/
├── research/
├── tasks/
└── sprints/
```

Standalone mode adds **customers**, **contacts**, and **projects** — useful for agencies, freelancers, or anyone managing multiple clients.

## What You Can Track

| Entity | Description | Example |
|--------|-------------|---------|
| **Tasks** | Work items with priority, status, dependencies | `mc new task "Fix auth bug" --priority 1` |
| **Sprints** | Time-boxed iterations | `mc new sprint "2026-W06" --goal "Auth module"` |
| **Meetings** | Notes with date, attendees, PDF export | `mc new meeting "Sprint Review" --date 2026-02-10` |
| **Research** | Multi-agent research topics, PDF export | `mc new research "LLM Benchmarks" --agents claude,gemini` |
| **Customers** | Client profiles (standalone only) | `mc new customer "Acme Corp"` |
| **Contacts** | Per-customer contacts (standalone only) | `mc new contact "Alice Smith" --customer CUST-001` |
| **Projects** | Project containers (standalone only) | `mc new project "Robot Arm" --customers CUST-001` |

## Task Management

Tasks are first-class citizens with a full kanban workflow.

### Kanban Board

```bash
mc task board
```

```
┌─────────────────────────────────────────────────────────────────────┐
│ backlog          todo             in-progress      done             │
├─────────────────────────────────────────────────────────────────────┤
│ TASK-003 [P3]    TASK-002 [P2]    TASK-001 [P1]                     │
│ Write tests      Update docs      Fix auth bug                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Move Tasks

```bash
mc task move TASK-001 in-progress
mc task move TASK-001 done
```

### Next Task

Get the highest-priority unblocked task:

```bash
mc task next
```

### Filtering

```bash
mc list tasks --status in-progress --priority 1
mc list tasks --sprint 2026-W06 --owner alice
mc task board --project PROJ-001
```

### Dependencies

Tasks can depend on other tasks:

```bash
mc new task "Deploy to prod" --depends-on TASK-001,TASK-002
```

Blocked tasks are hidden from `mc task next` until their dependencies are done.

## AI Agent Integration (MCP)

Connect your AI editor to `mc` with one command:

**Claude Code:**
```bash
claude mcp add mc -- mc mcp
```

**Cursor, Windsurf, Claude Desktop** (add to your MCP config):
```json
{
  "mcpServers": {
    "mc": {
      "command": "mc",
      "args": ["mcp"]
    }
  }
}
```

**VS Code** (`.vscode/mcp.json`):
```json
{
  "servers": {
    "mc": {
      "type": "stdio",
      "command": "mc",
      "args": ["mcp"]
    }
  }
}
```

Now your AI assistant can create tasks, move them through the board, query status, create meetings, and more.

### Available MCP Tools

| Tool | Description |
|------|-------------|
| `list_entities` | List entities by kind with optional filters |
| `list_tasks` | List tasks with rich filtering (project, sprint, priority, owner) |
| `get_entity` | Get details about any entity |
| `create_task` | Create a new task |
| `create_sprint` | Create a new sprint |
| `create_meeting` | Create a meeting |
| `create_contact` | Create a contact for a customer |
| `create_research` | Create a research topic |
| `move_task` | Move a task to a new status |
| `print_meeting` | Export meeting to PDF |
| `print_research` | Export research to PDF |
| `validate_repo` | Check repo structure |
| `get_status` | Dashboard with counts and recent activity |

## Web Dashboard

Browse all your data in a local web UI:

```bash
mc serve
# Open http://localhost:5000
```

## Installation

### From crates.io

```bash
cargo install mc
```

### Prebuilt Binaries

Download from [GitHub Releases](https://github.com/RaaSaaR-org/mission-control/releases):

| Platform | Archive |
|----------|---------|
| macOS (Apple Silicon) | `mc-macos-arm64.tar.gz` |
| macOS (Intel) | `mc-macos-amd64.tar.gz` |
| Linux (x86_64) | `mc-linux-amd64.tar.gz` |
| Linux (arm64) | `mc-linux-arm64.tar.gz` |

```bash
tar xzf mc-<platform>.tar.gz
sudo mv mc /usr/local/bin/
```

### Build from Source

```bash
git clone https://github.com/RaaSaaR-org/mission-control.git
cd mission-control
cargo build --release
# Binary is at target/release/mc
```

## Configuration

Config lives at `.mc/config.yml` (embedded) or `config/config.yml` (standalone). It controls:

- Directory paths for each entity type
- ID prefixes (`TASK`, `MTG`, `RES`, `SPR`, `CONT`, etc.)
- Allowed status values per entity type

The defaults work out of the box. Edit the config when you want to customize status workflows or add new prefixes.

## CLI Reference

| Command | Description |
|---------|-------------|
| `mc init` | Initialize a new repo (use `--embedded` for existing projects) |
| `mc new <type> "name"` | Create a new entity |
| `mc list <type>` | List entities with optional filters |
| `mc show <ID>` | Display entity details |
| `mc task board` | Kanban board view |
| `mc task move <ID> <status>` | Change task status |
| `mc task next` | Show next actionable task |
| `mc validate` | Check repo structure and frontmatter |
| `mc status` | Dashboard with counts and recent activity |
| `mc print meeting <ID>` | Export meeting to PDF |
| `mc print research <ID>` | Export research to PDF |
| `mc serve` | Start web dashboard |
| `mc mcp` | Start MCP server |

Use `mc <command> --help` for detailed options.

## Contributing

CI runs on every push and PR:
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- `cargo build --release`

See [CLAUDE.md](CLAUDE.md) for architecture details and build commands.

## License

MIT
