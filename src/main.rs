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

    /// Skip postinstall scripts entirely
    #[arg(long, global = true)]
    ignore_scripts: bool,
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

        /// Add as dev dependency
        #[arg(
            short = 'D',
            short_alias = 'd',
            long = "save-dev",
            visible_alias = "dev"
        )]
        dev: bool,
    },
    /// Remove one or more packages
    #[command(visible_aliases = ["rm", "uninstall", "un"])]
    Remove {
        /// Packages to remove
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Run a script from package.json
    Run {
        /// Script name to run
        script: String,

        /// Arguments to pass to the script
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Execute a package binary (like npx)
    #[command(visible_alias = "exec")]
    X {
        /// Package to execute (e.g. prettier, eslint@8.0.0)
        package: String,

        /// Arguments to pass to the package
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
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
    let manager = Manager::new(cli.force_no_cache, cli.yes, cli.ignore_scripts);

    println!(
        "\x1b[1;36mrpm\x1b[0m \x1b[90mv{}\x1b[0m\n",
        env!("CARGO_PKG_VERSION")
    );

    let result = match cli.command {
        Some(Commands::Add { packages, dev }) => manager.add_packages(packages, dev).await,
        Some(Commands::Remove { packages }) => manager.remove_packages(packages).await,
        Some(Commands::Run { script, args }) => manager.run_script(&script, args).await,
        Some(Commands::X { package, args }) => manager.exec_package(&package, args).await,
        Some(Commands::Cache { command }) => manager.handle_cache_command(command).await,
        Some(Commands::Install) | None => manager.install().await,
    };

    if let Err(e) = result {
        eprintln!("\x1b[1;31merror:\x1b[0m {}", e);
        std::process::exit(1);
    }

    println!(
        "\n\x1b[1;32mDone\x1b[0m in {:.2}s",
        start.elapsed().as_secs_f64()
    );
}
