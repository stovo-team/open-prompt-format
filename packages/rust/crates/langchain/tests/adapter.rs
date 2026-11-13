use langchain_rust::prompt::{FormatPrompter, MessageFormatter, PromptArgs};
use langchain_rust::schemas::messages::MessageType;
use open_prompt_format_langchain::OpfPromptTemplate;
use serde_json::json;
use std::path::PathBuf;

fn sample_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("..")
        .join("fixtures")
        .join("prompts")
        .join("conformance.sample.md")
}

#[test]
fn rejects_developer_role_by_default_and_maps_with_option() {
    let tmpl = OpfPromptTemplate::load(sample_path()).unwrap();
    assert_eq!(tmpl.get_input_variables(), vec!["customer_message".to_string()]);
    assert_eq!(tmpl.input_variables(), vec!["customer_message".to_string()]);

    let mut args = PromptArgs::new();
    args.insert("customer_message".to_string(), json!("Where is my order?"));

    // Default rejects developer role rather than silently changing semantics
    assert!(tmpl.format_prompt(args.clone()).is_err());

    let tmpl_sys = tmpl.with_developer_as_system(true);
    let pv = tmpl_sys.format_prompt(args).unwrap();
    let msgs = pv.to_chat_messages();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].message_type, MessageType::SystemMessage);
    assert_eq!(msgs[1].message_type, MessageType::SystemMessage);
    assert_eq!(msgs[2].message_type, MessageType::HumanMessage);
    assert!(msgs[2].content.contains("Where is my order?"));
}

#[test]
fn rejects_non_string_input() {
    let tmpl = OpfPromptTemplate::load(sample_path())
        .unwrap()
        .with_developer_as_system(true);

    let mut args = PromptArgs::new();
    args.insert("customer_message".to_string(), json!(42));
    assert!(tmpl.format_messages(args).is_err());
}
