use clap::Parser;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("magent=info".parse().unwrap()),
        )
        .init();

    let cli = magent::Cli::parse();
    if let Err(e) = magent::run(cli).await {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}
