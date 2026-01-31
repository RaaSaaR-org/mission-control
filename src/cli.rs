use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "mc",
    about = "MissionControl CLI -- manage customers, projects, meetings, research, and tasks",
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
  mc index                           Rebuild JSON indexes"
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
  mc new project \"Data Pipeline\" --owner alice --status active
  mc new meeting \"Weekly Sync\" --date 2025-06-01 --time 14:00
  mc new research \"LLM Benchmarks\" --agents claude,gemini
  mc new task \"Fix login bug\" --project PROJ-001 --priority 1")]
    New {
        #[command(subcommand)]
        entity: NewEntity,
    },
    /// List entities
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list customers
  mc list customers --status active
  mc list projects --tag ml
  mc list meetings --status scheduled
  mc list tasks --status in-progress --project PROJ-001")]
    List {
        #[command(subcommand)]
        entity: ListEntity,
    },
    /// Show details for an entity by ID
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc show CUST-001
  mc show PROJ-001
  mc show MTG-001
  mc show RES-001
  mc show TASK-001")]
    Show {
        /// Entity ID (e.g., CUST-001, PROJ-001, MTG-001, RES-001, TASK-001)
        id: String,
    },
    /// Rebuild data/*.json index files
    Index,
    /// Export an entity to a zip archive
    Export {
        #[command(subcommand)]
        entity: ExportEntity,
    },
    /// Generate a branded PDF from a meeting or research entity
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc print meeting MTG-001                    Meeting notes to PDF
  mc print meeting MTG-001 -o meeting.pdf     Custom output path
  mc print research RES-001                   Research final report to PDF
  mc print research RES-001 --file report.md  Specific file from final/")]
    Print {
        #[command(subcommand)]
        entity: PrintEntity,
    },
    /// Validate repo structure and frontmatter
    Validate,
    /// Show a dashboard with counts and recent activity
    Status,
    /// Start a local web server to browse all MissionControl data
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc serve                Start on default port 5000
  mc serve --port 8080    Start on port 8080")]
    Serve {
        /// Port to listen on
        #[arg(long, default_value_t = 5000)]
        port: u16,
    },
    /// Start an MCP (Model Context Protocol) server over stdio
    Mcp,
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
        /// Owner
        #[arg(long)]
        owner: Option<String>,
        /// Status
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
        /// Owner
        #[arg(long)]
        owner: Option<String>,
        /// Status
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
    },
    /// Create a new research topic
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc new research \"LLM Benchmarks\"
  mc new research \"LLM Benchmarks\" --owner alice --agents claude,gemini
  mc new research \"LLM Benchmarks\" --tags \"ai,benchmarks\"")]
    Research {
        /// Research title
        title: String,
        /// Owner
        #[arg(long)]
        owner: Option<String>,
        /// Comma-separated agent names
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
    /// List tasks
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc list tasks
  mc list tasks --status in-progress --project PROJ-001
  mc list tasks --priority 1 --owner huhn511
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
        /// Optionally set sprint label
        #[arg(long)]
        sprint: Option<String>,
    },
    /// Show the next actionable task
    #[command(after_help = "\x1b[1mExamples:\x1b[0m
  mc task next
  mc task next --project PROJ-001")]
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
    Customer {
        /// Customer ID or slug (e.g., CUST-001 or acme-inc)
        id: String,
    },
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
}
