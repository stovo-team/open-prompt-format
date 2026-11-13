# Open Prompt Format for Rust & langchain-rust

Rust workspace containing:

- `open-prompt-format` (`crates/opf`): Core OPF parser, validator, SHA-256 source digest calculator, and renderer.
- `open-prompt-format-langchain` (`crates/langchain`): Adapter implementing `langchain-rust`'s `MessageFormatter` and `FormatPrompter` traits.

## Usage with `langchain-rust`

```rust
use langchain_rust::prompt::{FormatPrompter, PromptArgs};
use open_prompt_format_langchain::OpfPromptTemplate;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let template = OpfPromptTemplate::load_by_id("support.reply", "examples")?;

    let mut args = PromptArgs::new();
    args.insert("customer_message".to_string(), json!("My order is late."));

    let prompt_value = template.format_prompt(args)?;
    let messages = prompt_value.to_chat_messages();
    println!("Formatted {} messages", messages.len());
    Ok(())
}
```

If an OPF prompt uses the `developer` role, `OpfPromptTemplate` rejects it by default because `langchain-rust`'s `MessageType` only supports `system`, `human`, `ai`, and `tool`. Call `.with_developer_as_system(true)` to explicitly map `developer` messages to `Message::new_system_message`.
