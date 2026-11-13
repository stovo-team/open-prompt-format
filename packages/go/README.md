# Open Prompt Format for Go & langchaingo

Go core library (`opf`) and [`langchaingo`](https://github.com/tmc/langchaingo) adapter (`opf/langchaingo`) for loading, validating, and rendering Open Prompt Format (`.md`) prompts.

## Installation

```sh
go get github.com/stovo-team/open-prompt-format/packages/go
```

## Usage with `langchaingo`

```go
package main

import (
	"context"
	"log"

	opflc "github.com/stovo-team/open-prompt-format/packages/go/langchaingo"
	"github.com/tmc/langchaingo/llms"
)

func main() {
	tmpl, err := opflc.LoadByID("support.reply", "examples")
	if err != nil {
		log.Fatal(err)
	}

	// Implements prompts.FormatPrompter and prompts.MessageFormatter
	promptValue, err := tmpl.FormatPrompt(map[string]any{
		"customer_message": "My order is late.",
	})
	if err != nil {
		log.Fatal(err)
	}
	_ = promptValue.Messages()

	// Or format directly as []llms.MessageContent for llms.Model.GenerateContent
	contents, err := tmpl.FormatMessageContents(map[string]any{
		"customer_message": "My order is late.",
	})
	if err != nil {
		log.Fatal(err)
	}
	_ = context.Background()
	_ = ([]llms.MessageContent)(contents)
}
```

Use `opflc.WithDeveloperAsSystem()` if your target provider does not accept the `developer` role.
