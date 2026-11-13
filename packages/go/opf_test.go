package opf_test

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	opf "github.com/stovo-team/open-prompt-format/packages/go"
)

type conformanceSuite struct {
	Valid []struct {
		File     string            `json:"file"`
		Inputs   map[string]string `json:"inputs"`
		Expected []struct {
			Role    opf.Role `json:"role"`
			Content string   `json:"content"`
		} `json:"expected"`
	} `json:"valid"`
	Invalid []string `json:"invalid"`
}

func fixturesDir(t *testing.T) string {
	t.Helper()
	return filepath.Join("..", "..", "fixtures")
}

func TestConformanceValidAndInvalid(t *testing.T) {
	fdir := fixturesDir(t)
	raw, err := os.ReadFile(filepath.Join(fdir, "conformance.json"))
	if err != nil {
		t.Fatalf("read conformance.json: %v", err)
	}
	var suite conformanceSuite
	if err := json.Unmarshal(raw, &suite); err != nil {
		t.Fatalf("parse conformance.json: %v", err)
	}

	for _, tc := range suite.Valid {
		t.Run("valid/"+tc.File, func(t *testing.T) {
			p, err := opf.Load(filepath.Join(fdir, tc.File))
			if err != nil {
				t.Fatalf("Load(%s) unexpected error: %v", tc.File, err)
			}
			got, err := p.Render(tc.Inputs)
			if err != nil {
				t.Fatalf("Render(%s) unexpected error: %v", tc.File, err)
			}
			want := make([]opf.RenderedMessage, len(tc.Expected))
			for i, m := range tc.Expected {
				want[i] = opf.RenderedMessage{Role: m.Role, Content: m.Content}
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("rendered mismatch:\ngot:  %#v\nwant: %#v", got, want)
			}
			if !strings.HasPrefix(p.SourceDigest, "sha256:") || len(p.SourceDigest) != 7+64 {
				t.Fatalf("invalid source digest: %q", p.SourceDigest)
			}
		})
	}

	for _, rel := range suite.Invalid {
		t.Run("invalid/"+rel, func(t *testing.T) {
			_, err := opf.Load(filepath.Join(fdir, rel))
			if err == nil {
				t.Fatalf("expected error for invalid fixture %s, got nil", rel)
			}
			var opfErr *opf.OPFError
			if !errors.As(err, &opfErr) {
				t.Fatalf("expected *opf.OPFError, got %T: %v", err, err)
			}
		})
	}
}

func TestNewlineNormalizationDigest(t *testing.T) {
	lf := "---\nformat: opf/0.2\nid: norm.test\n---\n\n## user\nHello\n"
	crlf := strings.ReplaceAll(lf, "\n", "\r\n")
	cr := strings.ReplaceAll(lf, "\n", "\r")

	pLF, err := opf.Parse(lf)
	if err != nil {
		t.Fatalf("parse LF: %v", err)
	}
	pCRLF, err := opf.Parse(crlf)
	if err != nil {
		t.Fatalf("parse CRLF: %v", err)
	}
	pCR, err := opf.Parse(cr)
	if err != nil {
		t.Fatalf("parse CR: %v", err)
	}

	if pLF.SourceDigest != pCRLF.SourceDigest || pLF.SourceDigest != pCR.SourceDigest {
		t.Fatalf("digests differ: LF=%s CRLF=%s CR=%s", pLF.SourceDigest, pCRLF.SourceDigest, pCR.SourceDigest)
	}
}

func TestRenderValidationErrors(t *testing.T) {
	src := "---\nformat: opf/0.1\nid: req.test\nversion: 1.0.0\ninputs:\n  name:\n    type: string\n    required: true\n  opt:\n    type: string\n    required: false\n---\n\n## user\nHi {{ name }}\n"
	p, err := opf.Parse(src)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}

	if _, err := p.Render(nil); err == nil {
		t.Fatal("expected error for missing required input")
	}
	if _, err := p.Render(map[string]string{"name": "Ada", "extra": "1"}); err == nil {
		t.Fatal("expected error for undeclared input")
	}
	msgs, err := p.Render(map[string]string{"name": "Ada"})
	if err != nil {
		t.Fatalf("render: %v", err)
	}
	if len(msgs) != 1 || msgs[0].Content != "Hi Ada" {
		t.Fatalf("unexpected rendered messages: %#v", msgs)
	}
}

func TestCollectionLoadByIDAndDuplicateRejection(t *testing.T) {
	dir := t.TempDir()
	a := "---\nformat: opf/0.2\nid: alpha.one\n---\n\n## user\nA\n"
	b := "---\nformat: opf/0.2\nid: beta.two\n---\n\n## user\nB\n"
	if err := os.WriteFile(filepath.Join(dir, "a.md"), []byte(a), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "b.md"), []byte(b), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "notes.md"), []byte("# Not OPF\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	p, err := opf.LoadByID("beta.two", dir)
	if err != nil {
		t.Fatalf("LoadByID: %v", err)
	}
	if p.Metadata.ID != "beta.two" {
		t.Fatalf("got id %q, want beta.two", p.Metadata.ID)
	}

	// Duplicate ID should fail
	if err := os.WriteFile(filepath.Join(dir, "dup.md"), []byte(a), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := opf.LoadCollection(dir); err == nil {
		t.Fatal("expected duplicate prompt id error")
	}
}
