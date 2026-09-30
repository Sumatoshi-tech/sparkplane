use clap::Parser;
use sparkplane::spark::cli::{SparkCli, SparkError};
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[command(
    name = "sparkplane",
    version,
    about = "Independent DGX Spark inference appliance"
)]
struct Cli {
    #[command(flatten)]
    spark: SparkCli,
}

fn main() {
    if std::env::args_os().nth(1).is_some_and(|a| a == "__eco") {
        let code = match sparkplane::spark::eco::run(std::env::args_os().skip(2).collect()) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("eco: {error}");
                1
            }
        };
        std::process::exit(code);
    }
    if std::env::args_os().skip(1).eq(["--bridge-protocol"]) {
        println!("sparkplane.bridge/v1");
        return;
    }
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with(sparkplane_core::obs::TraceCtxLayer::new())
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();
    if let Err(error) = sparkplane::spark::cli::dispatch(Cli::parse().spark) {
        if let Some(error) = error.downcast_ref::<SparkError>() {
            error.exit();
        }
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
