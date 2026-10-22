use std::{
    error::Error,
    time::{Duration, Instant},
};

use clap::{Args, Parser, Subcommand};
use graphmem::{
    EntityReference, GraphDirection, Memory, Scope, SearchResult,
    application::{
        GraphDetails, GraphRequest, HealthOptions, HealthReport, MemoryDetails, MemoryService,
        RememberRequest,
    },
    infrastructure::config::{ConfigOverrides, RetrievalOverrides},
};

#[derive(Parser)]
#[command(name = "gmem")]
struct Cli {
    /// Texts per embedding model call. Used when
    /// GRAPHMEM_EMBEDDING_BATCH_SIZE is unset; overrides `[embedding] batch_size`
    /// (default: 1 on CPU, 16 on CUDA).
    #[arg(
        long,
        global = true,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    embedding_batch_size: Option<usize>,
    /// Memories kept as PageRank seeds. Used when GRAPHMEM_RETRIEVAL_SEED_TOP_K
    /// is unset; overrides `[retrieval] seed_top_k`.
    #[arg(
        long,
        global = true,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    retrieval_seed_top_k: Option<usize>,
    /// Softmax temperature over the top-k seeds. Used when
    /// GRAPHMEM_RETRIEVAL_SEED_TEMPERATURE is unset; overrides
    /// `[retrieval] seed_temperature`.
    #[arg(long, global = true)]
    retrieval_seed_temperature: Option<f64>,
    /// Share of seed mass kept on memories vs. the graph. Used when
    /// GRAPHMEM_RETRIEVAL_MEMORY_SEED_WEIGHT is unset; overrides
    /// `[retrieval] memory_seed_weight`.
    #[arg(long, global = true)]
    retrieval_memory_seed_weight: Option<f64>,
    /// Boost for entities named in the query. Used when
    /// GRAPHMEM_RETRIEVAL_ENTITY_ANCHOR_WEIGHT is unset; overrides
    /// `[retrieval] entity_anchor_weight`.
    #[arg(long, global = true)]
    retrieval_entity_anchor_weight: Option<f64>,
    /// Personalized PageRank damping. Used when GRAPHMEM_RETRIEVAL_DAMPING is
    /// unset; overrides `[retrieval] damping`.
    #[arg(long, global = true)]
    retrieval_damping: Option<f64>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Remember(RememberArgs),
    List(ListArgs),
    Show(ShowArgs),
    Search(SearchArgs),
    /// Count tokens with the active embedding model's tokenizer (local model files only).
    Tokens(TokensArgs),
    Graph(GraphArgs),
    Forget {
        id: i64,
    },
    Flush(FlushArgs),
    Scopes,
    Reembed,
    Doctor,
    /// Report how much of the memory store is used and how much has gone stale.
    Health(HealthArgs),
    Migrate,
    Version,
    Mcp,
    Tui,
    /// Outline and find symbols in the current Git checkout.
    #[cfg(feature = "code")]
    #[command(subcommand)]
    Code(CodeCommand),
}

#[cfg(feature = "code")]
#[derive(Subcommand)]
enum CodeCommand {
    /// Index the Git checkout containing PATH (default: the current directory).
    Index { path: Option<std::path::PathBuf> },
    /// List the symbols in FILE, re-indexing it first if it changed.
    Outline {
        file: String,
        #[arg(long)]
        depth: Option<usize>,
    },
    /// List the imports declared in FILE, re-indexing it first if it changed.
    Imports { file: String },
    /// List the symbols changed between the merge base of BASE and HEAD, and HEAD.
    Diff {
        base: String,
        #[arg(default_value = "HEAD")]
        head: String,
    },
    /// Find indexed symbols by name, exact matches first.
    Find {
        query: String,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Args)]
struct RememberArgs {
    content: String,
    #[arg(long = "type", default_value = "observation")]
    memory_type: String,
    #[arg(long, default_value_t = 0.0)]
    importance: f64,
    #[arg(long = "scope")]
    scopes: Vec<String>,
}

#[derive(Args)]
struct ListArgs {
    #[arg(long = "scope")]
    scopes: Vec<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
    /// Memories to skip, for paging.
    #[arg(long, default_value_t = 0)]
    offset: usize,
}

#[derive(Args)]
struct TokensArgs {
    /// Text to count; read from stdin when omitted.
    #[arg(conflicts_with_all = ["id", "over_limit"])]
    text: Option<String>,
    /// Count a stored memory's content.
    #[arg(long, conflicts_with = "over_limit")]
    id: Option<i64>,
    /// List the memories whose content exceeds the model's token limit.
    #[arg(long)]
    over_limit: bool,
    /// Scopes for --id and --over-limit; global is always included.
    #[arg(long = "scope")]
    scopes: Vec<String>,
}

#[derive(Args)]
struct ShowArgs {
    id: i64,
    #[arg(long = "scope")]
    scopes: Vec<String>,
}

#[derive(Args)]
struct SearchArgs {
    query: String,
    #[arg(long = "scope")]
    scopes: Vec<String>,
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

#[derive(Args)]
struct GraphArgs {
    kind: String,
    name: String,
    #[arg(long, default_value = "both")]
    direction: String,
    #[arg(long, default_value_t = 1)]
    max_depth: usize,
    #[arg(long, default_value_t = 25)]
    limit: usize,
}

#[derive(Args)]
struct FlushArgs {
    #[arg(long)]
    yes: bool,
}

#[derive(Args)]
struct HealthArgs {
    /// Days without access before a memory counts as idle.
    #[arg(long, default_value_t = 30)]
    idle_days: u32,
}

pub async fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let overrides = ConfigOverrides {
        embedding_batch_size: cli.embedding_batch_size,
        retrieval: RetrievalOverrides {
            seed_top_k: cli.retrieval_seed_top_k,
            seed_temperature: cli.retrieval_seed_temperature,
            memory_seed_weight: cli.retrieval_memory_seed_weight,
            entity_anchor_weight: cli.retrieval_entity_anchor_weight,
            damping: cli.retrieval_damping,
        },
    };
    match cli.command {
        Command::Remember(args) => {
            let mut service = MemoryService::open_default(overrides)?;
            let memory = service.remember(RememberRequest {
                content: args.content,
                memory_type: args.memory_type,
                importance: args.importance,
                scopes: args.scopes,
                entities: Vec::new(),
                relations: Vec::new(),
            })?;
            println!("remembered: {}", memory.id);
        }
        Command::List(args) => {
            let service = MemoryService::open_default(overrides)?;
            print_memories(service.list_page(&args.scopes, None, args.limit, args.offset)?);
        }
        Command::Show(args) => {
            let mut service = MemoryService::open_default(overrides)?;
            print_memory_details(service.inspect(args.id, Some(&args.scopes))?);
        }
        Command::Tokens(args) => {
            let service = MemoryService::open_default(overrides)?;
            let counter = service.token_counter()?;
            let limit = counter
                .limit()
                .map_or_else(|| "-".to_owned(), |limit| limit.to_string());
            if args.over_limit {
                for (id, tokens) in service.memories_over_token_limit(&args.scopes)? {
                    println!("{id}\t{tokens}\t{limit}");
                }
            } else if let Some(id) = args.id {
                let details = service.show(id, Some(&args.scopes))?;
                println!("{id}\t{}\t{limit}", counter.count(&details.memory.content)?);
            } else {
                let text = match args.text {
                    Some(text) => text,
                    None => std::io::read_to_string(std::io::stdin())?,
                };
                println!("{}\t{limit}", counter.count(&text)?);
            }
        }
        Command::Search(args) => {
            let mut service = MemoryService::open_default(overrides)?;
            let results = service.search_scopes(&args.query, &args.scopes, args.limit)?;
            print_search_results(results);
        }
        Command::Graph(args) => {
            if !(1..=3).contains(&args.max_depth) {
                return Err(std::io::Error::other("max_depth must be between 1 and 3").into());
            }
            if !(1..=100).contains(&args.limit) {
                return Err(std::io::Error::other("limit must be between 1 and 100").into());
            }
            let direction = match args.direction.as_str() {
                "incoming" => GraphDirection::Incoming,
                "outgoing" => GraphDirection::Outgoing,
                "both" => GraphDirection::Both,
                _ => {
                    return Err(std::io::Error::other(
                        "direction must be incoming, outgoing, or both",
                    )
                    .into());
                }
            };
            let service = MemoryService::open_default(overrides)?;
            let details = service.graph(GraphRequest {
                entity: EntityReference {
                    kind: args.kind,
                    name: args.name,
                },
                direction,
                max_depth: args.max_depth,
                limit: args.limit,
            })?;
            print_graph_details(details);
        }
        Command::Forget { id } => {
            let service = MemoryService::open_default(overrides)?;
            service.forget(id, None)?;
            println!("forgot: {id}");
        }
        Command::Flush(args) => {
            if !args.yes {
                return Err(std::io::Error::other("refusing to flush; rerun with --yes").into());
            }
            let mut service = MemoryService::open_default(overrides)?;
            service.flush()?;
            println!("flushed all memories and graph data");
        }
        Command::Scopes => {
            let service = MemoryService::open_default(overrides)?;
            print_scopes(service.scopes()?);
        }
        Command::Reembed => {
            let mut service = MemoryService::open_default(overrides)?;
            if service.embedding_config().enabled {
                eprintln!("loading embedding model (first use may download it)...");
            }
            let mut last_update = Instant::now();
            let stats = service.reembed_all(|kind, done, total| {
                if done == 0 || done == total || last_update.elapsed() >= Duration::from_secs(2) {
                    eprintln!("{kind}: {done}/{total} processed");
                    last_update = Instant::now();
                }
            })?;
            println!(
                "reembedded {} memories, {} entities, {} edges under the current embedding model",
                stats.memories, stats.entities, stats.edges
            );
            if !stats.failures.is_empty() {
                eprintln!("{} item(s) failed to reembed:", stats.failures.len());
                for failure in &stats.failures {
                    eprintln!("  {failure}");
                }
                return Err(format!("{} item(s) failed to reembed", stats.failures.len()).into());
            }
        }
        Command::Doctor => {
            let service = MemoryService::open_default(overrides)?;
            println!("database: {}", service.database_path().display());
            println!("status: healthy");
        }
        Command::Health(args) => {
            let service = MemoryService::open_default(overrides)?;
            print_health(&service.health(HealthOptions {
                idle_days: args.idle_days,
            })?);
        }
        Command::Migrate => {
            let service = MemoryService::open_default(overrides)?;
            println!("schema version: {}", service.schema_version()?);
        }
        Command::Version => println!("gmem {}", env!("CARGO_PKG_VERSION")),
        Command::Mcp => crate::mcp::run(overrides).await?,
        Command::Tui => crate::tui::run(overrides)?,
        #[cfg(feature = "code")]
        Command::Code(command) => run_code(command)?,
    }
    Ok(())
}

#[cfg(feature = "code")]
fn run_code(command: CodeCommand) -> Result<(), Box<dyn Error>> {
    let Some(mut service) = graphmem::application::code::CodeService::open_default()? else {
        return Err("code tools are disabled ([code] enabled = false or GRAPHMEM_CODE=off)".into());
    };
    let directory = std::env::current_dir()?;
    match command {
        CodeCommand::Index { path } => {
            let mut last_update = Instant::now();
            let report = service.index(&path.unwrap_or(directory), |done, total| {
                if done == total || last_update.elapsed() >= Duration::from_secs(2) {
                    eprintln!("indexed {done}/{total} files");
                    last_update = Instant::now();
                }
            })?;
            println!(
                "{}: {} indexed, {} unchanged, {} removed, {} skipped",
                report.root.display(),
                report.indexed,
                report.unchanged,
                report.removed,
                report.skipped
            );
            if report.truncated {
                eprintln!("warning: too many source files; the rest were not indexed");
            }
            for failure in &report.failed {
                eprintln!("failed: {failure}");
            }
        }
        CodeCommand::Outline { file, depth } => {
            let outline = service.outline(&directory, &file)?;
            print!("{}", outline.to_text(depth, 0, usize::MAX));
        }
        CodeCommand::Imports { file } => {
            let outline = service.outline(&directory, &file)?;
            print!("{}", outline.imports_text(0, usize::MAX));
        }
        CodeCommand::Diff { base, head } => {
            print!(
                "{}",
                graphmem::application::code::diff(&directory, &base, &head, usize::MAX)?.to_text()
            );
        }
        CodeCommand::Find { query, kind, limit } => {
            let found = service.find_symbol(&directory, &query, kind.as_deref(), limit)?;
            print!("{}", found.to_text());
        }
    }
    Ok(())
}

fn print_memories(memories: Vec<Memory>) {
    for memory in memories {
        println!("{}\t{}\t{}", memory.id, memory.memory_type, memory.content);
    }
}

fn print_search_results(results: Vec<SearchResult>) {
    for result in results {
        println!(
            "{}\t{}\t{}\t{}",
            result.score, result.memory.id, result.memory.memory_type, result.memory.content
        );
    }
}

fn print_memory_details(details: MemoryDetails) {
    let scopes = details
        .scopes
        .iter()
        .map(|scope| scope.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    println!("id: {}", details.memory.id);
    println!("type: {}", details.memory.memory_type);
    println!("importance: {}", details.memory.importance);
    println!("created_at: {}", details.memory.created_at);
    println!("updated_at: {}", details.memory.updated_at);
    println!("scopes: {scopes}");
    let entities = details
        .entities
        .iter()
        .map(|entity| format!("{}:{}", entity.kind, entity.name))
        .collect::<Vec<_>>()
        .join(", ");
    println!("entities: {entities}");
    for relation in &details.relations {
        println!(
            "relation: {}:{} -{}-> {}:{}",
            relation.source.kind,
            relation.source.name,
            relation.edge.relation,
            relation.target.kind,
            relation.target.name
        );
    }
    println!("content:\n{}", details.memory.content);
}

fn print_graph_details(details: GraphDetails) {
    println!(
        "{}\t{}\t{}",
        details.entity.kind, details.entity.name, details.entity.canonical_name
    );
    for path in details.paths {
        for (depth, hop) in path.hops.iter().enumerate() {
            let direction = match hop.direction {
                GraphDirection::Incoming => "incoming",
                GraphDirection::Outgoing => "outgoing",
                GraphDirection::Both => unreachable!(),
            };
            println!(
                "{}\t{}\t{}\t{}\t{}",
                depth + 1,
                direction,
                hop.edge.relation,
                hop.entity.kind,
                hop.entity.name
            );
        }
        println!();
    }
}

fn percent(part: usize, total: usize) -> String {
    if total == 0 {
        return "0%".to_owned();
    }
    format!("{:.0}%", part as f64 * 100.0 / total as f64)
}

fn print_health(report: &HealthReport) {
    let total = report.memories;
    println!("memories: {total}");
    println!(
        "never accessed: {} ({})",
        report.never_accessed,
        percent(report.never_accessed, total)
    );
    println!(
        "idle for {}+ days: {} ({})",
        report.idle_days,
        report.idle,
        percent(report.idle, total)
    );

    println!("\n{:<16}{:>7}{:>8}{:>6}", "type", "total", "never", "idle");
    for row in &report.by_type {
        println!(
            "{:<16}{:>7}{:>8}{:>6}",
            row.memory_type, row.total, row.never_accessed, row.idle
        );
    }

    println!();
    match report.recall_log_days {
        None => println!("recall log: empty (events are recorded from schema version 2 on)"),
        Some(days) => {
            println!("recall log: since {days} day(s) ago");
            println!("{:<8}{:<9}{:>8}{:>7}", "source", "kind", "events", "empty");
            for row in &report.recalls.rows {
                println!(
                    "{:<8}{:<9}{:>8}{:>7}",
                    row.source, row.kind, row.events, row.empty
                );
            }
            println!(
                "memories returned at least once: {} of {total}",
                report.recalls.memories_returned
            );
        }
    }

    println!(
        "\nstale paths: {} memories reference paths missing from their checkout ({} not checkable)",
        report.stale.len(),
        report.unchecked
    );
    for memory in &report.stale {
        println!(
            "  #{} {}\t{}",
            memory.id,
            memory.memory_type,
            memory.missing.join(", ")
        );
    }
}

fn print_scopes(scopes: Vec<Scope>) {
    for scope in scopes {
        println!("{}\t{}", scope.id, scope.name);
    }
}
