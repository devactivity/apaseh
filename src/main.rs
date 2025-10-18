mod cache;
mod config;
mod file_server;
mod http;
mod master;
mod memory_pool;
mod proxy;
mod signal_handler;
mod worker;

use clap::Parser;
use log::{error, info, warn};
use std::{path::PathBuf, process};

use config::Config;
use master::Master;

#[derive(Parser)]
#[command(name = "apaseh")]
#[command(about = "An HTTP server written in Rust")]
struct Args {
    #[arg(help = "configuration file path")]
    config: Option<PathBuf>,

    #[arg(short, long)]
    port: Option<u16>,

    #[arg(short, long)]
    workers: Option<usize>,

    #[arg(long)]
    test_config: bool,

    #[arg(long)]
    verbose: bool,
}

fn main() {
    let args = Args::parse();

    if args.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::init();
    }

    let config_path = args.config.unwrap_or_else(|| PathBuf::from("server.conf"));
    let config = Config::load(&config_path).unwrap_or_else(|e| {
        if config_path.exists() {
            error!("failed to load config from {}: {e}", config_path.display());
            process::exit(1);
        } else {
            warn!("config file not found. using default configuration");
            Config::default()
        }
    });

    // override config with CLI args
    let mut config = config;
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(workers) = args.workers {
        config.worker_processes = workers;
    }

    if args.test_config {
        info!("configuration test successful");
        return;
    }

    info!("starting Apaseh server");
    info!(
        "configuration: port={}, workers={}, root={}",
        config.port, config.worker_processes, config.root
    );

    let master = Master::new(config);
    if let Err(e) = master.run() {
        error!("server error: {e}");
        process::exit(1);
    }
}
