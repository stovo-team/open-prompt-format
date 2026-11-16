use std::fs;
use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use walkdir::{DirEntry, WalkDir};

const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".venv",
    "venv",
    "env",
    "node_modules",
    "vendor",
    "dist",
    "build",
    "target",
    "__pycache__",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
];
const MAX_FILE_SIZE: u64 = 1_000_000;

#[derive(Debug, Serialize)]
pub struct PromptFinding {
    pub path: String,
    pub source_kind: &'static str,
    pub confidence: &'static str,
    pub reason: String,
}

pub fn scan(
    root: &Path,
    include: &[String],
    exclude: &[String],
) -> Result<Vec<PromptFinding>, String> {
    let base = root
        .canonicalize()
        .map_err(|error| format!("cannot resolve scan root {}: {error}", root.display()))?;
    if !base.is_dir() {
        return Err(format!("scan root is not a directory: {}", base.display()));
    }

    let include = build_globs(include)?;
    let exclude = build_globs(exclude)?;
    let mut findings = Vec::new();
    let walker = WalkDir::new(&base)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !excluded_directory(entry));

    for entry in walker {
        let entry = entry.map_err(|error| {
            format!(
                "cannot scan {}: {error}",
                error.path().unwrap_or(&base).display()
            )
        })?;
        if entry.path() == base {
            continue;
        }
        let relative = relative_path(&base, entry.path())?;
        if entry.file_type().is_symlink() {
            if fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir())
                && !resolves_within(&base, entry.path())
            {
                findings.push(PromptFinding {
                    path: relative,
                    source_kind: "symlink",
                    confidence: "low",
                    reason: "directory symlink escapes scan root and was not followed".into(),
                });
            }
            continue;
        }
        if !entry.file_type().is_file() || !matches_globs(&relative, &include, &exclude) {
            continue;
        }
        if !is_prompt_candidate(entry.path()) {
            continue;
        }
        match inspect_file(entry.path(), relative.clone()) {
            Ok(Some(finding)) => findings.push(finding),
            Ok(None) => {}
            Err(()) => findings.push(PromptFinding {
                path: relative,
                source_kind: "unreadable",
                confidence: "low",
                reason: "file could not be safely inspected as UTF-8 text".into(),
            }),
        }
    }
    findings.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(findings)
}

fn build_globs(patterns: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern)
            .map_err(|error| format!("invalid scan glob {pattern:?}: {error}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|error| format!("cannot compile scan globs: {error}"))
}

fn excluded_directory(entry: &DirEntry) -> bool {
    entry.depth() > 0
        && entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| EXCLUDED_DIRECTORIES.contains(&name))
}

fn relative_path(base: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(base)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .map_err(|error| {
            format!(
                "cannot make {} relative to scan root: {error}",
                path.display()
            )
        })
}

fn resolves_within(base: &Path, path: &Path) -> bool {
    path.canonicalize()
        .is_ok_and(|resolved| resolved.starts_with(base))
}

fn matches_globs(path: &str, include: &GlobSet, exclude: &GlobSet) -> bool {
    (include.is_empty() || include.is_match(path)) && !exclude.is_match(path)
}

fn is_prompt_candidate(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "md" | "j2" | "txt" | "json" | "yaml" | "yml" | "py"
    )
}

fn inspect_file(path: &Path, relative: String) -> Result<Option<PromptFinding>, ()> {
    let metadata = fs::metadata(path).map_err(|_| ())?;
    if metadata.len() > MAX_FILE_SIZE {
        return Ok(Some(PromptFinding {
            path: relative,
            source_kind: "oversized",
            confidence: "low",
            reason: "file exceeds the 1 MB inspection limit".into(),
        }));
    }
    let source = fs::read_to_string(path).map_err(|_| ())?;
    let Some((source_kind, confidence, reason)) = classify(path, &source) else {
        return Ok(None);
    };
    Ok(Some(PromptFinding {
        path: relative,
        source_kind,
        confidence,
        reason,
    }))
}

fn classify(path: &Path, source: &str) -> Option<(&'static str, &'static str, String)> {
    let suffix = path.extension()?.to_str()?.to_ascii_lowercase();
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    match suffix.as_str() {
        "md" => {
            if (source.starts_with("---\n") || source.starts_with("---\r"))
                && Regex::new(r"(?m)^format:\s*opf/0\.[123]\s*$")
                    .ok()?
                    .is_match(source)
            {
                return Some(("opf", "high", "OPF format frontmatter".into()));
            }
            if role_heading().is_match(source) {
                return Some((
                    "markdown.chat",
                    "high",
                    "contains explicit role message headings".into(),
                ));
            }
            if has_prompt_name(&name) {
                return Some(("markdown.prompt", "medium", "prompt-like filename".into()));
            }
        }
        "j2" => return Some(("jinja2", "high", "Jinja template file".into())),
        "txt" if has_prompt_name(&name) => {
            return Some(("text.prompt", "medium", "prompt-like filename".into()));
        }
        "yaml" | "yml" => {
            let lower = source.to_ascii_lowercase();
            if name == "opf.yaml" && lower.contains("schema: opf-registry/1") {
                return Some((
                    "opf.registry",
                    "high",
                    "OPF local registry configuration".into(),
                ));
            }
            if matches!(name.as_str(), "promptfooconfig.yaml" | "promptfoo.yaml")
                || (lower.contains("providers:") && lower.contains("prompts:"))
            {
                return Some((
                    "promptfoo.yaml",
                    "high",
                    "Promptfoo evaluation configuration".into(),
                ));
            }
            if [
                "system_template:",
                "prompt_template:",
                "custom_prompts.json",
            ]
            .iter()
            .any(|field| lower.contains(field))
            {
                return Some((
                    "crewai.yaml",
                    "high",
                    "CrewAI prompt configuration field".into(),
                ));
            }
            if lower.contains("_type: prompt")
                || (lower.contains("input_variables:") && lower.contains("template:"))
            {
                return Some((
                    "langchain.yaml",
                    "high",
                    "LangChain serialized prompt".into(),
                ));
            }
            if role_heading().is_match(source) || source.contains("{{") {
                return Some((
                    "yaml.prompt",
                    "low",
                    "contains prompt-like template syntax".into(),
                ));
            }
        }
        "json" => return classify_json(source),
        "py" => return classify_python(source),
        _ => {}
    }
    None
}

fn classify_json(source: &str) -> Option<(&'static str, &'static str, String)> {
    let value: Value = serde_json::from_str(source).ok()?;
    let object = value.as_object();
    if object.is_some_and(|object| object.get("slices").is_some_and(Value::is_object)) {
        return Some((
            "crewai.json",
            "high",
            "contains custom prompt slices".into(),
        ));
    }
    if object.is_some_and(|object| {
        object.contains_key("prompt_id")
            || (object.contains_key("prompt")
                && ["version", "variables", "tools", "text_format"]
                    .iter()
                    .any(|key| object.contains_key(*key)))
    }) {
        return Some((
            "openai.prompt.json",
            "high",
            "resembles an exported vendor prompt object".into(),
        ));
    }
    if value.as_array().is_some_and(|items| {
        !items.is_empty()
            && items.iter().all(|item| {
                item.get("role")
                    .and_then(Value::as_str)
                    .is_some_and(is_role)
                    && item
                        .get("content")
                        .is_some_and(|content| content.is_string() || content.is_array())
            })
    }) {
        return Some((
            "chat-messages.json",
            "high",
            "ordered role/content message array".into(),
        ));
    }
    if object.is_some_and(|object| {
        object
            .get("_type")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "prompt" | "chat"))
    }) {
        return Some((
            "langchain.json",
            "high",
            "serialized LangChain prompt".into(),
        ));
    }
    None
}

fn classify_python(source: &str) -> Option<(&'static str, &'static str, String)> {
    for (identifier, kind) in [
        ("PromptTemplate", "langchain.python"),
        ("ChatPromptTemplate", "langchain.python"),
        ("SystemMessagePromptTemplate", "langchain.python"),
    ] {
        if Regex::new(&format!(r"\b{}\s*\(", identifier))
            .ok()?
            .is_match(source)
        {
            return Some((kind, "high", format!("uses {identifier}")));
        }
    }
    if Regex::new(r"\bAssistantAgent\s*\([^)]*\b(system_message|system_prompt)\s*=")
        .ok()?
        .is_match(source)
    {
        return Some((
            "autogen.python",
            "high",
            "configures an agent system message".into(),
        ));
    }
    if Regex::new(r"\bAgent\s*\([^)]*\b(system_template|prompt_template)\s*=")
        .ok()?
        .is_match(source)
    {
        return Some((
            "crewai.python",
            "high",
            "configures a CrewAI prompt template".into(),
        ));
    }
    if Regex::new(r"\b(system_message|system_template|prompt_template)\s*=")
        .ok()?
        .is_match(source)
    {
        return Some((
            "python.prompt",
            "medium",
            "contains a framework prompt field".into(),
        ));
    }
    if Regex::new(r"(?i)\b(system prompt|prompt template|you are an? (helpful|expert|assistant))\b")
        .ok()?
        .is_match(source)
    {
        return Some((
            "python.prompt",
            "low",
            "prompt-like text in Python source".into(),
        ));
    }
    None
}

fn role_heading() -> &'static Regex {
    static ROLE_HEADING: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    ROLE_HEADING.get_or_init(|| {
        Regex::new(r"(?m)^## (?:system|developer|user|assistant)\s*$")
            .expect("role heading expression is valid")
    })
}

fn has_prompt_name(name: &str) -> bool {
    ["prompt", "system", "instruction"]
        .iter()
        .any(|word| name.contains(word))
}

fn is_role(role: &str) -> bool {
    matches!(role, "system" | "developer" | "user" | "assistant")
}
