mod discovery;
mod init;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{error::ErrorKind, ColorChoice, Parser, Subcommand};
use open_prompt_format::{load, load_collection, Prompt};
use serde_json::{json, Map, Value};

#[derive(Debug, Parser)]
#[command(
    name = "opf",
    about = "Open Prompt Format command-line tools",
    color = ColorChoice::Never
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Create a starter prompt, registry entry, and agent guidance.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long = "id", default_value = "example.prompt")]
        prompt_id: String,
        #[arg(long = "prompt", default_value = "prompts/example.prompt.md")]
        prompt_path: String,
        #[arg(long, value_parser = ["auto", "yes", "no"], default_value = "auto")]
        agents: String,
        #[arg(long)]
        github: bool,
    },
    /// Validate a prompt file.
    Validate {
        file: PathBuf,
        #[arg(long)]
        strict: bool,
    },
    /// Validate every prompt in a directory.
    ValidateCollection { directory: PathBuf },
    /// Render a prompt to role/content JSON.
    Render {
        file: PathBuf,
        #[arg(long = "input", value_name = "NAME=VALUE")]
        inputs: Vec<String>,
    },
    /// Report OPF extension compatibility.
    Compat {
        file: PathBuf,
        #[arg(long)]
        strict: bool,
        #[arg(long = "supports-extension", value_name = "ID=VERSION")]
        supported_extensions: Vec<String>,
        #[arg(long, value_parser = ["text", "json"], default_value = "text")]
        format: String,
    },
    /// Find likely prompt sources without changing files.
    Scan {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,
        #[arg(long, value_parser = ["text", "json"], default_value = "text")]
        format: String,
    },
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            if error.kind() == ErrorKind::DisplayHelp || error.kind() == ErrorKind::DisplayVersion {
                let _ = error.print();
                return ExitCode::SUCCESS;
            }
            let _ = error.print();
            return ExitCode::from(code as u8);
        }
    };

    match run(cli) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("opf: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    let exit_code = match cli.command {
        Commands::Init {
            path,
            prompt_id,
            prompt_path,
            agents,
            github,
        } => {
            let result = init::initialize(&path, &prompt_id, &prompt_path, agents != "no", github)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
            );
            ExitCode::SUCCESS
        }
        Commands::Validate { file, strict } => {
            let prompt = load(&file).map_err(|error| error.to_string())?;
            if strict {
                let report = compatibility_report(&prompt, true, &HashMap::new());
                if report["can_apply"] == false {
                    return Err(format!(
                        "strict compatibility failed; run 'opf compat {} --strict' for details",
                        file.display()
                    ));
                }
            }
            println!(
                "valid: {}@{} {}",
                prompt.metadata.id,
                prompt.metadata.version.as_deref().unwrap_or("draft"),
                prompt.source_digest
            );
            ExitCode::SUCCESS
        }
        Commands::ValidateCollection { directory } => {
            let prompts = load_collection(&directory).map_err(|error| error.to_string())?;
            println!("valid: {} prompt(s)", prompts.len());
            ExitCode::SUCCESS
        }
        Commands::Render { file, inputs } => {
            let prompt = load(&file).map_err(|error| error.to_string())?;
            let inputs = parse_inputs(inputs)?;
            let messages = prompt.render(&inputs).map_err(|error| error.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&messages).map_err(|error| error.to_string())?
            );
            ExitCode::SUCCESS
        }
        Commands::Compat {
            file,
            strict,
            supported_extensions,
            format,
        } => {
            let prompt = load(&file).map_err(|error| error.to_string())?;
            let supported = parse_supported_extensions(supported_extensions)?;
            let report = compatibility_report(&prompt, strict, &supported);
            if format == "json" {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
                );
            } else {
                print_compatibility_report(&report);
            }
            if report["can_apply"] == false {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            }
        }
        Commands::Scan {
            path,
            include,
            exclude,
            format,
        } => {
            let findings = discovery::scan(&path, &include, &exclude)?;
            if format == "json" {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&findings).map_err(|error| error.to_string())?
                );
            } else {
                for finding in &findings {
                    println!(
                        "{}  {}  {}  {}",
                        finding.path, finding.source_kind, finding.confidence, finding.reason
                    );
                }
                println!("{} candidate(s)", findings.len());
            }
            ExitCode::SUCCESS
        }
    };
    Ok(exit_code)
}

fn parse_supported_extensions(values: Vec<String>) -> Result<HashMap<String, Vec<String>>, String> {
    let mut supported = HashMap::new();
    for value in values {
        let Some((identifier, version)) = value.split_once('=') else {
            return Err("--supports-extension must use ID=VERSION".to_string());
        };
        if identifier.is_empty() || version.is_empty() {
            return Err("--supports-extension must use non-empty ID=VERSION".to_string());
        }
        supported
            .entry(identifier.to_string())
            .or_insert_with(Vec::new)
            .push(version.to_string());
    }
    Ok(supported)
}

fn compatibility_report(
    prompt: &Prompt,
    strict: bool,
    supported: &HashMap<String, Vec<String>>,
) -> Value {
    let mut findings = Vec::new();
    if let Some(extensions) = &prompt.metadata.extensions {
        for (identifier, record) in extensions {
            let version = record
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if supported
                .get(identifier)
                .is_some_and(|versions| versions.iter().any(|item| item == version))
            {
                continue;
            }
            let required = record
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let disposition = if required { "manual" } else { "preserved" };
            let message = if required {
                format!(
                    "required extension {identifier:?} version {version:?} is not supported by this consumer"
                )
            } else {
                format!(
                    "optional extension {identifier:?} version {version:?} is preserved but not interpreted"
                )
            };
            let recommendation = if required {
                "Use a consumer that supports this required extension before claiming full compatibility."
            } else {
                "Confirm that the target adapter understands this preserved extension before relying on its behavior."
            };
            let mut finding = Map::new();
            finding.insert("code".to_string(), json!("extension.unsupported"));
            finding.insert(
                "severity".to_string(),
                json!(if required && strict {
                    "error"
                } else {
                    "warning"
                }),
            );
            finding.insert("disposition".to_string(), json!(disposition));
            finding.insert("message".to_string(), json!(message));
            finding.insert("capability".to_string(), json!(identifier));
            finding.insert(
                "category".to_string(),
                json!(if required {
                    "unsupported"
                } else {
                    "preserved_resource"
                }),
            );
            finding.insert("recommendation".to_string(), json!(recommendation));
            if let Some(path) = &prompt.path {
                finding.insert(
                    "source_path".to_string(),
                    json!(path.to_string_lossy().to_string()),
                );
            }
            findings.push(Value::Object(finding));
        }
    }
    json!({
        "source_kind": prompt.metadata.format,
        "target_kind": "opf-core",
        "lossless": findings.iter().all(|finding| finding["disposition"] == "preserved"),
        "can_apply": findings.iter().all(|finding| finding["severity"] != "error"),
        "findings": findings
    })
}

fn print_compatibility_report(report: &Value) {
    println!(
        "{} -> {} (lossless={}, can_apply={})",
        report["source_kind"].as_str().unwrap_or_default(),
        report["target_kind"].as_str().unwrap_or_default(),
        report["lossless"],
        report["can_apply"]
    );
    if let Some(findings) = report["findings"].as_array() {
        for finding in findings {
            println!(
                "{} {} [{}]{}: {}",
                finding["severity"]
                    .as_str()
                    .unwrap_or_default()
                    .to_uppercase(),
                finding["code"].as_str().unwrap_or_default(),
                finding["disposition"].as_str().unwrap_or_default(),
                finding["category"]
                    .as_str()
                    .map(|category| format!(" [{category}]"))
                    .unwrap_or_default(),
                finding["message"].as_str().unwrap_or_default()
            );
            if let Some(recommendation) = finding["recommendation"].as_str() {
                println!("  Recommendation: {recommendation}");
            }
        }
        if findings.is_empty() {
            println!("all extensions are supported");
        }
    }
}

fn parse_inputs(values: Vec<String>) -> Result<HashMap<String, String>, String> {
    let mut inputs = HashMap::new();
    for value in values {
        let (name, value) = value
            .split_once('=')
            .ok_or_else(|| "--input must use NAME=VALUE".to_string())?;
        if inputs.contains_key(name) {
            return Err(format!("input {name:?} was supplied more than once"));
        }
        inputs.insert(name.to_string(), value.to_string());
    }
    Ok(inputs)
}
