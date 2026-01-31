# mc -- MissionControl CLI

A command-line tool for managing git-based knowledge repositories. Scaffolds new entities, validates repo structure, builds JSON indexes, exports archives, and serves a local dashboard.

## Installation

### From crates.io

```bash
cargo install mc
```

### Prebuilt binaries

Download the latest release for your platform from the [GitHub Releases](https://github.com/RaaSaaR-org/mission-control/releases) page:

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
cargo install --git https://github.com/RaaSaaR-org/mission-control
```

Or clone and build locally:

```bash
git clone https://github.com/RaaSaaR-org/mission-control.git
cd mission-control
cargo build --release
# Binary is at target/release/mc
```

## Usage

```
mc [--root <PATH>] [-y|--yes] <COMMAND>
```

| Flag | Description |
|------|-------------|
| `--root <PATH>` | Path to repo root (auto-detected if omitted) |
| `-y, --yes` | Skip interactive prompts and use defaults |

## Commands

| Command | Description |
|---------|-------------|
| `mc new <type> "<name>"` | Scaffold a new entity from template |
| `mc list <type>` | List entities with optional `--status` / `--tag` filters |
| `mc list tasks` | List tasks with `--project`, `--customer`, `--priority`, `--sprint`, `--owner` filters |
| `mc show <ID>` | Display entity details |
| `mc validate` | Check naming conventions, frontmatter, and repo structure |
| `mc index` | Rebuild JSON indexes in `data/` |
| `mc export customer <ID>` | Export a customer folder to ZIP |
| `mc status` | Repo overview with status counts |
| `mc serve [--port PORT]` | Local web dashboard (default port: 5000) |
| `mc mcp` | Start MCP server over stdio |

### `mc new`

Creates directories, files, and YAML frontmatter from templates. IDs are assigned automatically.

```bash
mc new customer "Acme Corp" --owner alice --status active --tags "robotics,eu"
mc new project "Robot Arm" --customers CUST-001
mc new meeting "Sprint Review" --date 2026-02-03 --time 14:00 --duration 1h
mc new research "Humanoid Robotics" --agents claude,gemini,perplexity
mc new task "Fix sensor calibration" --project PROJ-001 --priority 2
```

### `mc validate`

Checks all entities for:
- Folder/file naming conventions
- Frontmatter presence and YAML validity
- Required fields (`id`, `name`/`title`, `status`)
- ID prefix correctness
- Status values against `config/config.yml`

Exits with code 1 if any issues are found.

## Configuration

The CLI reads `config/config.yml` from the repo root. This file defines directory paths, ID prefixes (`CUST`, `PROJ`, `MTG`, `RES`, `TASK`), and allowed status values per entity type.

## MCP Server Integration

The `mc mcp` command exposes all CLI functionality as MCP tools and resources, allowing AI assistants to interact with MissionControl directly.

### Editor setup

> `--root` is a global flag and must come **before** the `mcp` subcommand.

If `mc` is installed via `cargo install mc` (or a prebuilt binary on your `PATH`), use `mc` as the command. Otherwise replace with the full path to the binary.

**Claude Code:**

```bash
claude mcp add mc -- mc --root /absolute/path/to/repo mcp
```

**Cursor** (`.cursor/mcp.json`), **Windsurf** (`~/.codeium/windsurf/mcp_config.json`), **Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "mc": {
      "command": "mc",
      "args": ["--root", "/absolute/path/to/repo", "mcp"]
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
      "args": ["--root", "/absolute/path/to/repo", "mcp"]
    }
  }
}
```

### Available MCP tools

| Tool | Description |
|------|-------------|
| `list_entities` | List entities by kind with optional filters |
| `get_entity` | Get detailed info about an entity |
| `read_entity_file` | Read full markdown content |
| `create_customer` | Create a new customer |
| `create_project` | Create a new project |
| `create_meeting` | Create a new meeting |
| `create_research` | Create a new research topic |
| `create_task` | Create a new task |
| `move_task` | Move a task to a new status |
| `list_tasks` | List tasks with rich filtering |
| `print_meeting` | Export a meeting to PDF |
| `print_research` | Export a research topic to PDF |
| `validate_repo` | Check repo structure and frontmatter |
| `build_index` | Rebuild JSON index files |
| `get_status` | Status overview with counts and recent activity |

### Available MCP resources

| URI | Description |
|-----|-------------|
| `mc://config` | Repository configuration |
| `mc://entities/customers` | All customers |
| `mc://entities/projects` | All projects |
| `mc://entities/meetings` | All meetings |
| `mc://entities/research` | All research topics |
| `mc://entities/tasks` | All tasks |

## CI/CD

### Continuous integration

Every push to `main` and every pull request runs:

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo test`
4. `cargo build --release`

### Releasing a new version

Releases are fully automated. When you push a version tag, the pipeline builds platform binaries, creates a GitHub Release, and publishes to crates.io.

1. Bump the version in `Cargo.toml`
2. Commit and push:
   ```bash
   git add Cargo.toml Cargo.lock
   git commit -m "Bump version to 0.2.0"
   git push
   ```
3. Tag and push:
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

| Job | What it does |
|-----|--------------|
| **build** | Cross-compiles binaries for Linux and macOS (amd64 + arm64) |
| **release** | Attaches `.tar.gz` archives to a GitHub Release |
| **publish** | Publishes the crate to [crates.io](https://crates.io/crates/mc) |

**One-time setup:** Add your crates.io API token as the GitHub secret `CARGO_REGISTRY_TOKEN` in repo **Settings > Secrets and variables > Actions**.

## License

MIT
