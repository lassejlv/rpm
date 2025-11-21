mod installer;
mod manager;
mod registry;
mod types;

use clap::{Parser, Subcommand};
use manager::Manager;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "rpm")]
#[command(version = "0.1.0")]
#[command(about = "Simple package manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Force download and ignore cache
    #[arg(long, global = true)]
    force_no_cache: bool,

    /// Skip postinstall confirmation
    #[arg(long, global = true)]
    yes: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Install dependencies from package.json
    Install,
    /// Add one or more packages
    Add {
        /// Packages to add (e.g. react, react@18.0.0)
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Manage package cache
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
}

#[derive(Subcommand)]
enum CacheCommands {
    /// Clear the global package cache
    Clean,
    /// Show cache location and size
    Info,
}

#[tokio::main]
async fn main() {
    let start = Instant::now();
    let cli = Cli::parse();
    let manager = Manager::new(cli.force_no_cache, cli.yes);

    println!("rpm - simple package manager");
    
    let result = match cli.command {
        Some(Commands::Add { packages }) => manager.add_packages(packages).await,
        Some(Commands::Cache { command }) => manager.handle_cache_command(command).await,
        Some(Commands::Install) | None => manager.install().await,
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    println!("Done in {:.2}s", start.elapsed().as_secs_f64());
}
