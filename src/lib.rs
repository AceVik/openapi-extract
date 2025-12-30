#![allow(clippy::collapsible_if)]
pub mod config;
pub mod error;
pub mod generics;
pub mod index;
pub mod merger;
pub mod preprocessor;
pub mod scanner;
pub mod visitor;

use config::Config;
use error::Result;
use std::path::PathBuf;

/// Main entry point for generating OpenAPI definitions.
#[derive(Default)]
pub struct Generator {
    inputs: Vec<PathBuf>,
    includes: Vec<PathBuf>,
    output_path: Option<PathBuf>,
}

impl Generator {
    /// Creates a new Generator instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures the generator from a Config object.
    pub fn with_config(mut self, config: Config) -> Self {
        if let Some(inputs) = config.input {
            self.inputs.extend(inputs);
        }
        if let Some(includes) = config.include {
            self.includes.extend(includes);
        }
        if let Some(output) = config.output {
            self.output_path = Some(output);
        }
        self
    }

    /// Adds an input directory to scan.
    pub fn input<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.inputs.push(path.into());
        self
    }

    /// Adds a specific file to include.
    pub fn include<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.includes.push(path.into());
        self
    }

    /// Sets the output file path.
    pub fn output<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.output_path = Some(path.into());
        self
    }

    /// Executes the generation process.
    pub fn generate(self) -> Result<()> {
        let output = self.output_path.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Output path is required")
        })?;

        // 1. Scan and Extract
        log::info!(
            "Scanning directories: {:?} and includes: {:?}",
            self.inputs,
            self.includes
        );
        let snippets = scanner::scan_directories(&self.inputs, &self.includes)?;

        // 2. Merge
        log::info!("Merging {} snippets", snippets.len());
        let merged_value = merger::merge_openapi(snippets)?;

        // 3. Write Output
        // Ensure parent directory exists
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::File::create(&output)?;
        let extension = output
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("yaml");

        match extension {
            "json" => {
                serde_json::to_writer_pretty(file, &merged_value)?;
            }
            "yaml" | "yml" => {
                serde_yaml::to_writer(file, &merged_value)?;
            }
            _ => {
                serde_yaml::to_writer(file, &merged_value)?;
            }
        }

        log::info!("Written output to {:?}", output);

        Ok(())
    }
}
