// SPDX-License-Identifier: MIT OR Apache-2.0

package profile

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The shared vectors are read by this test and by the edge's Rust test. Both validators must
// agree on every case.
const vectorsDir = "../../../profiles/testvectors"

type vectorCase struct {
	File     string `json:"file"`
	Valid    bool   `json:"valid"`
	Mentions string `json:"mentions"`
}

func TestSharedVectors(t *testing.T) {
	data, err := os.ReadFile(filepath.Join(vectorsDir, "manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	var manifest struct {
		Cases []vectorCase `json:"cases"`
	}
	if err := json.Unmarshal(data, &manifest); err != nil {
		t.Fatal(err)
	}
	if len(manifest.Cases) == 0 {
		t.Fatal("manifest has no cases")
	}
	for _, c := range manifest.Cases {
		body, err := os.ReadFile(filepath.Join(vectorsDir, c.File))
		if err != nil {
			t.Fatalf("%s: %v", c.File, err)
		}
		_, err = Parse(body)
		if c.Valid && err != nil {
			t.Errorf("%s: valid case rejected: %v", c.File, err)
		}
		if !c.Valid {
			if err == nil {
				t.Errorf("%s: invalid case accepted", c.File)
			} else if c.Mentions != "" && !strings.Contains(err.Error(), c.Mentions) {
				t.Errorf("%s: rejection does not mention %q: %v", c.File, c.Mentions, err)
			}
		}
	}
}
