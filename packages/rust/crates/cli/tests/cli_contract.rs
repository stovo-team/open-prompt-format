use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

#[test]
fn validate_prints_prompt_identity_and_digest() {
    let root = repo_root();
    let output = Command::new(env!("CARGO_BIN_EXE_opf"))
        .current_dir(&root)
        .args(["validate", "examples/support.reply.md"])
        .output()
        .expect("run Rust opf CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.starts_with("valid: support.reply@draft sha256:"));
    assert!(output.stderr.is_empty());
}

#[test]
fn validate_missing_file_uses_cli_error_exit_status() {
    let root = repo_root();
    let output = Command::new(env!("CARGO_BIN_EXE_opf"))
        .current_dir(&root)
        .args(["validate", "examples/missing.prompt.md"])
        .output()
        .expect("run Rust opf CLI");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("opf: cannot read prompt file"));
}

#[test]
fn render_substitutes_inputs_and_emits_role_content_json() {
    let root = repo_root();
    let output = Command::new(env!("CARGO_BIN_EXE_opf"))
        .current_dir(&root)
        .args([
            "render",
            "examples/support.reply.md",
            "--input",
            "customer_message=My order is late.",
        ])
        .output()
        .expect("run Rust opf CLI");

    assert!(output.status.success());
    let rendered: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("render output is JSON");
    assert_eq!(
        rendered,
        serde_json::json!([
            {
                "role": "system",
                "content": "You help customers with order questions. Be concise, kind, and factual.\nDo not invent order details. If key information is missing, ask one clear question."
            },
            {"role": "user", "content": "My order is late."}
        ])
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn render_rejects_missing_required_inputs() {
    let root = repo_root();
    let output = Command::new(env!("CARGO_BIN_EXE_opf"))
        .current_dir(&root)
        .args(["render", "examples/support.reply.md"])
        .output()
        .expect("run Rust opf CLI");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("missing required input: customer_message"));
}

#[test]
fn required_unknown_extension_is_reported_and_strict_validation_rejects_it() {
    let root = repo_root();
    let temporary = std::env::temp_dir().join(format!("opf-cli-extension-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).expect("create temporary prompt directory");
    let prompt = temporary.join("required.md");
    std::fs::write(
        &prompt,
        "---\nformat: opf/0.3\nid: extension.required\nextensions:\n  vendor.feature:\n    version: '1'\n    required: true\n    data: {}\n---\n\n## user\nHello\n",
    )
    .expect("write temporary prompt");

    let compatibility = Command::new(env!("CARGO_BIN_EXE_opf"))
        .current_dir(&root)
        .args([
            "compat",
            prompt.to_str().expect("UTF-8 path"),
            "--strict",
            "--format",
            "json",
        ])
        .output()
        .expect("run Rust opf CLI");
    assert_eq!(compatibility.status.code(), Some(2));
    let report: serde_json::Value =
        serde_json::from_slice(&compatibility.stdout).expect("compatibility output is JSON");
    assert_eq!(report["source_kind"], "opf/0.3");
    assert_eq!(report["target_kind"], "opf-core");
    assert_eq!(report["findings"][0]["code"], "extension.unsupported");
    assert_eq!(report["findings"][0]["severity"], "error");
    assert_eq!(report["findings"][0]["capability"], "vendor.feature");
    assert_eq!(report["can_apply"], false);

    let validation = Command::new(env!("CARGO_BIN_EXE_opf"))
        .current_dir(&root)
        .args(["validate", prompt.to_str().expect("UTF-8 path"), "--strict"])
        .output()
        .expect("run Rust opf CLI");
    assert_eq!(validation.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&validation.stderr).contains("strict compatibility failed"));

    std::fs::remove_dir_all(temporary).expect("remove temporary prompt directory");
}

#[test]
fn scan_finds_opf_files_without_writing_to_the_project() {
    let root = repo_root();
    let output = Command::new(env!("CARGO_BIN_EXE_opf"))
        .current_dir(&root)
        .args([
            "scan",
            "examples",
            "--include",
            "support.reply.md",
            "--format",
            "json",
        ])
        .output()
        .expect("run Rust opf CLI");

    assert!(output.status.success());
    let findings: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("scan output is JSON");
    let support = findings
        .as_array()
        .expect("scan result is a list")
        .iter()
        .find(|finding| finding["path"] == "support.reply.md")
        .expect("find checked-in OPF prompt");
    assert_eq!(support["source_kind"], "opf");
    assert_eq!(support["confidence"], "high");
    assert!(output.stderr.is_empty());
}

#[test]
fn init_creates_prompt_registry_and_agent_guidance() {
    let root = repo_root();
    let temporary = std::env::temp_dir().join(format!("opf-cli-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temporary);
    std::fs::create_dir_all(&temporary).expect("create temporary project");
    let output = Command::new(env!("CARGO_BIN_EXE_opf"))
        .current_dir(&root)
        .args([
            "init",
            temporary.to_str().expect("UTF-8 path"),
            "--id",
            "example.prompt",
        ])
        .output()
        .expect("run Rust opf CLI");

    assert!(output.status.success());
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("init output is JSON");
    assert!(result["created"]
        .as_array()
        .expect("created paths are a list")
        .iter()
        .any(|path| path == "prompts/example.prompt.md"));
    assert!(temporary.join("prompts/example.prompt.md").is_file());
    assert!(temporary.join("opf.yaml").is_file());
    assert!(temporary.join("AGENTS.md").is_file());
    assert!(output.stderr.is_empty());
    std::fs::remove_dir_all(temporary).expect("remove temporary project");
}
