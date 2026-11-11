// SPDX-License-Identifier: MIT OR Apache-2.0

package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestExampleTemplateIsRefused(t *testing.T) {
	_, err := loadTemplate("templates/example-120d.json")
	if err == nil || !strings.Contains(err.Error(), "example template") {
		t.Fatalf("example template was accepted: %v", err)
	}
}

func TestCompletedTemplateLoads(t *testing.T) {
	path := filepath.Join(t.TempDir(), "agreed.json")
	body := `{"stages":[{"name":"germination","days":7},{"name":"vegetative","days":35}]}`
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	tpl, err := loadTemplate(path)
	if err != nil || tpl.TotalDays() != 42 {
		t.Fatalf("completed template: %v, %+v", err, tpl)
	}
}
