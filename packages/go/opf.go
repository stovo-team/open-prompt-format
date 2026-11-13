// Package opf implements parsing, validation, digesting, and rendering for
// Open Prompt Format (OPF) files.
package opf

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"

	"gopkg.in/yaml.v3"
)

// Role identifies a chat message role supported by OPF.
type Role string

const (
	RoleSystem    Role = "system"
	RoleDeveloper Role = "developer"
	RoleUser      Role = "user"
	RoleAssistant Role = "assistant"
)

// RenderedMessage is a single provider-neutral chat message produced by Prompt.Render.
type RenderedMessage struct {
	Role    Role   `json:"role"`
	Content string `json:"content"`
}

// MessageTemplate is a parsed, unrendered role and template content pair.
type MessageTemplate struct {
	Role    Role
	Content string
}

// InputSpec describes a declared template input in OPF frontmatter.
type InputSpec struct {
	Type     string `json:"type"`
	Required *bool  `json:"required,omitempty"`
}

// IsRequired returns whether the input is required (defaults to true).
func (s InputSpec) IsRequired() bool {
	if s.Required == nil {
		return true
	}
	return *s.Required
}

// Metadata contains the validated OPF frontmatter fields.
type Metadata struct {
	Format      string               `json:"format"`
	ID          string               `json:"id"`
	Version     string               `json:"version,omitempty"`
	Description string               `json:"description,omitempty"`
	Tags        []string             `json:"tags,omitempty"`
	Inputs      map[string]InputSpec `json:"inputs,omitempty"`
	Extensions  map[string]any       `json:"extensions,omitempty"`
}

// OPFError represents a validation, parsing, loading, or rendering error in OPF.
type OPFError struct {
	Message string
}

func (e *OPFError) Error() string {
	return e.Message
}

func errf(format string, args ...any) error {
	return &OPFError{Message: fmt.Sprintf(format, args...)}
}

var (
	idRe               = regexp.MustCompile(`^[a-z0-9]+(?:[._-][a-z0-9]+)*$`)
	versionRe          = regexp.MustCompile(`^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*)?$`)
	inputNameRe        = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)
	extensionIDRe      = regexp.MustCompile(`^[a-z][a-z0-9]*(?:[.-][a-z0-9]+)+$`)
	extensionVersionRe = regexp.MustCompile(`^[!-~]{1,64}$`)
	headingRe          = regexp.MustCompile(`^## (system|developer|user|assistant)$`)
	escapedHeadingRe   = regexp.MustCompile(`^\\(## (?:system|developer|user|assistant))$`)
	fenceRe            = regexp.MustCompile("^ {0,3}(`{3,}|~{3,})(.*)$")
	variableRe         = regexp.MustCompile(`\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}`)
)

// Prompt is a parsed and validated OPF prompt artifact.
type Prompt struct {
	Metadata     Metadata
	Messages     []MessageTemplate
	SourceDigest string
	Path         string
}

// InputVariables returns the sorted list of declared input variable names.
func (p *Prompt) InputVariables() []string {
	if len(p.Metadata.Inputs) == 0 {
		return nil
	}
	vars := make([]string, 0, len(p.Metadata.Inputs))
	for k := range p.Metadata.Inputs {
		vars = append(vars, k)
	}
	sort.Strings(vars)
	return vars
}

// Render substitutes declared inputs into the prompt's message templates.
func (p *Prompt) Render(inputs map[string]string) ([]RenderedMessage, error) {
	if inputs == nil {
		inputs = map[string]string{}
	}
	var unknown []string
	for name := range inputs {
		if _, ok := p.Metadata.Inputs[name]; !ok {
			unknown = append(unknown, name)
		}
	}
	if len(unknown) > 0 {
		sort.Strings(unknown)
		return nil, errf("undeclared input(s): %s", strings.Join(unknown, ", "))
	}

	var requiredNames []string
	for name, spec := range p.Metadata.Inputs {
		if spec.IsRequired() {
			requiredNames = append(requiredNames, name)
		}
	}
	sort.Strings(requiredNames)
	for _, name := range requiredNames {
		if _, ok := inputs[name]; !ok {
			return nil, errf("missing required input: %s", name)
		}
	}

	rendered := make([]RenderedMessage, 0, len(p.Messages))
	for _, msg := range p.Messages {
		content, err := renderTemplate(msg.Content, inputs)
		if err != nil {
			return nil, err
		}
		rendered = append(rendered, RenderedMessage{
			Role:    msg.Role,
			Content: content,
		})
	}
	return rendered, nil
}

// Parse parses and validates an OPF prompt from its text content.
func Parse(text string, path ...string) (*Prompt, error) {
	var filePath string
	if len(path) > 0 {
		filePath = path[0]
	}

	normalizedSource := normalizeNewlines(text)
	rawMeta, body, err := splitFrontmatter(normalizedSource)
	if err != nil {
		return nil, err
	}
	meta, err := validateMetadata(rawMeta)
	if err != nil {
		return nil, err
	}
	messages, err := parseMessages(body)
	if err != nil {
		return nil, err
	}
	if err := validateTemplates(meta, messages); err != nil {
		return nil, err
	}

	sum := sha256.Sum256([]byte(normalizedSource))
	digest := "sha256:" + hex.EncodeToString(sum[:])

	return &Prompt{
		Metadata:     meta,
		Messages:     messages,
		SourceDigest: digest,
		Path:         filePath,
	}, nil
}

// Load reads and parses an OPF prompt file from disk.
func Load(path string) (*Prompt, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, errf("cannot read prompt file %s: %v", path, err)
	}
	return Parse(string(data), path)
}

// LoadByID loads all OPF prompts in collection and returns the one matching promptID.
func LoadByID(promptID string, collection string) (*Prompt, error) {
	prompts, err := LoadCollection(collection)
	if err != nil {
		return nil, err
	}
	for _, p := range prompts {
		if p.Metadata.ID == promptID {
			return p, nil
		}
	}
	return nil, errf("prompt id '%s' not found in %s", promptID, collection)
}

// LoadCollection recursively scans a directory for OPF .md files and rejects duplicate IDs.
func LoadCollection(collection string) ([]*Prompt, error) {
	info, err := os.Stat(collection)
	if err != nil {
		return nil, errf("cannot scan prompt collection %s: %v", collection, err)
	}
	if !info.IsDir() {
		return nil, errf("cannot scan prompt collection %s: not a directory", collection)
	}

	var files []string
	err = filepath.WalkDir(collection, func(p string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return errf("cannot scan prompt collection %s: %v", p, walkErr)
		}
		if !d.IsDir() && strings.HasSuffix(d.Name(), ".md") {
			files = append(files, p)
		}
		return nil
	})
	if err != nil {
		var opfErr *OPFError
		if errors.As(err, &opfErr) {
			return nil, err
		}
		return nil, errf("cannot scan prompt collection %s: %v", collection, err)
	}
	sort.Strings(files)

	prompts := make([]*Prompt, 0, len(files))
	seen := make(map[string]string)
	for _, filePath := range files {
		raw, err := os.ReadFile(filePath)
		if err != nil {
			return nil, errf("cannot read prompt file %s: %v", filePath, err)
		}
		source := string(raw)
		trimmedBOM := strings.TrimPrefix(source, "\uFEFF")
		firstLine := trimmedBOM
		if idx := strings.IndexAny(trimmedBOM, "\r\n"); idx >= 0 {
			firstLine = trimmedBOM[:idx]
		}
		if firstLine != "---" {
			continue
		}
		p, err := Parse(source, filePath)
		if err != nil {
			return nil, err
		}
		if prior, exists := seen[p.Metadata.ID]; exists {
			return nil, errf("duplicate prompt id '%s': %s, %s", p.Metadata.ID, prior, filePath)
		}
		seen[p.Metadata.ID] = filePath
		prompts = append(prompts, p)
	}
	return prompts, nil
}

func normalizeNewlines(text string) string {
	text = strings.ReplaceAll(text, "\r\n", "\n")
	text = strings.ReplaceAll(text, "\r", "\n")
	return text
}

func splitFrontmatter(normalized string) (map[string]any, string, error) {
	trimmed := strings.TrimPrefix(normalized, "\uFEFF")
	lines := strings.Split(trimmed, "\n")
	if len(lines) == 0 || lines[0] != "---" {
		return nil, "", errf("frontmatter opening delimiter '---' must be the first line")
	}
	closing := -1
	for i := 1; i < len(lines); i++ {
		if lines[i] == "---" {
			closing = i
			break
		}
	}
	if closing < 0 {
		return nil, "", errf("frontmatter closing delimiter '---' was not found")
	}

	yamlSource := strings.Join(lines[1:closing], "\n")
	var doc yaml.Node
	if err := yaml.Unmarshal([]byte(yamlSource), &doc); err != nil {
		return nil, "", errf("invalid YAML frontmatter: %v", err)
	}
	if doc.Kind != yaml.DocumentNode || len(doc.Content) == 0 {
		return nil, "", errf("frontmatter must be a YAML mapping")
	}
	root := doc.Content[0]
	if root.Kind != yaml.MappingNode {
		return nil, "", errf("frontmatter must be a YAML mapping")
	}

	val, err := nodeToValue(root, "frontmatter")
	if err != nil {
		return nil, "", err
	}
	mapping, ok := val.(map[string]any)
	if !ok {
		return nil, "", errf("frontmatter must be a YAML mapping")
	}

	body := strings.Join(lines[closing+1:], "\n")
	return mapping, body, nil
}

func nodeToValue(node *yaml.Node, path string) (any, error) {
	for node.Kind == yaml.AliasNode {
		if node.Alias == nil {
			return nil, errf("%s contains invalid YAML alias", path)
		}
		node = node.Alias
	}
	switch node.Kind {
	case yaml.MappingNode:
		if len(node.Content)%2 != 0 {
			return nil, errf("%s contains malformed YAML mapping", path)
		}
		out := make(map[string]any, len(node.Content)/2)
		for i := 0; i < len(node.Content); i += 2 {
			kNode := node.Content[i]
			vNode := node.Content[i+1]
			if kNode.Kind != yaml.ScalarNode || kNode.Tag != "!!str" {
				return nil, errf("%s must use string mapping keys", path)
			}
			key := kNode.Value
			if _, dup := out[key]; dup {
				return nil, errf("invalid YAML frontmatter: duplicate key %q in %s", key, path)
			}
			val, err := nodeToValue(vNode, path+"."+key)
			if err != nil {
				return nil, err
			}
			out[key] = val
		}
		return out, nil
	case yaml.SequenceNode:
		out := make([]any, len(node.Content))
		for i, item := range node.Content {
			val, err := nodeToValue(item, fmt.Sprintf("%s[%d]", path, i))
			if err != nil {
				return nil, err
			}
			out[i] = val
		}
		return out, nil
	case yaml.ScalarNode:
		if node.Tag == "!!str" || node.Tag == "!!timestamp" {
			return node.Value, nil
		}
		var v any
		if err := node.Decode(&v); err != nil {
			return nil, errf("invalid YAML frontmatter at %s: %v", path, err)
		}
		return v, nil
	default:
		return nil, errf("unsupported YAML node kind at %s", path)
	}
}

func validateMetadata(raw map[string]any) (Metadata, error) {
	allowed := map[string]bool{
		"format":      true,
		"id":          true,
		"version":     true,
		"description": true,
		"tags":        true,
		"inputs":      true,
		"extensions":  true,
	}
	var unknown []string
	for k := range raw {
		if !allowed[k] {
			unknown = append(unknown, k)
		}
	}
	if len(unknown) > 0 {
		sort.Strings(unknown)
		return Metadata{}, errf("unknown frontmatter field(s): %s", strings.Join(unknown, ", "))
	}

	for _, req := range []string{"format", "id"} {
		if _, ok := raw[req]; !ok {
			return Metadata{}, errf("missing required frontmatter field: %s", req)
		}
	}

	formatVal, ok := raw["format"].(string)
	if !ok || (formatVal != "opf/0.1" && formatVal != "opf/0.2" && formatVal != "opf/0.3") {
		return Metadata{}, errf("unsupported format %v; expected 'opf/0.1', 'opf/0.2', or 'opf/0.3'", raw["format"])
	}

	_, hasVersion := raw["version"]
	if formatVal == "opf/0.1" && !hasVersion {
		return Metadata{}, errf("missing required frontmatter field: version")
	}
	if (formatVal == "opf/0.2" || formatVal == "opf/0.3") && hasVersion {
		return Metadata{}, errf("%s keeps release versions in the registry; remove frontmatter version", formatVal)
	}

	idVal, ok := raw["id"].(string)
	if !ok || !idRe.MatchString(idVal) {
		return Metadata{}, errf("id must match [a-z0-9]+(?:[._-][a-z0-9]+)*")
	}

	var versionVal string
	if hasVersion {
		v, ok := raw["version"].(string)
		if !ok || !versionRe.MatchString(v) {
			return Metadata{}, errf("version must be SemVer MAJOR.MINOR.PATCH without build metadata")
		}
		versionVal = v
	}

	var descVal string
	if rawDesc, ok := raw["description"]; ok {
		d, ok := rawDesc.(string)
		if !ok {
			return Metadata{}, errf("description must be a string")
		}
		descVal = d
	}

	var tagsVal []string
	if rawTags, ok := raw["tags"]; ok {
		seq, ok := rawTags.([]any)
		if !ok {
			return Metadata{}, errf("tags must be a list of strings")
		}
		tagsVal = make([]string, len(seq))
		for i, item := range seq {
			s, ok := item.(string)
			if !ok {
				return Metadata{}, errf("tags must be a list of strings")
			}
			tagsVal[i] = s
		}
	}

	var extensionsVal map[string]any
	if rawExt, ok := raw["extensions"]; ok {
		extMap, ok := rawExt.(map[string]any)
		if !ok {
			return Metadata{}, errf("extensions must be a mapping")
		}
		if formatVal == "opf/0.3" {
			for identifier, rawRecord := range extMap {
				if len([]byte(identifier)) > 255 || !extensionIDRe.MatchString(identifier) {
					return Metadata{}, errf("invalid extension identifier %q", identifier)
				}
				rec, ok := rawRecord.(map[string]any)
				if !ok {
					return Metadata{}, errf("extension '%s' must be a mapping", identifier)
				}
				if len(rec) != 3 {
					return Metadata{}, errf("extension '%s' requires exactly version, required, and data", identifier)
				}
				verRaw, hasVer := rec["version"]
				reqRaw, hasReq := rec["required"]
				dataRaw, hasData := rec["data"]
				if !hasVer || !hasReq || !hasData {
					return Metadata{}, errf("extension '%s' requires exactly version, required, and data", identifier)
				}
				verStr, ok := verRaw.(string)
				if !ok || !extensionVersionRe.MatchString(verStr) {
					return Metadata{}, errf("extension '%s' version must be 1-64 printable non-space ASCII characters", identifier)
				}
				if _, ok := reqRaw.(bool); !ok {
					return Metadata{}, errf("extension '%s' required must be a boolean", identifier)
				}
				dataMap, ok := dataRaw.(map[string]any)
				if !ok {
					return Metadata{}, errf("extension '%s' data must be a mapping", identifier)
				}
				if err := validateJSONValue(dataMap, fmt.Sprintf("extension '%s' data", identifier)); err != nil {
					return Metadata{}, err
				}
			}
		}
		extensionsVal = extMap
	}

	inputsVal := make(map[string]InputSpec)
	if rawInputs, ok := raw["inputs"]; ok {
		inMap, ok := rawInputs.(map[string]any)
		if !ok {
			return Metadata{}, errf("inputs must be a mapping")
		}
		for name, rawSpec := range inMap {
			if !inputNameRe.MatchString(name) {
				return Metadata{}, errf("invalid input name %q", name)
			}
			specMap, ok := rawSpec.(map[string]any)
			if !ok {
				return Metadata{}, errf("input '%s' must be a mapping", name)
			}
			var extra []string
			for k := range specMap {
				if k != "type" && k != "required" {
					extra = append(extra, k)
				}
			}
			if len(extra) > 0 {
				sort.Strings(extra)
				return Metadata{}, errf("unknown field(s) for input '%s': %s", name, strings.Join(extra, ", "))
			}
			typeStr, ok := specMap["type"].(string)
			if !ok || typeStr != "string" {
				return Metadata{}, errf("input '%s' must declare type: string", name)
			}
			spec := InputSpec{Type: "string"}
			if reqRaw, hasReq := specMap["required"]; hasReq {
				reqBool, ok := reqRaw.(bool)
				if !ok {
					return Metadata{}, errf("input '%s' required must be a boolean", name)
				}
				spec.Required = &reqBool
			}
			inputsVal[name] = spec
		}
	}

	return Metadata{
		Format:      formatVal,
		ID:          idVal,
		Version:     versionVal,
		Description: descVal,
		Tags:        tagsVal,
		Inputs:      inputsVal,
		Extensions:  extensionsVal,
	}, nil
}

func validateJSONValue(val any, path string) error {
	switch v := val.(type) {
	case nil, string, bool, int, int64, uint64:
		return nil
	case float64:
		if math.IsNaN(v) || math.IsInf(v, 0) {
			return errf("%s must not contain a non-finite number", path)
		}
		return nil
	case []any:
		for i, item := range v {
			if err := validateJSONValue(item, fmt.Sprintf("%s[%d]", path, i)); err != nil {
				return err
			}
		}
		return nil
	case map[string]any:
		for k, item := range v {
			if err := validateJSONValue(item, path+"."+k); err != nil {
				return err
			}
		}
		return nil
	default:
		return errf("%s contains a value that is not JSON-compatible", path)
	}
}

func parseMessages(body string) ([]MessageTemplate, error) {
	var messages []MessageTemplate
	var currentRole Role
	var content []string
	var fenceChar byte
	fenceLength := 0

	for _, line := range strings.Split(body, "\n") {
		if m := fenceRe.FindStringSubmatch(line); m != nil {
			marker := m[1]
			ch := marker[0]
			if fenceChar == 0 {
				fenceChar = ch
				fenceLength = len(marker)
			} else if ch == fenceChar && len(marker) >= fenceLength && strings.TrimSpace(m[2]) == "" {
				fenceChar = 0
				fenceLength = 0
			} else if currentRole != "" {
				content = append(content, line)
				continue
			}
			if currentRole != "" {
				content = append(content, line)
			} else if strings.TrimSpace(line) != "" {
				return nil, errf("non-whitespace content appears before the first message")
			}
			continue
		}

		if fenceChar == 0 {
			if m := headingRe.FindStringSubmatch(line); m != nil {
				if currentRole != "" {
					messages = append(messages, MessageTemplate{
						Role:    currentRole,
						Content: trimBlankEdges(content),
					})
				}
				currentRole = Role(m[1])
				content = nil
				continue
			}
		}

		if currentRole == "" {
			if strings.TrimSpace(line) != "" {
				return nil, errf("non-whitespace content appears before the first message")
			}
			continue
		}

		if fenceChar == 0 {
			if m := escapedHeadingRe.FindStringSubmatch(line); m != nil {
				content = append(content, m[1])
				continue
			}
		}
		content = append(content, line)
	}

	if currentRole != "" {
		messages = append(messages, MessageTemplate{
			Role:    currentRole,
			Content: trimBlankEdges(content),
		})
	}
	if len(messages) == 0 {
		return nil, errf("prompt must contain at least one message")
	}
	return messages, nil
}

func trimBlankEdges(lines []string) string {
	start := 0
	end := len(lines)
	for start < end && strings.TrimSpace(lines[start]) == "" {
		start++
	}
	for end > start && strings.TrimSpace(lines[end-1]) == "" {
		end--
	}
	return strings.Join(lines[start:end], "\n")
}

// findTemplateVariables returns unique variable names in template and checks for
// malformed/unsupported template syntax (e.g. unescaped {{ not matching {{ var }} or {%).
func findTemplateVariables(template string) ([]string, error) {
	// Replace \{{ with a placeholder that cannot contain {{ or {%
	const escPlaceholder = "\x00OPF_ESC_BRACE\x00"
	withoutEsc := strings.ReplaceAll(template, `\{{`, escPlaceholder)
	stripped := variableRe.ReplaceAllString(withoutEsc, "")
	if strings.Contains(stripped, "{{") || strings.Contains(stripped, "{%") {
		return nil, errf("unsupported or malformed template expression; use only {{ input_name }}")
	}

	matches := variableRe.FindAllStringSubmatch(withoutEsc, -1)
	seen := make(map[string]bool)
	var vars []string
	for _, m := range matches {
		name := m[1]
		if !seen[name] {
			seen[name] = true
			vars = append(vars, name)
		}
	}
	return vars, nil
}

func validateTemplates(meta Metadata, messages []MessageTemplate) error {
	for _, msg := range messages {
		vars, err := findTemplateVariables(msg.Content)
		if err != nil {
			return err
		}
		var unknown []string
		for _, v := range vars {
			if _, ok := meta.Inputs[v]; !ok {
				unknown = append(unknown, v)
			}
		}
		if len(unknown) > 0 {
			sort.Strings(unknown)
			return errf("template uses undeclared input(s): %s", strings.Join(unknown, ", "))
		}
	}
	return nil
}

func renderTemplate(template string, inputs map[string]string) (string, error) {
	// Protect escaped \{{ so variableRe doesn't match inside `\{{ foo }}`
	const escPlaceholder = "\x00OPF_ESC_BRACE\x00"
	protected := strings.ReplaceAll(template, `\{{`, escPlaceholder)

	vars, err := findTemplateVariables(template)
	if err != nil {
		return "", err
	}
	var missing []string
	for _, v := range vars {
		if _, ok := inputs[v]; !ok {
			missing = append(missing, v)
		}
	}
	if len(missing) > 0 {
		sort.Strings(missing)
		return "", errf("missing template input(s): %s", strings.Join(missing, ", "))
	}

	rendered := variableRe.ReplaceAllStringFunc(protected, func(match string) string {
		sub := variableRe.FindStringSubmatch(match)
		return inputs[sub[1]]
	})
	return strings.ReplaceAll(rendered, escPlaceholder, "{{"), nil
}
