//! Open Prompt Format (OPF) parser, validator, digest calculator, and renderer.

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use thiserror::Error;

/// Chat message roles supported by OPF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    Developer,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::Developer => "developer",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// A single provider-neutral rendered message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedMessage {
    pub role: Role,
    pub content: String,
}

/// A parsed, unrendered message template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageTemplate {
    pub role: Role,
    pub content: String,
}

/// Specification of a declared template input in OPF frontmatter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSpec {
    #[serde(rename = "type")]
    pub input_type: String,
    pub required: Option<bool>,
}

impl InputSpec {
    /// Returns whether this input is required (defaults to `true`).
    pub fn is_required(&self) -> bool {
        self.required.unwrap_or(true)
    }
}

/// Validated OPF frontmatter metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    pub format: String,
    pub id: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub inputs: BTreeMap<String, InputSpec>,
    pub extensions: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Error returned by OPF parsing, validation, loading, and rendering.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct OpfError {
    pub message: String,
}

impl OpfError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// A parsed and validated OPF prompt artifact.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub metadata: Metadata,
    pub messages: Vec<MessageTemplate>,
    pub source_digest: String,
    pub path: Option<PathBuf>,
}

impl Prompt {
    /// Returns the sorted list of declared input variable names.
    pub fn input_variables(&self) -> Vec<String> {
        self.metadata.inputs.keys().cloned().collect()
    }

    /// Renders all message templates using the provided string inputs.
    pub fn render(&self, inputs: &HashMap<String, String>) -> Result<Vec<RenderedMessage>, OpfError> {
        let mut unknown: Vec<&str> = inputs
            .keys()
            .filter(|k| !self.metadata.inputs.contains_key(k.as_str()))
            .map(|k| k.as_str())
            .collect();
        if !unknown.is_empty() {
            unknown.sort_unstable();
            return Err(OpfError::new(format!(
                "undeclared input(s): {}",
                unknown.join(", ")
            )));
        }

        for (name, spec) in &self.metadata.inputs {
            if spec.is_required() && !inputs.contains_key(name) {
                return Err(OpfError::new(format!("missing required input: {name}")));
            }
        }

        let mut rendered = Vec::with_capacity(self.messages.len());
        for msg in &self.messages {
            let content = render_template(&msg.content, inputs)?;
            rendered.push(RenderedMessage {
                role: msg.role,
                content,
            });
        }
        Ok(rendered)
    }
}

fn id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9]+(?:[._-][a-z0-9]+)*$").unwrap())
}

fn version_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*)?$").unwrap()
    })
}

fn input_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap())
}

fn extension_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z][a-z0-9]*(?:[.-][a-z0-9]+)+$").unwrap())
}

fn extension_version_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[!-~]{1,64}$").unwrap())
}

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^## (system|developer|user|assistant)$").unwrap())
}

fn escaped_heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\\(## (?:system|developer|user|assistant))$").unwrap())
}

fn fence_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ {0,3}(`{3,}|~{3,})(.*)$").unwrap())
}

fn variable_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}").unwrap())
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Parses and validates an OPF prompt from its text content.
pub fn parse(text: &str, path: Option<&Path>) -> Result<Prompt, OpfError> {
    let normalized_source = normalize_newlines(text);
    let (raw_meta, body) = split_frontmatter(&normalized_source)?;
    let metadata = validate_metadata(&raw_meta)?;
    let messages = parse_messages(body)?;
    validate_templates(&metadata, &messages)?;

    let mut hasher = Sha256::new();
    hasher.update(normalized_source.as_bytes());
    let digest_bytes = hasher.finalize();
    let mut hex_str = String::with_capacity(64);
    for b in digest_bytes {
        hex_str.push_str(&format!("{b:02x}"));
    }

    Ok(Prompt {
        metadata,
        messages,
        source_digest: format!("sha256:{hex_str}"),
        path: path.map(Path::to_path_buf),
    })
}

/// Reads and parses an OPF prompt file from disk.
pub fn load(path: impl AsRef<Path>) -> Result<Prompt, OpfError> {
    let path_ref = path.as_ref();
    let text = fs::read_to_string(path_ref)
        .map_err(|e| OpfError::new(format!("cannot read prompt file {}: {e}", path_ref.display())))?;
    parse(&text, Some(path_ref))
}

/// Loads all OPF prompts in `collection` and returns the one matching `prompt_id`.
pub fn load_by_id(prompt_id: &str, collection: impl AsRef<Path>) -> Result<Prompt, OpfError> {
    let collection_ref = collection.as_ref();
    let prompts = load_collection(collection_ref)?;
    prompts
        .into_iter()
        .find(|p| p.metadata.id == prompt_id)
        .ok_or_else(|| {
            OpfError::new(format!(
                "prompt id '{prompt_id}' not found in {}",
                collection_ref.display()
            ))
        })
}

/// Recursively scans `collection` for OPF `.md` files and rejects duplicate IDs.
pub fn load_collection(collection: impl AsRef<Path>) -> Result<Vec<Prompt>, OpfError> {
    let collection_ref = collection.as_ref();
    let mut files = Vec::new();
    collect_markdown_files(collection_ref, &mut files)?;
    files.sort();

    let mut prompts = Vec::new();
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for file_path in files {
        let source = fs::read_to_string(&file_path).map_err(|e| {
            OpfError::new(format!(
                "cannot read prompt file {}: {e}",
                file_path.display()
            ))
        })?;
        let trimmed = source.strip_prefix('\u{FEFF}').unwrap_or(&source);
        let first_line = trimmed.split(['\r', '\n']).next().unwrap_or("");
        if first_line != "---" {
            continue;
        }
        let prompt = parse(&source, Some(&file_path))?;
        if let Some(prior) = seen.get(&prompt.metadata.id) {
            return Err(OpfError::new(format!(
                "duplicate prompt id '{}': {}, {}",
                prompt.metadata.id,
                prior.display(),
                file_path.display()
            )));
        }
        seen.insert(prompt.metadata.id.clone(), file_path);
        prompts.push(prompt);
    }
    Ok(prompts)
}

fn collect_markdown_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), OpfError> {
    let entries = fs::read_dir(dir).map_err(|e| {
        OpfError::new(format!(
            "cannot scan prompt collection {}: {e}",
            dir.display()
        ))
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            OpfError::new(format!(
                "cannot scan prompt collection {}: {e}",
                dir.display()
            ))
        })?;
        paths.push(entry.path());
    }
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_markdown_files(&p, out)?;
        } else if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(p);
        }
    }
    Ok(())
}

fn split_frontmatter(normalized: &str) -> Result<(serde_yaml::Mapping, &str), OpfError> {
    let trimmed = normalized.strip_prefix('\u{FEFF}').unwrap_or(normalized);
    let lines: Vec<&str> = trimmed.split('\n').collect();
    if lines.is_empty() || lines[0] != "---" {
        return Err(OpfError::new(
            "frontmatter opening delimiter '---' must be the first line",
        ));
    }

    let closing = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| **line == "---")
        .map(|(i, _)| i)
        .ok_or_else(|| OpfError::new("frontmatter closing delimiter '---' was not found"))?;

    let yaml_source = lines[1..closing].join("\n");
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml_source)
        .map_err(|e| OpfError::new(format!("invalid YAML frontmatter: {e}")))?;

    let mapping = match value {
        serde_yaml::Value::Mapping(m) => m,
        _ => return Err(OpfError::new("frontmatter must be a YAML mapping")),
    };
    check_string_keys(&serde_yaml::Value::Mapping(mapping.clone()), "frontmatter")?;

    // Compute byte offset of the body after the closing `---` line
    let mut body_start = 0usize;
    for line in &lines[..=closing] {
        body_start += line.len() + 1; // +1 for '\n'
    }
    let body = if body_start <= trimmed.len() {
        &trimmed[body_start..]
    } else {
        ""
    };
    Ok((mapping, body))
}

fn check_string_keys(val: &serde_yaml::Value, path: &str) -> Result<(), OpfError> {
    match val {
        serde_yaml::Value::Mapping(map) => {
            for (k, v) in map {
                let key_str = k.as_str().ok_or_else(|| {
                    OpfError::new(format!("{path} must use string mapping keys"))
                })?;
                check_string_keys(v, &format!("{path}.{key_str}"))?;
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for (i, item) in seq.iter().enumerate() {
                check_string_keys(item, &format!("{path}[{i}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn yaml_to_json(val: &serde_yaml::Value, path: &str) -> Result<serde_json::Value, OpfError> {
    match val {
        serde_yaml::Value::Null => Ok(serde_json::Value::Null),
        serde_yaml::Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(serde_json::Value::Number(i.into()))
            } else if let Some(u) = n.as_u64() {
                Ok(serde_json::Value::Number(u.into()))
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| {
                        OpfError::new(format!("{path} must not contain a non-finite number"))
                    })
            } else {
                Err(OpfError::new(format!(
                    "{path} contains a value that is not JSON-compatible"
                )))
            }
        }
        serde_yaml::Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        serde_yaml::Value::Sequence(seq) => {
            let mut out = Vec::with_capacity(seq.len());
            for (i, item) in seq.iter().enumerate() {
                out.push(yaml_to_json(item, &format!("{path}[{i}]"))?);
            }
            Ok(serde_json::Value::Array(out))
        }
        serde_yaml::Value::Mapping(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let key = k.as_str().ok_or_else(|| {
                    OpfError::new(format!("{path} must use string mapping keys"))
                })?;
                out.insert(key.to_string(), yaml_to_json(v, &format!("{path}.{key}"))?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_yaml::Value::Tagged(_) => Err(OpfError::new(format!(
            "{path} contains a value that is not JSON-compatible"
        ))),
    }
}

fn get_field<'a>(
    map: &'a serde_yaml::Mapping,
    key: &str,
) -> Option<&'a serde_yaml::Value> {
    map.get(serde_yaml::Value::String(key.to_string()))
}

fn validate_metadata(raw: &serde_yaml::Mapping) -> Result<Metadata, OpfError> {
    const ALLOWED: [&str; 7] = [
        "format",
        "id",
        "version",
        "description",
        "tags",
        "inputs",
        "extensions",
    ];
    let mut unknown = Vec::new();
    for k in raw.keys() {
        let key = k.as_str().unwrap_or("");
        if !ALLOWED.contains(&key) {
            unknown.push(key);
        }
    }
    if !unknown.is_empty() {
        unknown.sort_unstable();
        return Err(OpfError::new(format!(
            "unknown frontmatter field(s): {}",
            unknown.join(", ")
        )));
    }

    let format = get_field(raw, "format")
        .ok_or_else(|| OpfError::new("missing required frontmatter field: format"))?
        .as_str()
        .ok_or_else(|| {
            OpfError::new("unsupported format; expected 'opf/0.1', 'opf/0.2', or 'opf/0.3'")
        })?
        .to_string();
    if format != "opf/0.1" && format != "opf/0.2" && format != "opf/0.3" {
        return Err(OpfError::new(format!(
            "unsupported format {format}; expected 'opf/0.1', 'opf/0.2', or 'opf/0.3'"
        )));
    }

    let id = get_field(raw, "id")
        .ok_or_else(|| OpfError::new("missing required frontmatter field: id"))?
        .as_str()
        .filter(|s| id_re().is_match(s))
        .ok_or_else(|| OpfError::new("id must match [a-z0-9]+(?:[._-][a-z0-9]+)*"))?
        .to_string();

    let raw_version = get_field(raw, "version");
    if format == "opf/0.1" && raw_version.is_none() {
        return Err(OpfError::new("missing required frontmatter field: version"));
    }
    if (format == "opf/0.2" || format == "opf/0.3") && raw_version.is_some() {
        return Err(OpfError::new(format!(
            "{format} keeps release versions in the registry; remove frontmatter version"
        )));
    }
    let version = if let Some(v) = raw_version {
        let s = v
            .as_str()
            .filter(|s| version_re().is_match(s))
            .ok_or_else(|| {
                OpfError::new("version must be SemVer MAJOR.MINOR.PATCH without build metadata")
            })?;
        Some(s.to_string())
    } else {
        None
    };

    let description = if let Some(d) = get_field(raw, "description") {
        let s = d
            .as_str()
            .ok_or_else(|| OpfError::new("description must be a string"))?;
        Some(s.to_string())
    } else {
        None
    };

    let tags = if let Some(t) = get_field(raw, "tags") {
        let seq = t
            .as_sequence()
            .ok_or_else(|| OpfError::new("tags must be a list of strings"))?;
        let mut out = Vec::with_capacity(seq.len());
        for item in seq {
            let s = item
                .as_str()
                .ok_or_else(|| OpfError::new("tags must be a list of strings"))?;
            out.push(s.to_string());
        }
        Some(out)
    } else {
        None
    };

    let extensions = if let Some(ext) = get_field(raw, "extensions") {
        let ext_map = ext
            .as_mapping()
            .ok_or_else(|| OpfError::new("extensions must be a mapping"))?;
        if format == "opf/0.3" {
            for (k, v) in ext_map {
                let identifier = k.as_str().unwrap_or("");
                if identifier.len() > 255 || !extension_id_re().is_match(identifier) {
                    return Err(OpfError::new(format!(
                        "invalid extension identifier {identifier:?}"
                    )));
                }
                let rec = v.as_mapping().ok_or_else(|| {
                    OpfError::new(format!("extension '{identifier}' must be a mapping"))
                })?;
                if rec.len() != 3 {
                    return Err(OpfError::new(format!(
                        "extension '{identifier}' requires exactly version, required, and data"
                    )));
                }
                let ver = get_field(rec, "version")
                    .and_then(|x| x.as_str())
                    .filter(|s| extension_version_re().is_match(s))
                    .ok_or_else(|| {
                        OpfError::new(format!(
                            "extension '{identifier}' version must be 1-64 printable non-space ASCII characters"
                        ))
                    })?;
                let _ = ver;
                if get_field(rec, "required").and_then(|x| x.as_bool()).is_none() {
                    return Err(OpfError::new(format!(
                        "extension '{identifier}' required must be a boolean"
                    )));
                }
                let data = get_field(rec, "data").ok_or_else(|| {
                    OpfError::new(format!(
                        "extension '{identifier}' requires exactly version, required, and data"
                    ))
                })?;
                if !data.is_mapping() {
                    return Err(OpfError::new(format!(
                        "extension '{identifier}' data must be a mapping"
                    )));
                }
                yaml_to_json(data, &format!("extension '{identifier}' data"))?;
            }
        }
        let json_val = yaml_to_json(ext, "extensions")?;
        match json_val {
            serde_json::Value::Object(m) => Some(m),
            _ => return Err(OpfError::new("extensions must be a mapping")),
        }
    } else {
        None
    };

    let mut inputs = BTreeMap::new();
    if let Some(raw_inputs) = get_field(raw, "inputs") {
        let in_map = raw_inputs
            .as_mapping()
            .ok_or_else(|| OpfError::new("inputs must be a mapping"))?;
        for (k, v) in in_map {
            let name = k.as_str().unwrap_or("");
            if !input_name_re().is_match(name) {
                return Err(OpfError::new(format!("invalid input name {name:?}")));
            }
            let spec_map = v
                .as_mapping()
                .ok_or_else(|| OpfError::new(format!("input '{name}' must be a mapping")))?;
            let mut extra = Vec::new();
            for sk in spec_map.keys() {
                let sk_str = sk.as_str().unwrap_or("");
                if sk_str != "type" && sk_str != "required" {
                    extra.push(sk_str);
                }
            }
            if !extra.is_empty() {
                extra.sort_unstable();
                return Err(OpfError::new(format!(
                    "unknown field(s) for input '{name}': {}",
                    extra.join(", ")
                )));
            }
            let input_type = get_field(spec_map, "type")
                .and_then(|x| x.as_str())
                .filter(|s| *s == "string")
                .ok_or_else(|| {
                    OpfError::new(format!("input '{name}' must declare type: string"))
                })?;
            let required = if let Some(req_val) = get_field(spec_map, "required") {
                Some(req_val.as_bool().ok_or_else(|| {
                    OpfError::new(format!("input '{name}' required must be a boolean"))
                })?)
            } else {
                None
            };
            inputs.insert(
                name.to_string(),
                InputSpec {
                    input_type: input_type.to_string(),
                    required,
                },
            );
        }
    }

    Ok(Metadata {
        format,
        id,
        version,
        description,
        tags,
        inputs,
        extensions,
    })
}

fn parse_messages(body: &str) -> Result<Vec<MessageTemplate>, OpfError> {
    let mut messages = Vec::new();
    let mut current_role: Option<Role> = None;
    let mut content: Vec<&str> = Vec::new();
    let mut fence_char: Option<u8> = None;
    let mut fence_length: usize = 0;

    for line in body.split('\n') {
        if let Some(caps) = fence_re().captures(line) {
            let marker = caps.get(1).unwrap().as_str();
            let ch = marker.as_bytes()[0];
            let trailing = caps.get(2).unwrap().as_str();
            if fence_char.is_none() {
                fence_char = Some(ch);
                fence_length = marker.len();
            } else if fence_char == Some(ch)
                && marker.len() >= fence_length
                && trailing.trim().is_empty()
            {
                fence_char = None;
                fence_length = 0;
            } else if current_role.is_some() {
                content.push(line);
                continue;
            }
            if current_role.is_some() {
                content.push(line);
            } else if !line.trim().is_empty() {
                return Err(OpfError::new(
                    "non-whitespace content appears before the first message",
                ));
            }
            continue;
        }

        if fence_char.is_none() {
            if let Some(caps) = heading_re().captures(line) {
                if let Some(role) = current_role {
                    messages.push(MessageTemplate {
                        role,
                        content: trim_blank_edges(&content),
                    });
                }
                current_role = Some(match caps.get(1).unwrap().as_str() {
                    "system" => Role::System,
                    "developer" => Role::Developer,
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    _ => unreachable!(),
                });
                content.clear();
                continue;
            }
        }

        if current_role.is_none() {
            if !line.trim().is_empty() {
                return Err(OpfError::new(
                    "non-whitespace content appears before the first message",
                ));
            }
            continue;
        }

        if fence_char.is_none() {
            if let Some(caps) = escaped_heading_re().captures(line) {
                content.push(caps.get(1).unwrap().as_str());
                continue;
            }
        }
        content.push(line);
    }

    if let Some(role) = current_role {
        messages.push(MessageTemplate {
            role,
            content: trim_blank_edges(&content),
        });
    }
    if messages.is_empty() {
        return Err(OpfError::new("prompt must contain at least one message"));
    }
    Ok(messages)
}

fn trim_blank_edges(lines: &[&str]) -> String {
    let mut start = 0;
    let mut end = lines.len();
    while start < end && lines[start].trim().is_empty() {
        start += 1;
    }
    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    lines[start..end].join("\n")
}

fn find_template_variables(template: &str) -> Result<BTreeSet<String>, OpfError> {
    const ESC_PLACEHOLDER: &str = "\x00OPF_ESC_BRACE\x00";
    let without_esc = template.replace(r"\{{", ESC_PLACEHOLDER);
    let stripped = variable_re().replace_all(&without_esc, "");
    if stripped.contains("{{") || stripped.contains("{%") {
        return Err(OpfError::new(
            "unsupported or malformed template expression; use only {{ input_name }}",
        ));
    }
    let vars = variable_re()
        .captures_iter(&without_esc)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .collect();
    Ok(vars)
}

fn validate_templates(meta: &Metadata, messages: &[MessageTemplate]) -> Result<(), OpfError> {
    for msg in messages {
        let vars = find_template_variables(&msg.content)?;
        let unknown: Vec<String> = vars
            .into_iter()
            .filter(|v| !meta.inputs.contains_key(v))
            .collect();
        if !unknown.is_empty() {
            return Err(OpfError::new(format!(
                "template uses undeclared input(s): {}",
                unknown.join(", ")
            )));
        }
    }
    Ok(())
}

fn render_template(template: &str, inputs: &HashMap<String, String>) -> Result<String, OpfError> {
    const ESC_PLACEHOLDER: &str = "\x00OPF_ESC_BRACE\x00";
    let protected = template.replace(r"\{{", ESC_PLACEHOLDER);
    let vars = find_template_variables(template)?;
    let missing: Vec<String> = vars
        .into_iter()
        .filter(|v| !inputs.contains_key(v))
        .collect();
    if !missing.is_empty() {
        return Err(OpfError::new(format!(
            "missing template input(s): {}",
            missing.join(", ")
        )));
    }
    let rendered = variable_re().replace_all(&protected, |caps: &regex::Captures| {
        let name = caps.get(1).unwrap().as_str();
        inputs[name].as_str()
    });
    Ok(rendered.replace(ESC_PLACEHOLDER, "{{"))
}
