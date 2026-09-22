use std::error::Error;

use clap::{Args, Parser, Subcommand};
use graphmem::{
    EntityReference, GraphDirection, Memory, Scope, SearchResult,
    application::{GraphDetails, GraphRequest, MemoryDetails, MemoryService, RememberRequest},
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
    Show { id: i64 },
    Search(SearchArgs),
    Graph(GraphArgs),
    Forget { id: i64 },
    Flush(FlushArgs),
    Scopes,
    Reembed,
    Doctor,
    Migrate,
    Mcp,
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
    #[arg(long)]
    scope: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
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
            print_memories(service.list(args.scope.as_deref(), args.limit)?);
        }
        Command::Show { id } => {
            let service = MemoryService::open_default(overrides)?;
            print_memory_details(service.show(id, None)?);
        }
        Command::Search(args) => {
            let mut service = MemoryService::open_default(overrides)?;
            let results = if args.scopes.is_empty() {
                service.search(&args.query, None, args.limit)?
            } else {
                service.search_scopes(&args.query, &args.scopes, args.limit)?
            };
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
            let stats = service.reembed_all()?;
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
        Command::Migrate => {
            let service = MemoryService::open_default(overrides)?;
            println!("schema version: {}", service.schema_version()?);
        }
        Command::Mcp => crate::mcp::run(overrides).await?,
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

fn print_scopes(scopes: Vec<Scope>) {
    for scope in scopes {
        println!("{}\t{}", scope.id, scope.name);
    }
}
