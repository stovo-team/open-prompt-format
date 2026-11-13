package langchaingo_test

import (
	"path/filepath"
	"reflect"
	"testing"

	"github.com/stovo-team/open-prompt-format/packages/go/langchaingo"
	"github.com/tmc/langchaingo/llms"
	"github.com/tmc/langchaingo/prompts"
)

func TestLangchaingoChatPromptTemplate(t *testing.T) {
	samplePath := filepath.Join("..", "..", "..", "fixtures", "prompts", "conformance.sample.md")
	tmpl, err := langchaingo.Load(samplePath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}

	var _ prompts.FormatPrompter = tmpl
	var _ prompts.MessageFormatter = tmpl

	if got, want := tmpl.GetInputVariables(), []string{"customer_message"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("GetInputVariables() = %v, want %v", got, want)
	}

	pv, err := tmpl.FormatPrompt(map[string]any{
		"customer_message": "Where is my order?",
	})
	if err != nil {
		t.Fatalf("FormatPrompt: %v", err)
	}
	msgs := pv.Messages()
	if len(msgs) != 3 {
		t.Fatalf("got %d messages, want 3", len(msgs))
	}
	if msgs[0].GetType() != llms.ChatMessageTypeSystem {
		t.Fatalf("msg[0] type = %q, want system", msgs[0].GetType())
	}
	if msgs[1].GetType() != llms.ChatMessageTypeGeneric {
		t.Fatalf("msg[1] type = %q, want generic", msgs[1].GetType())
	}
	if gen, ok := msgs[1].(llms.GenericChatMessage); !ok || gen.Role != "developer" {
		t.Fatalf("msg[1] = %#v, want GenericChatMessage with role developer", msgs[1])
	}
	if msgs[2].GetType() != llms.ChatMessageTypeHuman {
		t.Fatalf("msg[2] type = %q, want human", msgs[2].GetType())
	}
}

func TestDeveloperAsSystemOptionAndMessageContents(t *testing.T) {
	samplePath := filepath.Join("..", "..", "..", "fixtures", "prompts", "conformance.sample.md")
	tmpl, err := langchaingo.Load(samplePath, langchaingo.WithDeveloperAsSystem())
	if err != nil {
		t.Fatalf("Load: %v", err)
	}

	contents, err := tmpl.FormatMessageContents(map[string]any{
		"customer_message": "Hello",
	})
	if err != nil {
		t.Fatalf("FormatMessageContents: %v", err)
	}
	if len(contents) != 3 {
		t.Fatalf("got %d contents, want 3", len(contents))
	}
	if contents[1].Role != llms.ChatMessageTypeSystem {
		t.Fatalf("contents[1].Role = %q, want system", contents[1].Role)
	}
}

func TestNonStringInputRejected(t *testing.T) {
	samplePath := filepath.Join("..", "..", "..", "fixtures", "prompts", "conformance.sample.md")
	tmpl, err := langchaingo.Load(samplePath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}

	if _, err := tmpl.FormatMessages(map[string]any{"customer_message": 123}); err == nil {
		t.Fatal("expected error for non-string input")
	}
}
