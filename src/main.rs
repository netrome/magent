use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = magent::Cli::parse();
    if let Err(e) = magent::run(cli).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
