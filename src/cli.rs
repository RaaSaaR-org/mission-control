use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "mc",
    about = "MissionControl CLI -- manage customers, contacts, projects, meetings, research, tasks, and proposals",
    version,
    after_help = "\x1b[1mExamples:\x1b[0m
  mc status                          Show dashboard
  mc list customers --status active  List active customers
  mc show CUST-001                   Show customer details
  mc new customer \"Acme Inc\"         Create a new customer (interactive)
  mc -y new customer \"Acme Inc\"      Create with defaults (no prompts)
  mc new task \"Fix bug\" --project PROJ-001 --priority 2
  mc task board --project PROJ-001   Show kanban board
  mc task move TASK-001 in-progress  Change task status
  mc task next                       Show next actionable task
  mc validate                        Check repo structure
  mc index                           Rebuild JSON indexes
  mc init                            Initialize a new MissionControl repo
  mc init --project                  Initialize a lightweight project repo"
)]
pub struct Cli {
    /// Path to repo root (auto-detected if omitted)
    #[arg(long)]
    pub root: Option<String>,

    /// Skip interactive prompts (use defaults)
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a new entity
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new customer \"Acme Inc\"
  mc new contact \"Alice Smith\" --customer CUST-001
  mc new project \"Data Pipeline\" --owner alice --status active
  mc new meeting \"Weekly Sync\" --date 2025-06-01 --time 14:00
  mc new research \"LLM Benchmarks\" --agents claude,gemini
  mc new task \"Fix login bug\" --project PROJ-001 --priority 1
  mc new sprint \"2026-W05\" --start-date 2026-01-27 --end-date 2026-02-07 --goal \"Auth module\"
  mc new proposal \"Use PostgreSQL\" --type architecture --author alice")]
    New {
        #[command(subcommand)]
        entity: NewEntity,
    },
    /// List entities
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list customers
  mc list customers --status active
  mc list contacts --customer CUST-001
  mc list projects --tag ml
  mc list meetings --status scheduled
  mc list tasks --status in-progress --project PROJ-001
  mc list sprints --status active
  mc list proposals --status accepted")]
    List {
        #[command(subcommand)]
        entity: ListEntity,
    },
    /// Show details for an entity by ID
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc show CUST-001
  mc show CONT-001
  mc show PROJ-001
  mc show MTG-001
  mc show RES-001
  mc show TASK-001
  mc show SPR-001
  mc show PROP-001")]
    Show {
        /// Entity ID (e.g., CUST-001, CONT-001, PROJ-001, MTG-001, TASK-001, SPR-001, PROP-001)
        id: String,
    },
    /// Rebuild entity index files (data/*.json)
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc index          Rebuild all JSON indexes from entity files")]
    Index,
    /// Export an entity to a zip archive
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc export customer CUST-001     Export customer folder to a zip file")]
    Export {
        #[command(subcommand)]
        entity: ExportEntity,
    },
    /// Generate a branded PDF from a meeting, research entity, or any markdown file
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc print meeting MTG-001                    Meeting notes to PDF
  mc print meeting MTG-001 -o meeting.pdf     Custom output path
  mc print research RES-001                   Research final report to PDF
  mc print research RES-001 --file report.md  Specific file from final/
  mc print file ./docs/architecture.md        Any markdown file to PDF
  mc print file ./notes/kickoff.md -t meeting Use meeting cover template")]
    Print {
        #[command(subcommand)]
        entity: PrintEntity,
    },
    /// Validate repo structure and frontmatter
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc validate       Check all entities for missing/invalid frontmatter fields

Prints warnings for each issue found, or \"All files valid\" if clean.")]
    Validate,
    /// Show a dashboard with counts and recent activity
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc status         Show entity counts, recent activity, and task summary")]
    Status,
    /// Start a local web server to browse all MissionControl data
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc serve                Start on default port 5000
  mc serve --port 8080    Start on port 8080

Open http://localhost:<port> in your browser to view the dashboard.")]
    Serve {
        /// Port to listen on
        #[arg(long, default_value_t = 5000)]
        port: u16,
    },
    /// Start an MCP (Model Context Protocol) server over stdio
    #[command(after_help = "\x1b[1mIntegration:\x1b[0m
  Claude Code  Add to .mcp.json:  {\"mcpServers\":{\"mc\":{\"command\":\"mc\",\"args\":[\"mcp\"]}}}
  Cursor       Settings > MCP > Add server: command = mc, args = [\"mcp\"]
  VS Code      Add to .vscode/mcp.json with command \"mc\" and args [\"mcp\"]")]
    Mcp,
    /// Initialize a new MissionControl repository
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc init                            Full setup (customers, projects, meetings, etc.)
  mc init --project                  Lightweight project setup (tasks, meetings, research)
  mc init --embedded                 Embedded .mc/ folder in an existing project
  mc init --name \"My Project\"        Set the repo name
  mc init /path/to/dir               Initialize in a specific directory
  mc -y init                         Skip prompts, use defaults
  mc init --force                    Reinitialize even if config exists")]
    Init {
        /// Create a lightweight project-only repo (tasks, meetings, research)
        #[arg(long)]
        project: bool,

        /// Create an embedded .mc/ folder inside an existing project
        #[arg(long)]
        embedded: bool,

        /// Repository or project name
        #[arg(long)]
        name: Option<String>,

        /// Target directory (defaults to current directory)
        path: Option<String>,

        /// Overwrite existing config
        #[arg(long)]
        force: bool,
    },
    /// Task management commands (board, move, next)
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc task board                           Show kanban board
  mc task board --project PROJ-001        Board for a project
  mc task move TASK-001 in-progress       Change task status
  mc task move TASK-001 done              Complete a task
  mc task next                            Show next actionable task
  mc task next --project PROJ-001         Next task for a project")]
    Task {
        #[command(subcommand)]
        subcmd: TaskSubcommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum NewEntity {
    /// Create a new customer
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new customer \"Acme Inc\"
  mc new customer \"Acme Inc\" --owner alice --status active
  mc new customer \"Acme Inc\" --tags \"enterprise,priority\"")]
    Customer {
        /// Customer name
        name: String,
        /// Owner (username or name)
        #[arg(long)]
        owner: Option<String>,
        /// Status (e.g., active, inactive, prospect)
        #[arg(long)]
        status: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
    },
    /// Create a new project
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new project \"Data Pipeline\"
  mc new project \"Data Pipeline\" --owner bob --status active
  mc new project \"Data Pipeline\" --customers CUST-001 --tags \"ml,infra\"")]
    Project {
        /// Project name
        name: String,
        /// Owner (username or name)
        #[arg(long)]
        owner: Option<String>,
        /// Status (e.g., active, completed, on-hold)
        #[arg(long)]
        status: Option<String>,
        /// Linked customer IDs (comma-separated)
        #[arg(long)]
        customers: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
    },
    /// Create a new meeting
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new meeting \"Weekly Sync\"
  mc new meeting \"Sprint Review\" --date 2025-06-01 --time 14:00 --duration 1h
  mc new meeting \"Client Call\" --customers CUST-001 --projects PROJ-001
  mc new meeting \"Standup\" --status scheduled --tags \"recurring\"")]
    Meeting {
        /// Meeting title
        title: String,
        /// Date (YYYY-MM-DD, defaults to today)
        #[arg(long)]
        date: Option<String>,
        /// Time (HH:MM)
        #[arg(long)]
        time: Option<String>,
        /// Duration (e.g., 30m, 1h)
        #[arg(long)]
        duration: Option<String>,
        /// Status (e.g., scheduled, completed)
        #[arg(long)]
        status: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// Linked customer IDs (comma-separated)
        #[arg(long)]
        customers: Option<String>,
        /// Linked project IDs (comma-separated)
        #[arg(long)]
        projects: Option<String>,
        /// Comma-separated attendee names
        #[arg(long)]
        attendees: Option<String>,
    },
    /// Create a new research topic
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new research \"LLM Benchmarks\"
  mc new research \"LLM Benchmarks\" --owner alice --agents claude,gemini
  mc new research \"LLM Benchmarks\" --tags \"ai,benchmarks\"")]
    Research {
        /// Research title
        title: String,
        /// Owner (username or name)
        #[arg(long)]
        owner: Option<String>,
        /// Comma-separated AI agent names (e.g., claude, gemini)
        #[arg(long)]
        agents: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
    },
    /// Create a new task
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new task \"Select motors\" --project PROJ-001 --priority 2
  mc new task \"Review contract\" --customer CUST-001
  mc new task \"Update CI pipeline\"
  mc new task \"Write tests\" --sprint 2026-W05 --depends-on TASK-001
  mc -y new task \"Quick task\" --project PROJ-001")]
    Task {
        /// Task title
        title: String,
        /// Link to a project (e.g., PROJ-001)
        #[arg(long)]
        project: Option<String>,
        /// Link to a customer (e.g., CUST-001)
        #[arg(long)]
        customer: Option<String>,
        /// Owner
        #[arg(long)]
        owner: Option<String>,
        /// Initial status (default: backlog)
        #[arg(long)]
        status: Option<String>,
        /// Priority: 1=critical, 2=high, 3=medium, 4=low (default: 3)
        #[arg(long)]
        priority: Option<u32>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// Sprint label (e.g., 2026-W05)
        #[arg(long)]
        sprint: Option<String>,
        /// Comma-separated task IDs this depends on
        #[arg(long)]
        depends_on: Option<String>,
        /// Due date (YYYY-MM-DD)
        #[arg(long)]
        due_date: Option<String>,
    },
    /// Create a new sprint
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new sprint \"2026-W05\"
  mc new sprint \"2026-W05\" --start-date 2026-01-27 --end-date 2026-02-07
  mc new sprint \"2026-W05\" --goal \"Complete auth module\" --projects PROJ-001")]
    Sprint {
        /// Sprint title (e.g., 2026-W05)
        title: String,
        /// Owner (username or name)
        #[arg(long)]
        owner: Option<String>,
        /// Status (default: planning)
        #[arg(long)]
        status: Option<String>,
        /// Sprint goal
        #[arg(long)]
        goal: Option<String>,
        /// Start date (YYYY-MM-DD, defaults to today)
        #[arg(long)]
        start_date: Option<String>,
        /// End date (YYYY-MM-DD)
        #[arg(long)]
        end_date: Option<String>,
        /// Linked project IDs (comma-separated)
        #[arg(long)]
        projects: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
    },
    /// Create a new proposal (BIP/ADR-style decision record)
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new proposal \"Use PostgreSQL for primary database\"
  mc new proposal \"Adopt microservices\" --type architecture --author alice
  mc new proposal \"Switch to React\" --status proposed --tags \"frontend,framework\"
  mc new proposal \"New auth flow\" --supersedes PROP-001")]
    Proposal {
        /// Proposal title
        title: String,
        /// Author
        #[arg(long)]
        author: Option<String>,
        /// Status (default: draft)
        #[arg(long)]
        status: Option<String>,
        /// Proposal type: architecture, feature, or process
        #[arg(long = "type")]
        proposal_type: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// ID of proposal this supersedes (e.g., PROP-001)
        #[arg(long)]
        supersedes: Option<String>,
    },
    /// Create a new contact (standalone mode only)
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new contact \"Alice Smith\" --customer CUST-001
  mc new contact \"Bob Jones\" --customer CUST-001 --role \"VP Engineering\"
  mc new contact \"Carol Lee\" --customer CUST-002 --email carol@example.com")]
    Contact {
        /// Contact name
        name: String,
        /// Customer ID (required, e.g. CUST-001)
        #[arg(long)]
        customer: String,
        /// Role / job title
        #[arg(long)]
        role: Option<String>,
        /// Email address
        #[arg(long)]
        email: Option<String>,
        /// Phone number
        #[arg(long)]
        phone: Option<String>,
        /// Status (default: active; values: active, inactive)
        #[arg(long)]
        status: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ListEntity {
    /// List customers
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list customers
  mc list customers --status active
  mc list customers --tag enterprise")]
    Customers {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// List projects
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list projects
  mc list projects --status active
  mc list projects --tag ml")]
    Projects {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// List meetings
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list meetings
  mc list meetings --status scheduled
  mc list meetings --tag recurring")]
    Meetings {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// List research topics
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list research
  mc list research --status draft
  mc list research --tag ai")]
    Research {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// List sprints
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list sprints
  mc list sprints --status active
  mc list sprints --tag q1")]
    Sprints {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// List proposals
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list proposals
  mc list proposals --status accepted
  mc list proposals --tag architecture")]
    Proposals {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// List contacts (standalone mode only)
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list contacts
  mc list contacts --status active
  mc list contacts --customer CUST-001
  mc list contacts --tag engineering")]
    Contacts {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Filter by customer ID
        #[arg(long)]
        customer: Option<String>,
    },
    /// List tasks
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list tasks
  mc list tasks --status in-progress --project PROJ-001
  mc list tasks --priority 1 --owner alice
  mc list tasks --sprint 2026-W05")]
    Tasks {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Filter by project ID
        #[arg(long)]
        project: Option<String>,
        /// Filter by customer ID
        #[arg(long)]
        customer: Option<String>,
        /// Filter by priority (1-4)
        #[arg(long)]
        priority: Option<u32>,
        /// Filter by sprint label
        #[arg(long)]
        sprint: Option<String>,
        /// Filter by owner
        #[arg(long)]
        owner: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum TaskSubcommand {
    /// Show a kanban board view of tasks
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc task board
  mc task board --project PROJ-001
  mc task board --sprint 2026-W05
  mc task board --customer CUST-001")]
    Board {
        /// Filter by project ID
        #[arg(long)]
        project: Option<String>,
        /// Filter by customer ID
        #[arg(long)]
        customer: Option<String>,
        /// Filter by sprint label
        #[arg(long)]
        sprint: Option<String>,
    },
    /// Move a task to a new status
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc task move TASK-001 in-progress
  mc task move TASK-001 todo --sprint 2026-W05
  mc task move TASK-003 done
  mc task move TASK-003 backlog")]
    Move {
        /// Task ID (e.g., TASK-001)
        id: String,
        /// New status (backlog, todo, in-progress, review, done, cancelled)
        status: String,
        /// Assign to a sprint (e.g., 2026-W05)
        #[arg(long)]
        sprint: Option<String>,
    },
    /// Show the next actionable task
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc task next
  mc task next --project PROJ-001
  mc task next --customer CUST-001")]
    Next {
        /// Filter by project ID
        #[arg(long)]
        project: Option<String>,
        /// Filter by customer ID
        #[arg(long)]
        customer: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ExportEntity {
    /// Export a customer to a zip archive
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc export customer CUST-001       Export by ID
  mc export customer acme-inc       Export by slug")]
    Customer {
        /// Customer ID or slug (e.g., CUST-001 or acme-inc)
        id: String,
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum PrintTemplate {
    Standard,
    Meeting,
    Research,
    Sprint,
}

#[derive(Subcommand, Debug)]
pub enum PrintEntity {
    /// Generate PDF from meeting notes
    Meeting {
        /// Meeting ID (e.g., MTG-001)
        id: String,
        /// Output PDF path (default: {id}.pdf)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Generate PDF from research final report
    Research {
        /// Research ID (e.g., RES-001)
        id: String,
        /// Output PDF path (default: {id}-final-report.pdf)
        #[arg(short, long)]
        output: Option<String>,
        /// Specific file from final/ directory
        #[arg(long)]
        file: Option<String>,
    },
    /// Generate PDF from any markdown file
    File {
        /// Path to markdown file
        path: String,
        /// Output PDF path (default: <filename>.pdf)
        #[arg(short, long)]
        output: Option<String>,
        /// Cover page template
        #[arg(short, long, default_value = "standard")]
        template: PrintTemplate,
        /// Override document title (default: auto-detect from first H1 or filename)
        #[arg(long)]
        title: Option<String>,
    },
}
