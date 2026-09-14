mod cli;
mod mcp;

#[tokio::main]
async fn main() {
    if let Err(error) = cli::run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
