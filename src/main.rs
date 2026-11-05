use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use houritsu_builder::api::{Client, DELAY};
use houritsu_builder::bills;
use houritsu_builder::build::{self, build};
use houritsu_builder::cache::Cache;
use houritsu_builder::fetch::{self, fetch};
use houritsu_builder::verify::{self, Verdict};

#[derive(Parser)]
#[command(
    name = "houritsu-builder",
    about = "Builds the houritsu repository: Japanese laws as git history from the e-Gov law API"
)]
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
        #[arg(long, default_value_t = DELAY)]
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
    /// Check a law file of a houritsu checkout against what e-Gov serves now
    Verify {
        /// A law file, or a law ID to look up in --repo
        target: String,
        /// The houritsu checkout to look a law ID up in
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Check this revision from the history of HEAD instead of the file as it is
        #[arg(long)]
        revision: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Cmd::Fetch {
            cache,
            law_type,
            law_ids,
            limit,
            delay,
            jobs,
            full,
        } => {
            let options = fetch::Options {
                law_type: law_type.as_deref(),
                law_ids,
                limit,
                jobs,
                full,
            };
            let cache = Cache::new(cache);
            let stats = fetch(&Client::new(delay), &cache, &options, &|line| {
                println!("{line}")
            })?;
            println!("{}", serde_json::to_string(&stats)?);
            if let Err(e) = bills::fetch(&cache) {
                eprintln!("bill list not refreshed, keeping the previous one: {e:#}");
            }
        }
        Cmd::Build {
            cache,
            out,
            law_type,
            no_lint,
        } => {
            let report = build(
                &Cache::new(cache),
                &out,
                &build::Options {
                    law_type,
                    lint: !no_lint,
                    ..Default::default()
                },
            )?;
            println!("{}", report.to_json(false));
            println!(
                "unknown elements: {}",
                serde_json::to_string(&report.to_json(true)["unknown"])?
            );
        }
        Cmd::Verify {
            target,
            repo,
            revision,
        } => {
            let result = verify::locate(&repo, &target, revision.as_deref())
                .and_then(|local| Ok((verify::check(&Client::new(DELAY), &local)?, local)));
            match result {
                Ok((Verdict::Match, local)) => println!("OK {}", local.path),
                Ok((Verdict::Mismatch { reason, diff }, local)) => {
                    println!("MISMATCH {}: {reason}", local.path);
                    if !diff.is_empty() {
                        println!("{diff}");
                    }
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("error: {e:#}");
                    std::process::exit(2);
                }
            }
        }
    }
    Ok(())
}
