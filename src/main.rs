mod cli;
mod mcp;

fn main() {
    let data_dir = graphmem::Database::default_path()
        .ok()
        .and_then(|path| path.parent().map(ToOwned::to_owned));
    let worker_threads = data_dir
        .as_deref()
        .and_then(|path| graphmem::config::runtime_config(path).ok())
        .map(|config| config.worker_threads)
        .unwrap_or(4);
    if let Err(error) = init_logging() {
        eprintln!("logging unavailable: {error}");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .expect("Tokio runtime builds");
    if let Err(error) = runtime.block_on(cli::run()) {
        tracing::error!(%error, "graphmem command failed");
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn init_logging() -> std::io::Result<()> {
    let path = graphmem::Database::default_path()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let data_dir = path.parent().expect("default database path has a parent");
    graphmem::infrastructure::logging::init(data_dir)?;
    tracing::info!(path = %graphmem::infrastructure::logging::log_path(data_dir).display(), "logging initialized");
    Ok(())
}
