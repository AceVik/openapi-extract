use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Parser, Default, Clone)]
#[serde(default)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Input directories to scan for Rust files and OpenAPI fragments
    #[arg(short = 'i', long = "input")]
    pub input: Option<Vec<PathBuf>>,

    /// Specific files to include (e.g., .json, .yaml)
    #[arg(long = "include")]
    pub include: Option<Vec<PathBuf>>,

    /// Output file for the generated OpenAPI definition (defaults to openapi.yaml)
    #[arg(short = 'o', long = "output")]
    pub output: Option<PathBuf>,

    /// Path to a configuration file (toml)
    #[arg(long = "config")]
    #[serde(skip)]
    pub config_file: Option<PathBuf>,
}

#[derive(Deserialize)]
struct CargoConfig {
    package: Option<CargoPackage>,
}

#[derive(Deserialize)]
struct CargoPackage {
    metadata: Option<CargoMetadata>,
}

#[derive(Deserialize)]
struct CargoMetadata {
    #[serde(rename = "oas-forge")]
    oas_forge: Option<Config>,
}

impl Config {
    /// Load configuration with priority:
    /// 1. CLI Arguments (Highest)
    /// 2. --config file
    /// 3. openapi.toml
    /// 4. Cargo.toml [package.metadata.oas-forge]
    pub fn load() -> Self {
        let cli_args = Config::parse();

        // Start with default empty config
        let mut final_config = Config::default();

        // 4. Try loading Cargo.toml
        if let Ok(cargo_conf) = load_cargo_toml() {
            final_config.merge(cargo_conf);
        }

        // 3. Try loading openapi.toml
        if let Ok(toml_conf) = load_toml_file("openapi.toml") {
            final_config.merge(toml_conf);
        }

        // 2. Try loading explicit config file
        if let Some(path) = &cli_args.config_file {
            if let Ok(file_conf) = load_toml_file(path) {
                final_config.merge(file_conf);
            }
        }

        // 1. Merge CLI args (taking precedence)
        final_config.merge(cli_args);

        final_config
    }

    fn merge(&mut self, other: Config) {
        if let Some(input) = other.input {
            self.input = Some(input);
        }
        if let Some(include) = other.include {
            self.include = Some(include);
        }
        if let Some(output) = other.output {
            self.output = Some(output);
        }
    }
}

fn load_cargo_toml() -> Result<Config, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string("Cargo.toml")?;
    let config: CargoConfig = toml::from_str(&content)?;
    Ok(config
        .package
        .and_then(|p| p.metadata)
        .and_then(|m| m.oas_forge)
        .unwrap_or_default())
}

fn load_toml_file<P: AsRef<std::path::Path>>(
    path: P,
) -> Result<Config, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
