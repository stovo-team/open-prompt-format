//! `langchain-rust` adapter for Open Prompt Format (OPF).

use langchain_rust::prompt::{FormatPrompter, MessageFormatter, PromptArgs, PromptError};
use langchain_rust::schemas::messages::Message;
use langchain_rust::schemas::prompt::PromptValue;
use open_prompt_format::{load, load_by_id, OpfError, Prompt, Role};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Adapts an [`open_prompt_format::Prompt`] to `langchain-rust`'s
/// [`MessageFormatter`] and [`FormatPrompter`] traits.
#[derive(Debug, Clone)]
pub struct OpfPromptTemplate {
    prompt: Prompt,
    developer_as_system: bool,
}

impl OpfPromptTemplate {
    /// Wraps a parsed OPF [`Prompt`].
    pub fn new(prompt: Prompt) -> Self {
        Self {
            prompt,
            developer_as_system: false,
        }
    }

    /// Loads an OPF prompt file from disk and wraps it.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, OpfError> {
        Ok(Self::new(load(path)?))
    }

    /// Loads an OPF prompt by ID from a collection directory and wraps it.
    pub fn load_by_id(prompt_id: &str, collection: impl AsRef<Path>) -> Result<Self, OpfError> {
        Ok(Self::new(load_by_id(prompt_id, collection)?))
    }

    /// Controls whether OPF `developer` messages are mapped to `langchain-rust`
    /// system messages (`Message::new_system_message`).
    ///
    /// By default (`false`), `developer` messages return an error because
    /// `langchain-rust` does not have a dedicated `developer` message type.
    pub fn with_developer_as_system(mut self, enabled: bool) -> Self {
        self.developer_as_system = enabled;
        self
    }

    /// Returns a reference to the underlying OPF [`Prompt`].
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    /// Formats the OPF prompt into `langchain-rust` [`Message`] values.
    pub fn format_opf_messages(
        &self,
        input_variables: &PromptArgs,
    ) -> Result<Vec<Message>, PromptError> {
        to_messages_with_options(&self.prompt, input_variables, self.developer_as_system)
    }
}

impl MessageFormatter for OpfPromptTemplate {
    fn format_messages(&self, input_variables: PromptArgs) -> Result<Vec<Message>, PromptError> {
        self.format_opf_messages(&input_variables)
    }

    fn input_variables(&self) -> Vec<String> {
        self.prompt.input_variables()
    }
}

impl FormatPrompter for OpfPromptTemplate {
    fn format_prompt(&self, input_variables: PromptArgs) -> Result<PromptValue, PromptError> {
        let messages = self.format_opf_messages(&input_variables)?;
        Ok(PromptValue::from_messages(messages))
    }

    fn get_input_variables(&self) -> Vec<String> {
        self.prompt.input_variables()
    }
}

/// Renders an OPF [`Prompt`] and converts its messages into `langchain-rust` [`Message`]s.
pub fn to_messages(prompt: &Prompt, input_variables: &PromptArgs) -> Result<Vec<Message>, PromptError> {
    to_messages_with_options(prompt, input_variables, false)
}

/// Renders an OPF [`Prompt`] and converts its messages into `langchain-rust` [`Message`]s,
/// optionally mapping `developer` messages to system messages.
pub fn to_messages_with_options(
    prompt: &Prompt,
    input_variables: &PromptArgs,
    developer_as_system: bool,
) -> Result<Vec<Message>, PromptError> {
    let mut string_inputs = HashMap::with_capacity(input_variables.len());
    for (k, v) in input_variables {
        match v {
            Value::String(s) => {
                string_inputs.insert(k.clone(), s.clone());
            }
            _ => {
                return Err(PromptError::OtherError(format!(
                    "input '{k}' must be a string"
                )));
            }
        }
    }

    let rendered = prompt
        .render(&string_inputs)
        .map_err(|e| PromptError::OtherError(e.to_string()))?;

    let mut out = Vec::with_capacity(rendered.len());
    for msg in rendered {
        let lc_msg = match msg.role {
            Role::System => Message::new_system_message(msg.content),
            Role::Developer => {
                if developer_as_system {
                    Message::new_system_message(msg.content)
                } else {
                    return Err(PromptError::OtherError(
                        "OPF role 'developer' is not supported by langchain-rust MessageType; enable with_developer_as_system(true) to map it to system".to_string(),
                    ));
                }
            }
            Role::User => Message::new_human_message(msg.content),
            Role::Assistant => Message::new_ai_message(msg.content),
        };
        out.push(lc_msg);
    }
    Ok(out)
}
