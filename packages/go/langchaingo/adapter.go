// Package langchaingo adapts Open Prompt Format (OPF) prompts to
// github.com/tmc/langchaingo prompt and message interfaces.
package langchaingo

import (
	"fmt"

	opf "github.com/stovo-team/open-prompt-format/packages/go"
	"github.com/tmc/langchaingo/llms"
	"github.com/tmc/langchaingo/prompts"
)

// Option configures how an OPF prompt is adapted to langchaingo types.
type Option func(*config)

type config struct {
	developerAsSystem bool
}

// WithDeveloperAsSystem maps OPF "developer" role messages to
// llms.SystemChatMessage / llms.ChatMessageTypeSystem instead of a
// generic "developer" role message. Use this when targeting providers that
// do not accept the "developer" role.
func WithDeveloperAsSystem() Option {
	return func(c *config) {
		c.developerAsSystem = true
	}
}

func applyOptions(opts []Option) config {
	var cfg config
	for _, opt := range opts {
		if opt != nil {
			opt(&cfg)
		}
	}
	return cfg
}

// ChatPromptTemplate adapts an *opf.Prompt to langchaingo's
// prompts.FormatPrompter and prompts.MessageFormatter interfaces.
type ChatPromptTemplate struct {
	Prompt *opf.Prompt
	cfg    config
}

var (
	_ prompts.FormatPrompter   = (*ChatPromptTemplate)(nil)
	_ prompts.MessageFormatter = (*ChatPromptTemplate)(nil)
)

// NewChatPromptTemplate wraps a parsed OPF prompt for use with langchaingo.
func NewChatPromptTemplate(prompt *opf.Prompt, opts ...Option) *ChatPromptTemplate {
	return &ChatPromptTemplate{
		Prompt: prompt,
		cfg:    applyOptions(opts),
	}
}

// Load reads an OPF file from disk and wraps it as a ChatPromptTemplate.
func Load(path string, opts ...Option) (*ChatPromptTemplate, error) {
	p, err := opf.Load(path)
	if err != nil {
		return nil, err
	}
	return NewChatPromptTemplate(p, opts...), nil
}

// LoadByID loads an OPF prompt by ID from a collection directory and wraps it
// as a ChatPromptTemplate.
func LoadByID(promptID, collection string, opts ...Option) (*ChatPromptTemplate, error) {
	p, err := opf.LoadByID(promptID, collection)
	if err != nil {
		return nil, err
	}
	return NewChatPromptTemplate(p, opts...), nil
}

// GetInputVariables returns the sorted list of input variable names declared
// in the OPF prompt frontmatter.
func (t *ChatPromptTemplate) GetInputVariables() []string {
	if t == nil || t.Prompt == nil {
		return nil
	}
	return t.Prompt.InputVariables()
}

// FormatPrompt renders the OPF prompt with the supplied input values and
// returns a langchaingo llms.PromptValue (specifically prompts.ChatPromptValue).
func (t *ChatPromptTemplate) FormatPrompt(values map[string]any) (llms.PromptValue, error) {
	msgs, err := t.FormatMessages(values)
	if err != nil {
		return nil, err
	}
	return prompts.ChatPromptValue(msgs), nil
}

// FormatMessages renders the OPF prompt with the supplied input values and
// returns a slice of langchaingo llms.ChatMessage.
func (t *ChatPromptTemplate) FormatMessages(values map[string]any) ([]llms.ChatMessage, error) {
	if t == nil || t.Prompt == nil {
		return nil, &opf.OPFError{Message: "langchaingo adapter: nil OPF prompt"}
	}
	return toChatMessages(t.Prompt, values, t.cfg)
}

// FormatMessageContents renders the OPF prompt with the supplied input values
// and returns a slice of llms.MessageContent suitable for llms.Model.GenerateContent.
func (t *ChatPromptTemplate) FormatMessageContents(values map[string]any) ([]llms.MessageContent, error) {
	if t == nil || t.Prompt == nil {
		return nil, &opf.OPFError{Message: "langchaingo adapter: nil OPF prompt"}
	}
	return toMessageContents(t.Prompt, values, t.cfg)
}

// ToChatMessages renders an *opf.Prompt and converts its messages into
// []llms.ChatMessage.
func ToChatMessages(prompt *opf.Prompt, values map[string]any, opts ...Option) ([]llms.ChatMessage, error) {
	if prompt == nil {
		return nil, &opf.OPFError{Message: "langchaingo adapter: nil OPF prompt"}
	}
	return toChatMessages(prompt, values, applyOptions(opts))
}

// ToMessageContents renders an *opf.Prompt and converts its messages into
// []llms.MessageContent.
func ToMessageContents(prompt *opf.Prompt, values map[string]any, opts ...Option) ([]llms.MessageContent, error) {
	if prompt == nil {
		return nil, &opf.OPFError{Message: "langchaingo adapter: nil OPF prompt"}
	}
	return toMessageContents(prompt, values, applyOptions(opts))
}

func convertInputs(values map[string]any) (map[string]string, error) {
	if len(values) == 0 {
		return map[string]string{}, nil
	}
	out := make(map[string]string, len(values))
	for k, v := range values {
		s, ok := v.(string)
		if !ok {
			return nil, &opf.OPFError{
				Message: fmt.Sprintf("input '%s' must be a string, got %T", k, v),
			}
		}
		out[k] = s
	}
	return out, nil
}

func toChatMessages(prompt *opf.Prompt, values map[string]any, cfg config) ([]llms.ChatMessage, error) {
	inputs, err := convertInputs(values)
	if err != nil {
		return nil, err
	}
	rendered, err := prompt.Render(inputs)
	if err != nil {
		return nil, err
	}
	out := make([]llms.ChatMessage, 0, len(rendered))
	for _, m := range rendered {
		switch m.Role {
		case opf.RoleSystem:
			out = append(out, llms.SystemChatMessage{Content: m.Content})
		case opf.RoleDeveloper:
			if cfg.developerAsSystem {
				out = append(out, llms.SystemChatMessage{Content: m.Content})
			} else {
				out = append(out, llms.GenericChatMessage{Role: string(opf.RoleDeveloper), Content: m.Content})
			}
		case opf.RoleUser:
			out = append(out, llms.HumanChatMessage{Content: m.Content})
		case opf.RoleAssistant:
			out = append(out, llms.AIChatMessage{Content: m.Content})
		default:
			return nil, &opf.OPFError{Message: fmt.Sprintf("unsupported OPF role %q", m.Role)}
		}
	}
	return out, nil
}

func toMessageContents(prompt *opf.Prompt, values map[string]any, cfg config) ([]llms.MessageContent, error) {
	inputs, err := convertInputs(values)
	if err != nil {
		return nil, err
	}
	rendered, err := prompt.Render(inputs)
	if err != nil {
		return nil, err
	}
	out := make([]llms.MessageContent, 0, len(rendered))
	for _, m := range rendered {
		var role llms.ChatMessageType
		switch m.Role {
		case opf.RoleSystem:
			role = llms.ChatMessageTypeSystem
		case opf.RoleDeveloper:
			if cfg.developerAsSystem {
				role = llms.ChatMessageTypeSystem
			} else {
				role = llms.ChatMessageType("developer")
			}
		case opf.RoleUser:
			role = llms.ChatMessageTypeHuman
		case opf.RoleAssistant:
			role = llms.ChatMessageTypeAI
		default:
			return nil, &opf.OPFError{Message: fmt.Sprintf("unsupported OPF role %q", m.Role)}
		}
		out = append(out, llms.TextParts(role, m.Content))
	}
	return out, nil
}
