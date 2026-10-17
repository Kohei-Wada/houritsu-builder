use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use houritsu_builder::api::Client;
use houritsu_builder::build::{self, build};
use houritsu_builder::cache::Cache;
use houritsu_builder::fetch::{self, fetch};

#[derive(Parser)]
#[command(name = "houritsu-builder", about = "Builds the houritsu repository: Japanese laws as git history from the e-Gov law API")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

fn default_cache() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/houritsu")
}

#[derive(Subcommand)]
enum Cmd {
    /// Crawl e-Gov law API v2 into the cache
    Fetch {
        #[arg(long, default_value_os_t = default_cache())]
        cache: PathBuf,
        /// Only this e-Gov law_type, e.g. Act
        #[arg(long)]
        law_type: Option<String>,
        /// Only these law IDs (repeatable)
        #[arg(long = "law-id")]
        law_ids: Vec<String>,
        /// Stop after this many laws
        #[arg(long)]
        limit: Option<usize>,
        /// Seconds between requests of one worker
        #[arg(long, default_value_t = 0.1)]
        delay: f64,
        /// Laws fetched at the same time
        #[arg(long, default_value_t = 2)]
        jobs: usize,
        /// Refetch every revision list, not only those that may have changed
        #[arg(long)]
        full: bool,
    },
    /// Build the houritsu repository from the cache
    Build {
        #[arg(long, default_value_os_t = default_cache())]
        cache: PathBuf,
        /// Directory for the new repository
        #[arg(long)]
        out: PathBuf,
        /// Only this e-Gov law_type, e.g. Act
        #[arg(long)]
        law_type: Option<String>,
        /// Skip rumdl
        #[arg(long)]
        no_lint: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Cmd::Fetch { cache, law_type, law_ids, limit, delay, jobs, full } => {
            let options = fetch::Options { law_type: law_type.as_deref(), law_ids, limit, jobs, full };
            let stats = fetch(&Client::new(delay), &Cache::new(cache), &options, &|line| println!("{line}"))?;
            println!("{}", serde_json::to_string(&stats)?);
        }
        Cmd::Build { cache, out, law_type, no_lint } => {
            let report = build(&Cache::new(cache), &out, &build::Options { law_type, lint: !no_lint, ..Default::default() })?;
            println!("{}", report.to_json(false));
            println!("unknown elements: {}", serde_json::to_string(&report.to_json(true)["unknown"])?);
        }
    }
    Ok(())
}
