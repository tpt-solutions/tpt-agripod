// SPDX-License-Identifier: MIT OR Apache-2.0

package profile

import (
	"os"
	"regexp"
	"strings"
	"testing"
)

const examplePath = "../../../profiles/examples/example-plant-v0.json"

func example(t *testing.T) string {
	t.Helper()
	data, err := os.ReadFile(examplePath)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

func TestShippedExampleIsValid(t *testing.T) {
	p, err := Load(examplePath)
	if err != nil {
		t.Fatalf("example profile rejected: %v", err)
	}
	if p.ProfileID != "example-plant" || p.Controls["pump_duty"].Value == nil {
		t.Fatalf("unexpected parse: %+v", p)
	}
}

// mutate applies one textual edit to the example and expects Parse to fail with a message
// containing want.
func mutate(t *testing.T, old, new, want string) {
	t.Helper()
	orig := example(t)
	text := strings.Replace(orig, old, new, 1)
	if text == orig {
		t.Fatalf("test setup: %q not found in example", old)
	}
	_, err := Parse([]byte(text))
	if err == nil {
		t.Fatalf("accepted %q -> %q", old, new)
	}
	if !strings.Contains(err.Error(), want) {
		t.Fatalf("error %q does not mention %q", err, want)
	}
}

func TestSchemaRules(t *testing.T) {
	mutate(t, `"kind": "plant"`, `"kind": "culture"`, "reserved")
	mutate(t, `"kind": "plant"`, `"kind": "animal"`, `must be "plant"`)
	mutate(t, `"profile_id": "example-plant"`, `"profile_id": "Example_Plant"`, "profile_id")
	mutate(t, `"profile_version": "0.1.0"`, `"profile_version": "0.1"`, "MAJOR.MINOR.PATCH")
	mutate(t, `"cartridge_types": ["C"]`, `"cartridge_types": ["D"]`, "must be A, B or C")
	mutate(t, `"cartridge_types": ["C"]`, `"cartridge_types": []`, "at least one")
	mutate(t, `"schema_version": "0"`, `"schema_version": "1"`, "schema_version")
}

func TestUnknownFieldsAreRejected(t *testing.T) {
	mutate(t, `"kind": "plant",`, `"kind": "plant", "surprise": true,`, "surprise")
	mutate(t, `"interval_min": 15,`, `"interval_min": 15, "extra": 1,`, "extra")
}

func TestTargetRanges(t *testing.T) {
	mutate(t, `"vpd_kpa": { "min": 0.8, "max": 1.0 }`, `"vpd_kpa": { "min": 1.2, "max": 1.0 }`, "vpd_kpa")
	mutate(t, `"vpd_kpa": { "min": 0.8, "max": 1.0 }`, `"vpd_kpa": { "min": 0.8 }`, "vpd_kpa")
	mutate(t, `"ph": { "min": 5.8, "max": 6.2 }`, `"ph": { "min": 5.8, "max": 15 }`, "ph")
	mutate(t, `"dli_mol_m2_day": 14`, `"dli_mol_m2_day": 120`, "dli_mol_m2_day")
	mutate(t, `"photoperiod_h": 16`, `"photoperiod_h": 30`, "photoperiod_h")
}

func TestControlsAreCheckedAgainstTheEdge(t *testing.T) {
	mutate(t, `"value": 0.10,`, `"value": 0.30,`, "above max")
	mutate(t, `"max": 0.15,`, `"max": 0.20,`, "exceeds the edge pump contract")
	mutate(t, `"pump_duty": {`, `"pump_speed": {`, "unknown to the edge")
	mutate(t, `"value": 0.9,`, `"value": 0.5,`, "below min")
	mutate(t, `"unit": "frac",
      "min": 0,`, `"unit": "",
      "min": 0,`, "needs a unit")
}

// The pump limit in Go must match the edge contract. This test fails if either side moves.
func TestPumpLimitMatchesTheEdgeContract(t *testing.T) {
	data, err := os.ReadFile("../../../edge/contracts/pump.contract")
	if err != nil {
		t.Fatal(err)
	}
	re := regexp.MustCompile(`pump_duty_cycle_frac\s*<=\s*([0-9.]+)`)
	m := re.FindSubmatch(data)
	if m == nil {
		t.Fatal("pump contract has no pump_duty_cycle_frac <= limit")
	}
	if string(m[1]) != "0.15" || PumpDutyMax != 0.15 {
		t.Fatalf("edge contract limit %s, Go PumpDutyMax %g: keep them equal", m[1], PumpDutyMax)
	}
}
