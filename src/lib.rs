pub mod watcher;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "magent", about = "A markdown-native AI agent daemon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Watch a directory for @magent directives
    Watch {
        /// Directory to watch for markdown files
        directory: PathBuf,

        /// LLM API base URL
        #[arg(long, default_value = "http://localhost:11434/v1")]
        api_url: String,

        /// Model name
        #[arg(long, default_value = "llama3")]
        model: String,
    },
}

pub async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Watch { directory, .. } => {
            if !directory.exists() {
                return Err(format!("{} does not exist", directory.display()).into());
            }
            if !directory.is_dir() {
                return Err(format!("{} is not a directory", directory.display()).into());
            }

            let (tx, rx) = mpsc::channel(100);
            let _watcher = watcher::start(&directory, tx)?;

            println!("Watching {}...", directory.display());

            process_events(rx).await;

            Ok(())
        }
    }
}

async fn process_events(mut rx: mpsc::Receiver<PathBuf>) {
    loop {
        tokio::select! {
            Some(path) = rx.recv() => {
                println!("Changed: {}", path.display());
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down.");
                break;
            }
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run__should_fail_when_directory_does_not_exist() {
        // Given
        let cli = Cli {
            command: Command::Watch {
                directory: PathBuf::from("/nonexistent/path"),
                api_url: "http://localhost:11434/v1".to_string(),
                model: "llama3".to_string(),
            },
        };

        // When
        let result = run(cli).await;

        // Then
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("does not exist"),
            "error should mention directory does not exist"
        );
    }

    #[tokio::test]
    async fn run__should_fail_when_path_is_a_file() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("not_a_dir.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let cli = Cli {
            command: Command::Watch {
                directory: file_path,
                api_url: "http://localhost:11434/v1".to_string(),
                model: "llama3".to_string(),
            },
        };

        // When
        let result = run(cli).await;

        // Then
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("not a directory"),
            "error should mention path is not a directory"
        );
    }
}
