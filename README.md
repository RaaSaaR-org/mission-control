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

Add `mc` to an existing project. Creates a `.mc/` folder for tasks, milestones, meetings, research, sprints, and proposals (no customers, projects, or contacts).

```bash
cd my-project
mc init --embedded
```

```
my-project/
├── .mc/
│   ├── config.yml
│   ├── tasks/
│   ├── milestones/
│   ├── meetings/
│   ├── research/
│   ├── sprints/
│   └── proposals/
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
├── milestones/
├── sprints/
└── proposals/
```

Standalone mode adds **customers**, **contacts**, and **projects** — useful for agencies, freelancers, or anyone managing multiple clients.

## What You Can Track

| Entity | Description | Example |
|--------|-------------|---------|
| **Tasks** | Work items with priority, status, dependencies | `mc new task "Fix auth bug" --priority 1` |
| **Milestones** | Work packages that group tasks, with start date and deadline | `mc new milestone "Beta" --due-date 2026-11-30` |
| **Sprints** | Time-boxed iterations | `mc new sprint "2026-W06" --goal "Auth module"` |
| **Meetings** | Notes with date, attendees, PDF export | `mc new meeting "Sprint Review" --date 2026-02-10` |
| **Research** | Multi-agent research topics, PDF export | `mc new research "LLM Benchmarks" --agents claude,gemini` |
| **Customers** | Client profiles (standalone only) | `mc new customer "Acme Corp"` |
| **Contacts** | Per-customer contacts (standalone only) | `mc new contact "Alice Smith" --customer CUST-001` |
| **Projects** | Project containers (standalone only) | `mc new project "Robot Arm" --customers CUST-001` |
| **Proposals** | Decision records (BIP/ADR-style) | `mc new proposal "Migrate to Postgres"` |

## Task Management

Tasks are first-class citizens with a full kanban workflow.

### Kanban Board

```bash
mc task board
```

```
  ◆ TASK BOARD  project=PROJ-001

  BACKLOG 2                TODO 1                   IN-PROGRESS 1            DONE 4
  ━━━━━━━━━━━━━━━━━━━━━━   ━━━━━━━━━━━━━━━━━━━━━━   ━━━━━━━━━━━━━━━━━━━━━━   ━━━━━━━━━━━━━━━━━━━━━━
  P2 TASK-003              P2 TASK-002              P1 TASK-001              P3 TASK-007
  Write tests              Update docs              Fix auth bug             Set up CI
  @alice                   due Oct 30 · @bob        due Oct 09 · @alice
```

Columns follow your configured task statuses. Cards are sorted by priority (done: most recently updated first); each column shows up to `--limit` cards (done: 5), `--all` shows everything including cancelled. Filter with `--project`, `--customer`, `--sprint`, `--milestone` or `--owner`. Narrow terminals get a stacked layout.

### Move Tasks

```bash
mc task move TASK-001 in-progress
mc task move 1 done          # bare numbers and task-1 work too
mc task move 7 wip           # case-insensitive, common aliases (wip, doing, open, finished)
```

Typos get a suggestion instead of a silent failure:

```
error: 'doen' is not a valid task status
  hint: did you mean 'done'? Valid: backlog, todo, in-progress, review, done, cancelled
```

Moving between active and finished statuses moves the file between `todo/` and `done/`. After finishing a task, mc tells you what is next.

### Change Task Fields

```bash
mc task set TASK-001 --priority 1 --owner alice
mc task set 7 --due-date 2026-10-31 --sprint 2026-W05   # sprint by ID or title
mc task set 7 --tags "backend,api" --depends-on TASK-003
mc task set 7 --milestone MS-001                        # milestone by ID or title
mc task set 7 --owner "" --due-date ""                  # empty values clear a field
```

`--title`, `--status`, `--project`, `--customer` work too; list options replace the whole list. Values are checked like `mc new task` (references must exist, dates are `YYYY-MM-DD`) and nothing is written if one is wrong. The dashboard's edit form, the MCP `update_task` tool and `PATCH /v1/tasks/{id}` do the same.

### Next Task

Get the highest-priority unblocked task (`todo` before `backlog`, then priority, then due date):

```bash
mc task next
mc task next -n 5 --owner alice   # the top of the queue
```

### Filtering

```bash
mc list tasks --status in-progress --priority 1
mc list tasks --sprint 2026-W06 --owner alice
mc list tasks --milestone MS-001         # "" lists tasks without a milestone
mc list tasks --open --sort due          # hide done/cancelled, earliest due first
mc list tasks --overdue                  # open tasks past their due date
mc task board --project proj-1           # same as --project PROJ-001
```

Status filters are validated against your config, so `--status in-prog` suggests `in-progress` rather than returning an empty list. `--project` and `--customer` take loose IDs, and an ID that doesn't exist fails with a "did you mean" hint instead of an empty list. `--sort` takes `id` (default), `priority`, `due` or `updated`. `mc list meetings` is in date order.

### Dependencies

Tasks can depend on other tasks:

```bash
mc new task "Deploy to prod" --depends-on TASK-001,TASK-002
```

Blocked tasks are hidden from `mc task next` until their dependencies are done.

### Milestones

Milestones (work packages, epics, delivery milestones) group tasks around a description, a planned start and a deadline. They are independent of sprints.

```bash
mc new milestone "AP3: Integration" --description "Integrate and train the system" \
  --start-date 2026-09-01 --due-date 2026-11-30 --projects PROJ-001
mc new task "Collect demonstrations" --milestone MS-001
mc task set TASK-024 --milestone "AP3: Integration"   # ID, loose ID or unique title
mc task set TASK-024 --milestone ""                   # clear the assignment
mc list milestones --status active
mc task board --milestone MS-001
```

A task stores its milestone as `milestone: "[[MS-001]]"`. Milestones live in `milestones/MS-NNN-<slug>/` (`.mc/milestones/` in embedded mode) with the statuses `planned`, `active`, `completed` and `cancelled`; `paths.milestones`, `id_prefixes.milestone` and `statuses.milestone` override the defaults. Repos that have tasks get milestones without a config change. The dashboard's **Milestones** page (`/milestones`) shows them as a Gantt chart; the REST API has `POST /v1/milestones` and the MCP server `create_milestone`.

### Checklists and Comments

Markdown task lists in any entity (`- [ ] Book room`) are a checklist you can tick from the terminal, the dashboard or an agent. Only the box character changes; nothing else in the file is touched.

```bash
mc check TASK-069              # numbered list with progress
mc check TASK-069 2            # tick item 2
mc check TASK-069 2 --uncheck  # untick it
```

Tasks and meetings take comments. They are appended to a `## Comments` section at the end of the file, one `### 2026-10-06 14:32 · Author` heading each, so they diff cleanly and read fine in any editor. The author defaults to `git config user.name` (else `$USER`). Control characters are dropped and an unclosed code fence or HTML block is closed at the end of the comment, so one comment can't swallow the ones after it.

```bash
mc comment TASK-069 "Shipped, see TASK-070"
mc comment TASK-069                              # write it in $VISUAL / $EDITOR
git log -1 --format=%B | mc comment TASK-069 -   # text from stdin
```

## AI Agent Integration (MCP)

Connect your AI editor to `mc` with one command:

Editors start MCP servers from a working directory of their choosing (Claude Desktop often uses `/`), so point `mc` at your repo with `--root`.

**Claude Code** (run inside the repo):
```bash
claude mcp add mc -- mc --root "$PWD" mcp
```

**Cursor, Windsurf, Claude Desktop** (add to your MCP config):
```json
{
  "mcpServers": {
    "mc": {
      "command": "mc",
      "args": ["--root", "/path/to/your/repo", "mcp"]
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
      "args": ["--root", "${workspaceFolder}", "mcp"]
    }
  }
}
```

Now your AI assistant can create tasks, move them through the board, query status, create meetings, and more.

### Available MCP Tools

All tools return JSON with documented fields. Parameter descriptions include valid values, defaults, and examples — AI agents can discover the full API from the tool schema alone.

| Tool | Description |
|------|-------------|
| `get_status` | Status overview with per-entity counts and recent activity |
| `list_entities` | List entities by kind with optional status/tag filters |
| `list_tasks` | List tasks with rich filtering (status, project, customer, priority, sprint, milestone, owner, tag) |
| `get_entity` | Get entity detail with frontmatter fields and body preview |
| `read_entity_file` | Read full markdown content (YAML frontmatter + body) |
| `create_task` | Create a task (with priority, sprint, milestone, dependencies, scoping) |
| `create_milestone` | Create a milestone / work package |
| `create_sprint` | Create a sprint |
| `create_meeting` | Create a meeting |
| `create_research` | Create a research topic |
| `create_customer` | Create a customer (standalone only) |
| `create_project` | Create a project (standalone only) |
| `create_contact` | Create a contact under a customer (standalone only) |
| `create_proposal` | Create a proposal / decision record |
| `move_task` | Move a task to a new status, optionally assign a sprint |
| `update_task` | Change a task's title, status, priority, owner, sprint, milestone, due date, projects, customers, tags or dependencies |
| `next_tasks` | What to work on next: unblocked open tasks, best first, plus how many are blocked |
| `list_checklist` | List an entity's `- [ ]` checklist items with progress |
| `check_item` | Tick or untick a checklist item (only that character changes) |
| `add_comment` | Comment on a task or meeting |
| `print_meeting` | Export meeting to PDF |
| `print_research` | Export research to PDF |
| `print_file` | Generate branded PDF from any markdown file |
| `validate_repo` | Validate repo structure and frontmatter |
| `build_index` | Rebuild the `data/*.json` index files (for external tools) |

### MCP Resources

Resources provide read-only data snapshots. Start with `mc://config` to discover valid status values and ID prefixes.

| Resource | Description |
|----------|-------------|
| `mc://config` | Valid status values, ID prefixes, and directory paths |
| `mc://entities/customers` | All customers (standalone only) |
| `mc://entities/contacts` | All contacts (standalone only) |
| `mc://entities/projects` | All projects (standalone only) |
| `mc://entities/meetings` | All meetings |
| `mc://entities/research` | All research topics |
| `mc://entities/tasks` | All tasks (unfiltered — use `list_tasks` tool for filtering) |
| `mc://entities/milestones` | All milestones |
| `mc://entities/sprints` | All sprints |
| `mc://entities/proposals` | All proposals |

## Web Dashboard

Browse and update your data in a local web UI:

```bash
mc serve                                  # http://localhost:5000, editing on
mc serve --read-only                      # browse only
mc serve --base-path /hq                  # behind a reverse proxy at /hq (read-only)
mc serve --base-path /hq --allow-edits    # behind a proxy, with editing
```

- **Overview**: a six-week flight plan (open task deadlines as ticks coloured by urgency, meetings as diamonds, a magenta line for today, older overdue work in a separate bin), overdue and soon-due tasks, upcoming meetings, active sprint progress against plan, a status breakdown per entity type, and recently changed files.
- **Tasks**: a kanban board (lanes follow your configured task statuses, late counts per lane, finished work collapsed) and a sortable, filterable list with a compact-rows toggle.
- **Milestones** (`/milestones`): a Gantt chart of the work packages with their planned window, task completion, deadline diamonds and a line for today, filterable by project. Expand a row for its description and its tasks' deadlines, or open its filtered task list. The overview shows the open milestones in a compact chart.
- **Meeting calendar** (`/meetings/calendar?month=2026-10`, or **Calendar** on the meetings list): a month grid with weeks starting on Monday, ISO week numbers and today in magenta. Meetings sit on their `date` with their `time` and status; busy days fold into "+n more". A switch overlays sprints as bands and open task deadlines as ticks. On phones the grid becomes an agenda grouped by day. Meetings without a valid `date` are listed under **Undated**.
- **Editing**: drag cards between lanes, or use a card's ⋮ menu (or focus it and press `m`) to move it; every move can be undone from its toast. On a task's page, **Edit** (`e`) changes title, status, priority, owner, milestone, sprint, due date, project and customer. **New task** (`c`, anywhere) creates a task with the same logic as `mc new task`, prefilled from the page you're on. Changes are written straight to the Markdown files.
- **Checklists and comments**: `- [ ]` items in any page's notes can be ticked in place, with a progress bar above the notes; if the file changed on disk meanwhile, the tick is refused instead of hitting the wrong line. Tasks and meetings show their comments below the notes and a composer (`⌘↵` posts) that writes to the file's `## Comments` section.
- **Command palette**: `⌘K` / `Ctrl K` or `/` jumps to any entity by ID, number, name or tag, to any page, or runs an action (new task, switch theme). The last row searches all notes.
- **Previews**: rest the mouse on any link to an entity (a `[[TASK-069]]` in notes, a related task, a board card, a table row), or tab to it, and a small card shows its status, priority, owner, due date, sprint/project/customer, meeting time and attendees, the first lines of its notes, checklist progress and how many comments it has. `esc` closes it; on touch screens a tap just opens the link.
- **Live refresh**: when the files change on disk (the CLI, an editor, `git pull`), the open page updates itself, or offers a reload if you're in the middle of something.
- **Lists and detail pages**: filter by status or tag, sort by column, narrow rows with the filter box; detail pages show a title block, named `[[ID|alias]]` links and everything that references the entity, upcoming meetings first.
- **Linked files**: relative links and images in notes (`notes/deep-dive.md`, `../RES-002-x/RES-002.md`, `assets/photo.jpg`) work. Links to another entity's file open its page; other Markdown files in the repo open as a page of their own, and images and PDFs are shown as they are. Hidden paths (`.git`, `.env`) and other file types are never served. Wide tables become stacked rows on phones.

**Keyboard shortcuts** (press `?` in the dashboard for the full list): `⌘K` or `/` palette, `c` new task, `g d` overview, `g t` task board, `g l` task list, `g m` / `g c` / `g p` / `g r` / `g s` meetings, customers, projects, research, sprints, `b` / `l` board or list, `←` / `→` (or `[` / `]`) previous or next month and `.` this month in the calendar, `j` / `k` move through rows or cards, `m` move the focused card, `e` edit the task, `t` switch light and dark, `esc` close.

**Editing and security.** The server only listens on 127.0.0.1. Write requests must carry an `X-MC-Request: 1` header and come from the dashboard's own origin; locally every request, reads included, must also address `localhost` (or `127.0.0.1`), which stops other websites (including DNS-rebinding tricks) from reading or changing your files. Pages can't be framed by other sites, and files served from the repo can't run scripts. Every value is checked against your configured statuses, priorities and existing sprints, projects and customers. With `--base-path` the dashboard is read-only unless you pass `--allow-edits`; put authentication in front of it before you do. Read-only mode hides all edit controls and answers writes with 403. Every page still works without JavaScript; editing needs it.

The dashboard follows the system light/dark setting; a switch in the sidebar (or `t`) overrides it per browser. It works on phones and in print.

To theme it, set `brand.custom_css` in `config.yml` and override the `--mc-*` CSS custom properties: `--mc-blue` is the primary colour, `--mc-amber` the caution/accent colour, `--mc-route` the "today" marker, and `--mc-surface*`, `--mc-text-*` and `--mc-border-light` the neutrals. Fonts come from `--font-sans`, `--font-display`, `--font-readout` and `--font-mono`. The built-in tokens sit in a cascade layer, so any rule in your stylesheet wins; a stylesheet that pins a single `color-scheme` locks the theme and hides the switch. Relative `url(...)` references in that stylesheet (fonts, images) resolve against the stylesheet's own directory.

## REST API

For non-LLM clients (custom UIs, mobile apps, CI bots, multi-tenant gateways), `mc api serve` exposes the same surface as MCP over HTTP/JSON with bearer auth and an OpenAPI 3.1 spec.

```bash
mc api hash-token "your-secret" > hash.txt
cat > tokens.yml <<EOF
tokens:
  - name: deploy-bot
    hash: "$(cat hash.txt)"
    capabilities: [read, write]
EOF
mc api serve --tokens-file tokens.yml --port 5100
```

```bash
curl -H "Authorization: Bearer your-secret" http://127.0.0.1:5100/v1/tasks
```

See [`docs/api.md`](docs/api.md) for the full reference (endpoints, auth, error model, multi-tenant gateway pattern) and [`docs/examples/curl-cookbook.md`](docs/examples/curl-cookbook.md) for copy-paste recipes. The live spec is at `/v1/openapi.json`.

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

### Shell Completions

```bash
# bash
mc completions bash > ~/.local/share/bash-completion/completions/mc
# zsh (put ~/.zfunc on $fpath before compinit in ~/.zshrc: fpath+=~/.zfunc)
mc completions zsh > ~/.zfunc/_mc
# fish
mc completions fish > ~/.config/fish/completions/mc.fish
```

## Configuration

Config lives at `.mc/config.yml` (embedded) or `config/config.yml` (standalone). It controls:

- Directory paths for each entity type
- ID prefixes (`CUST`, `CONT`, `PROJ`, `MTG`, `RES`, `TASK`, `MS`, `SPR`, `PROP`)
- Allowed status values per entity type

The defaults work out of the box. Edit the config when you want to customize status workflows or add new prefixes.

## CLI Reference

| Command | Description |
|---------|-------------|
| `mc init` | Initialize a new repo (use `--embedded` for existing projects) |
| `mc new <type> "name"` | Create a new entity |
| `mc list <type>` (`mc ls`) | List entities with optional filters; tables adapt to the terminal width |
| `mc show <ID> [--raw] [--open]` | Entity details with the notes rendered for the terminal, clickable links and a pager |
| `mc task board` | Kanban board view |
| `mc task move <ID> <status>` | Change task status |
| `mc task set <ID> --field value...` | Change task fields (title, status, priority, owner, sprint, milestone, due date, links, tags, dependencies) |
| `mc task next [-n N]` | Show next actionable task(s) |
| `mc check <ID> [N] [--uncheck]` | List an entity's checklist, or tick/untick item N |
| `mc comment <ID> ["text"]` | Comment on a task or meeting (`-` reads stdin, no text opens `$VISUAL` / `$EDITOR`, `--author` overrides) |
| `mc validate` | Check repo structure and frontmatter, grouped by file |
| `mc status` | Dashboard: counts per status, focus (overdue, in progress, next up), recent activity |
| `mc print meeting <ID>` | Export meeting to PDF |
| `mc print research <ID>` | Export research to PDF |
| `mc print file <path>` | Generate branded PDF from any markdown file |
| `mc index` | Rebuild JSON index files (`data/*.json`) |
| `mc export customer <ID or slug>` | Export a customer folder to a zip archive |
| `mc serve` | Start web dashboard |
| `mc mcp` | Start MCP server |
| `mc api serve` | Start the REST/JSON API (`--tokens-file`, `--bind`, `--port`, `--read-only`) |
| `mc api hash-token [SECRET]` | Hash a bearer token for `tokens.yml` (reads stdin without SECRET) |
| `mc completions <shell>` | Print a completion script for bash, zsh, fish, elvish or powershell |

Use `mc <command> --help` for detailed options; the command's own options come first, the global ones (`--root`, `-y`, `--json`, `--color`) after them.

IDs are forgiving: `TASK-007`, `task-7` and `task7` are the same, and unknown IDs come with a "did you mean" hint.

`mc show` renders the notes on a terminal: headings, lists and checklists (`☐`/`☑`), quotes, boxed code blocks and tables with borders that wrap to fit the width. Checklist progress sits above the notes, and the comments of tasks and meetings get their own section below them. Entity references (`TASK-069`, `[[TASK-069|alias]]`) and relative file links are clickable OSC 8 hyperlinks to the file on disk (cmd/ctrl-click in iTerm2, WezTerm, kitty, Ghostty, VS Code). A footer lists the linked entities with status, title and path, plus what references this one. Output taller than the terminal goes through `$MC_PAGER`, `$PAGER` or `less` (`--no-pager` or `MC_PAGER=cat` to turn it off). `--raw` prints the markdown source, `--open` opens the file in `$VISUAL` / `$EDITOR` (or the system opener). Piped output keeps the plain layout (fields plus the Markdown body as written). Control characters in a note never reach the terminal, and file links only open files inside the repo.

### Output, colors and scripting

- **`--json`** prints machine-readable output for `list`, `show`, `status`, `validate`, `index`, `export`, `task board|move|next|set`, `check`, `comment` and `new` (`{"id", "kind", "title"|"name", "path"}` with a repo-relative path; no prompts). Entities have the same shape as `data/*.json` (wiki-links stripped, `_source` path). In JSON mode errors, argument errors included, are written to stderr as `{"error": {"message", "hint", "exit_code"}}`.
- **Pipes stay plain.** When stdout is not a terminal, every command prints plain ASCII without colors: `mc list` tables have no status glyphs, rules or truncation (empty cells print as `-`), symbols in `status`, `show`, `task board|next|move` and the rest use their ASCII forms, hints are skipped, and empty results print nothing to stdout, so `mc list tasks | tail -n +2 | wc -l` and `awk` work as expected. For anything you parse, prefer `--json`.
- **Colors:** `--color auto|always|never`, which also covers `--help` and argument errors; `NO_COLOR` and `CLICOLOR_FORCE` are respected, and `TERM=dumb` turns colors off.
- **Escapes from files:** titles, owners and other values from files are printed without control characters, so a file can't set the window title, clear the screen or fake a link.
- **Glyphs:** on a terminal, unicode glyphs (`✓ ● ◐ ○ ◌ ✗ →`); ASCII (`ok x -> |`) when stdout is piped or when `MC_ASCII=1`, `TERM=dumb` or a non-UTF-8 locale is set.
- **Width:** tables hide optional columns and truncate long titles to fit; set `MC_WIDTH=<n>` to force a width (`0` = unlimited).
- **Hyperlinks:** on a colour terminal (not `TERM=dumb`) `mc show` emits OSC 8 links; `MC_HYPERLINKS=0` turns them off, `MC_HYPERLINKS=1` forces them on in the rendered terminal view (piped output keeps the plain layout, without links).
- **Exit codes:** `0` success, `1` failure (not found, validation issues, I/O), `2` invalid usage (bad option, status, priority, date or ID, or an entity kind this repo does not enable).

## Contributing

CI runs on every push and PR:
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- `cargo build --release`

See [CLAUDE.md](CLAUDE.md) for architecture details and build commands.

## License

MIT
