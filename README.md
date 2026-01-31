# mc -- MissionControl CLI

A command-line tool for managing git-based knowledge repositories. Scaffolds new entities, validates repo structure, builds JSON indexes, exports archives, and serves a local dashboard.

## Installation

### Prebuilt binaries

Download the latest release for your platform from the [GitHub Releases](https://github.com/emai-immo/mc/releases) page:

| Platform | Archive |
|----------|---------|
| macOS (Apple Silicon) | `mc-macos-arm64.tar.gz` |
| macOS (Intel) | `mc-macos-amd64.tar.gz` |
| Linux (x86_64) | `mc-linux-amd64.tar.gz` |
| Linux (arm64) | `mc-linux-arm64.tar.gz` |

Extract and place `mc` somewhere on your `PATH`:

```bash
tar xzf mc-<platform>.tar.gz
sudo mv mc /usr/local/bin/
```

### Build from source

```bash
cargo install --git https://github.com/emai-immo/mc
```

Or clone and build locally:

```bash
git clone https://github.com/emai-immo/mc.git
cd mc
cargo build --release
# Binary is at target/release/mc
```

## Usage

```
mc [--root <PATH>] [-y|--yes] <COMMAND>
```

**Global flags:**

| Flag | Description |
|------|-------------|
| `--root <PATH>` | Path to repo root. Auto-detected by walking up the directory tree looking for `config/config.yml` if omitted. |
| `-y, --yes` | Skip interactive prompts and use defaults. |

## Commands

### `mc new` -- scaffold a new entity

Creates directories, files, and YAML frontmatter from templates. The next sequential ID is assigned automatically.

#### `mc new customer "<name>"`

| Option | Description |
|--------|-------------|
| `--owner <OWNER>` | Owner name |
| `--status <STATUS>` | Initial status (default: prompted interactively) |
| `--tags <TAGS>` | Comma-separated tags |

Creates `customers/CUST-NNN-slug/` with:
- `_index.md` -- customer overview with frontmatter
- `contacts.md` -- contact list template
- Subdirectories: `contracts/`, `meetings/`, `projects/`, `assets/`

#### `mc new project "<name>"`

| Option | Description |
|--------|-------------|
| `--owner <OWNER>` | Project owner |
| `--status <STATUS>` | Initial status |
| `--customers <IDS>` | Comma-separated customer IDs to link |
| `--tags <TAGS>` | Comma-separated tags |

Creates `projects/PROJ-NNN-slug/` with:
- `overview.md` -- project overview with frontmatter
- `roadmap.md` -- milestone tracking template
- `backlog.md` -- task tracking template
- Subdirectories: `specs/`, `releases/`, `infra/`

#### `mc new meeting "<title>"`

| Option | Description |
|--------|-------------|
| `--date <DATE>` | Date in `YYYY-MM-DD` format (defaults to today) |
| `--time <TIME>` | Time in `HH:MM` format (defaults to `10:00`) |
| `--duration <DUR>` | Duration string (e.g. `30m`, `1h`) |
| `--status <STATUS>` | Initial status |
| `--tags <TAGS>` | Comma-separated tags |
| `--customers <IDS>` | Comma-separated customer IDs to link |
| `--projects <IDS>` | Comma-separated project IDs to link |

Creates `meetings/YYYY-MM-DD-slug.md` with frontmatter and agenda/notes/action-items sections.

#### `mc new research "<title>"`

| Option | Description |
|--------|-------------|
| `--owner <OWNER>` | Research owner |
| `--agents <AGENTS>` | Comma-separated agent names (defaults to `claude,gemini,chatgpt,perplexity`) |
| `--tags <TAGS>` | Comma-separated tags |

Creates `research/RES-NNN-slug/` with:
- `_index.md` -- research overview with frontmatter
- One subdirectory per agent
- `final/` -- for the consolidated report

---

### `mc list` -- list entities

```
mc list <customers|projects|meetings|research> [OPTIONS]
```

| Option | Description |
|--------|-------------|
| `--status <STATUS>` | Filter by status (case-insensitive) |
| `--tag <TAG>` | Filter by tag (case-insensitive) |

Prints a formatted table with color-coded statuses:
- **Green:** active, completed, final
- **Yellow:** on-hold, draft, in-progress
- **Blue:** prospect, scheduled
- **Red:** inactive, cancelled, churned, outdated

---

### `mc show` -- display entity details

```
mc show <ID>
```

Accepts any entity ID (e.g. `CUST-001`, `PROJ-001`, `MTG-001`, `RES-001`). Prints all frontmatter fields and the first 20 non-empty lines of body content.

---

### `mc validate` -- check repo integrity

```
mc validate
```

Runs these checks across all entities:

| Check | Description |
|-------|-------------|
| Folder naming | Directories match `PREFIX-NNN-slug` pattern |
| Meeting filenames | Files match `YYYY-MM-DD-title.md` pattern |
| Index file presence | `_index.md` (customers, research) or `overview.md` (projects) exists |
| Frontmatter presence | YAML frontmatter block (`---`) is present |
| YAML validity | Frontmatter parses as valid YAML |
| Required fields | `id` is present; `name` for customers/projects; `title` for meetings/research |
| ID prefix | ID starts with the correct prefix for its entity type |
| Status validity | Status value is in the configured allowed list |
| Slug consistency | Frontmatter slug matches the directory name |

Exits with code 1 if any issues are found.

---

### `mc index` -- rebuild JSON indexes

```
mc index
```

Parses frontmatter from all entities and writes:
- `data/index.json` -- combined index of all entities
- `data/customers.json` -- customers only
- `data/projects.json` -- projects only
- `data/research.json` -- research only

Each record includes the frontmatter fields plus a `_source` path. These files are gitignored and regenerated on demand.

---

### `mc export` -- export to ZIP

```
mc export customer <ID_OR_SLUG>
```

Accepts a customer ID (`CUST-001`) or slug (`acme-inc`). Writes a ZIP archive to `archive/CUST-NNN-slug-YYYY-MM-DD.zip` containing the entire customer directory.

---

### `mc status` -- repo overview

```
mc status
```

Prints entity counts broken down by status with ASCII bar charts, plus the 5 most recently modified files.

---

### `mc serve` -- local dashboard

```
mc serve [--port <PORT>]
```

Starts a web server on `127.0.0.1` (default port: `5000`).

| Route | Description |
|-------|-------------|
| `/` | Dashboard with status counts and recent activity |
| `/customers` | Customer list (supports `?status=` and `?tag=` query params) |
| `/projects` | Project list (supports `?status=` and `?tag=` query params) |
| `/meetings` | Meeting list (supports `?status=` and `?tag=` query params) |
| `/research` | Research list (supports `?status=` and `?tag=` query params) |
| `/entity/{id}` | Entity detail page |

### `mc mcp` -- MCP server

Starts a [Model Context Protocol](https://modelcontextprotocol.io) (MCP) server over stdio. This allows AI assistants (Claude Code, Cursor, Windsurf, VS Code + Copilot, etc.) to interact with MissionControl directly -- listing entities, creating new ones, validating the repo, and more.

```
mc mcp
```

The server communicates via JSON-RPC 2.0 over stdin/stdout. It is not meant to be run manually -- configure it in your editor/assistant instead (see [MCP Server Integration](#mcp-server-integration) below).

---

## Configuration

The CLI reads `config/config.yml` from the repo root. This file defines:

- **paths** -- directory names for each entity type, data, templates, and archive
- **id_prefixes** -- prefix strings per entity type (`CUST`, `PROJ`, `MTG`, `RES`)
- **statuses** -- allowed status values per entity type

See `config/config.yml` for the full schema and current values.

## MCP Server Integration

The `mc mcp` command exposes all CLI functionality as MCP tools and resources. Once connected, an AI assistant can create entities, query data, validate the repo, rebuild indexes, and more -- all without leaving the editor.

### Prerequisites

- Build `mc` from source (`cargo build --release`) or download a prebuilt binary.
- Know the **absolute path** to the `mc` binary and the repo root.

### Editor configuration

> **Note:** `--root` is a global flag and must come **before** the `mcp` subcommand in the args list.

#### Claude Code

One-line setup:

```bash
claude mcp add mc -- /absolute/path/to/mc --root /absolute/path/to/repo mcp
```

Or create `.mcp.json` in the project root manually:

```json
{
  "mcpServers": {
    "mc": {
      "command": "/absolute/path/to/mc",
      "args": ["--root", "/absolute/path/to/repo", "mcp"]
    }
  }
}
```

#### Cursor

Create `.cursor/mcp.json` in the project root:

```json
{
  "mcpServers": {
    "mc": {
      "command": "/absolute/path/to/mc",
      "args": ["--root", "/absolute/path/to/repo", "mcp"]
    }
  }
}
```

#### Windsurf

Edit `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "mc": {
      "command": "/absolute/path/to/mc",
      "args": ["--root", "/absolute/path/to/repo", "mcp"]
    }
  }
}
```

#### VS Code

Create `.vscode/mcp.json` in the project root:

```json
{
  "servers": {
    "mc": {
      "type": "stdio",
      "command": "/absolute/path/to/mc",
      "args": ["--root", "/absolute/path/to/repo", "mcp"]
    }
  }
}
```

#### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS):

```json
{
  "mcpServers": {
    "mc": {
      "command": "/absolute/path/to/mc",
      "args": ["--root", "/absolute/path/to/repo", "mcp"]
    }
  }
}
```

### Available tools

| Tool | Description | Required params |
|------|-------------|-----------------|
| `list_entities` | List entities by kind with optional filters | `kind` |
| `get_entity` | Get detailed info about an entity | `id` |
| `read_entity_file` | Read full markdown content | `id` |
| `create_customer` | Create a new customer | `name` |
| `create_project` | Create a new project | `name` |
| `create_meeting` | Create a new meeting | `title` |
| `create_research` | Create a new research topic | `title` |
| `create_task` | Create a new task | `title` |
| `move_task` | Move a task to a new status | `id`, `status` |
| `list_tasks` | List tasks with rich filtering (project, customer, priority, sprint, owner) | _(none)_ |
| `print_meeting` | Export a meeting to PDF | `id` |
| `print_research` | Export a research topic to PDF | `id` |
| `validate_repo` | Check repo structure and frontmatter | _(none)_ |
| `build_index` | Rebuild JSON index files in `data/` | _(none)_ |
| `get_status` | Get status overview with counts and recent activity | _(none)_ |

### Available resources

| URI | Description |
|-----|-------------|
| `mc://config` | Repository configuration as JSON |
| `mc://entities/customers` | All customers |
| `mc://entities/projects` | All projects |
| `mc://entities/meetings` | All meetings |
| `mc://entities/research` | All research topics |
| `mc://entities/tasks` | All tasks |

### Usage examples

Once connected, you can ask your assistant things like:

- "List all active customers"
- "Create a new meeting called 'Sprint Review' for next Monday"
- "Validate the repo structure"

### Troubleshooting

- **Server failed to connect** -- check that the binary path is absolute and the file exists.
- **Server not responding** -- ensure `--root` comes before `mcp` in the args array.
- **Tools not appearing** -- restart the editor or assistant after changing the config file.

## Project structure

```
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
└── src/
    ├── main.rs          # Entry point and error display
    ├── cli.rs           # Clap argument definitions
    ├── config.rs        # Config loading and repo root detection
    ├── data.rs          # Entity collection and JSON index generation
    ├── entity.rs        # EntityKind enum, ID assignment
    ├── error.rs         # Error types with user-facing hints
    ├── frontmatter.rs   # YAML frontmatter parsing and serialization
    ├── html.rs          # Dashboard HTML rendering
    ├── mcp.rs           # MCP server implementation
    ├── template.rs      # Template scaffolding
    ├── util.rs          # Shared helpers
    └── commands/
        ├── new.rs       # mc new
        ├── list.rs      # mc list
        ├── show.rs      # mc show
        ├── index.rs     # mc index
        ├── validate.rs  # mc validate
        ├── export.rs    # mc export
        ├── serve.rs     # mc serve
        ├── status.rs    # mc status
        └── mcp.rs       # mc mcp
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| clap | CLI argument parsing |
| serde / serde_yaml / serde_json | Serialization |
| walkdir | Directory traversal |
| chrono | Date/time handling |
| regex | Pattern matching for validation |
| colored | Terminal output styling |
| dialoguer | Interactive prompts |
| axum / tokio | Web server for `mc serve` |
| pulldown-cmark | Markdown-to-HTML conversion |
| zip | ZIP archive creation |
| rmcp | MCP server over stdio |
| schemars | JSON Schema generation for tool parameters |
| tracing-subscriber | Structured logging to stderr |
| thiserror | Error type derivation |

## Running tests

```bash
cargo test
```

## License

MIT
