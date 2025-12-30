#[cfg(feature = "cli")]
use colored::Colorize;
#[cfg(feature = "cli")]
use oas_forge::{Generator, config::Config};

#[cfg(feature = "cli")]
fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Load configuration (CLI + TOML + Cargo.toml)
    let config = Config::load();
    let output = config
        .output
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("openapi.yaml"));

    println!("{} Starting oas-forge...", "INFO:".blue().bold());

    // Set up Generator
    match Generator::new().with_config(config).generate() {
        Ok(_) => {
            println!(
                "{} Successfully generated OpenAPI definition at {:?}",
                "SUCCESS:".green().bold(),
                output
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("{} {}", "ERROR:".red().bold(), e);
            Err(anyhow::anyhow!(e))
        }
    }
}

#[cfg(not(feature = "cli"))]
fn main() {
    eprintln!("This binary requires the 'cli' feature to be enabled.");
    std::process::exit(1);
}
