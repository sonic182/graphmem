use std::error::Error;

use clap::{Args, Parser, Subcommand};
use graphmem::{
    Memory, Scope,
    application::{MemoryDetails, MemoryService, RememberRequest},
};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "graphmem")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Remember(RememberArgs),
    List(ListArgs),
    Show { id: Uuid },
    Search(SearchArgs),
    Forget { id: Uuid },
    Scopes,
    Doctor,
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
    #[arg(long)]
    scope: Option<String>,
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Remember(args) => {
            let mut service = MemoryService::open_default()?;
            let memory = service.remember(RememberRequest {
                content: args.content,
                memory_type: args.memory_type,
                importance: args.importance,
                scopes: args.scopes,
            })?;
            println!("remembered: {}", memory.id);
        }
        Command::List(args) => {
            let service = MemoryService::open_default()?;
            print_memories(service.list(args.scope.as_deref(), args.limit)?);
        }
        Command::Show { id } => {
            let service = MemoryService::open_default()?;
            print_memory_details(service.show(id)?);
        }
        Command::Search(args) => {
            let service = MemoryService::open_default()?;
            print_memories(service.search(&args.query, args.scope.as_deref(), args.limit)?);
        }
        Command::Forget { id } => {
            let service = MemoryService::open_default()?;
            service.forget(id)?;
            println!("forgot: {id}");
        }
        Command::Scopes => {
            let service = MemoryService::open_default()?;
            print_scopes(service.scopes()?);
        }
        Command::Doctor => {
            let service = MemoryService::open_default()?;
            println!("database: {}", service.database_path().display());
            println!("status: healthy");
        }
    }
    Ok(())
}

fn print_memories(memories: Vec<Memory>) {
    for memory in memories {
        println!("{}\t{}\t{}", memory.id, memory.memory_type, memory.content);
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

fn print_scopes(scopes: Vec<Scope>) {
    for scope in scopes {
        println!("{}\t{}", scope.id, scope.name);
    }
}
