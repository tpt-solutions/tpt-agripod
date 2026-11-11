// SPDX-License-Identifier: MIT OR Apache-2.0

package cycle

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

var start = time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)

func day(n int) time.Time { return start.Add(time.Duration(n) * 24 * time.Hour) }

func template() Template {
	return Template{Stages: []Stage{
		{Name: "germination", Days: 7},
		{Name: "vegetative", Days: 35},
		{Name: "maturity", Days: 24},
	}}
}

func mustNew(t *testing.T) *State {
	t.Helper()
	s, err := New("lettuce", template(), start)
	if err != nil {
		t.Fatal(err)
	}
	return s
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func TestStagesAdvanceByActiveTime(t *testing.T) {
	s := mustNew(t)
	must(t, s.Tick(day(6)))
	if s.StageName() != "germination" {
		t.Fatalf("day 6: stage %q", s.StageName())
	}
	must(t, s.Tick(day(7)))
	if s.StageName() != "vegetative" {
		t.Fatalf("day 7: stage %q", s.StageName())
	}
	must(t, s.Tick(day(42)))
	if s.StageName() != "maturity" {
		t.Fatalf("day 42: stage %q", s.StageName())
	}
	must(t, s.Tick(day(66)))
	if !s.Done || s.StageName() != "" {
		t.Fatalf("day 66: done=%v stage=%q", s.Done, s.StageName())
	}
	if s.Progress() != 1 {
		t.Fatalf("progress %v", s.Progress())
	}
}

func TestHoldPausesTheClock(t *testing.T) {
	s := mustNew(t)
	must(t, s.Hold(day(3), "pump service"))
	must(t, s.Tick(day(30))) // 27 days held: no progress
	if s.StageName() != "germination" {
		t.Fatalf("held cycle advanced to %q", s.StageName())
	}
	must(t, s.Resume(day(30)))
	must(t, s.Tick(day(34))) // 3 + 4 active days = 7
	if s.StageName() != "vegetative" {
		t.Fatalf("after resume: stage %q", s.StageName())
	}
}

func TestCarryOverBetweenStages(t *testing.T) {
	s := mustNew(t)
	must(t, s.Tick(day(10))) // 3 days into vegetative
	if s.StageName() != "vegetative" || s.Elapsed != 3*24*time.Hour {
		t.Fatalf("stage %q elapsed %v", s.StageName(), s.Elapsed)
	}
}

func TestOverrideNeedsAReasonAndIsJournalled(t *testing.T) {
	s := mustNew(t)
	if err := s.Override(day(1), "maturity", ""); err == nil {
		t.Fatal("override without reason accepted")
	}
	if err := s.Override(day(1), "nope", "x"); err == nil {
		t.Fatal("override to unknown stage accepted")
	}
	must(t, s.Override(day(1), "maturity", "early harvest test"))
	if s.StageName() != "maturity" || s.Elapsed != 0 {
		t.Fatalf("after override: %q %v", s.StageName(), s.Elapsed)
	}
	last := s.Journal[len(s.Journal)-1]
	if last.Kind != "override" || !strings.Contains(last.Detail, "early harvest test") {
		t.Fatalf("journal: %+v", last)
	}
}

func TestClockCannotGoBackwards(t *testing.T) {
	s := mustNew(t)
	must(t, s.Tick(day(5)))
	if err := s.Tick(day(4)); err == nil {
		t.Fatal("backwards tick accepted")
	}
	if err := s.Hold(day(4), "x"); err == nil {
		t.Fatal("backwards hold accepted")
	}
}

func TestHoldAndResumeRules(t *testing.T) {
	s := mustNew(t)
	if err := s.Hold(day(1), ""); err == nil {
		t.Error("hold without reason accepted")
	}
	must(t, s.Hold(day(1), "maintenance"))
	if err := s.Hold(day(2), "again"); err == nil {
		t.Error("double hold accepted")
	}
	must(t, s.Resume(day(2)))
	if err := s.Resume(day(3)); err == nil {
		t.Error("resume when not held accepted")
	}
}

func TestTemplateValidation(t *testing.T) {
	bad := map[string]Template{
		"empty":     {},
		"no name":   {Stages: []Stage{{Name: "", Days: 1}}},
		"zero days": {Stages: []Stage{{Name: "a", Days: 0}}},
		"repeated":  {Stages: []Stage{{Name: "a", Days: 1}, {Name: "a", Days: 2}}},
	}
	for name, tpl := range bad {
		if _, err := New("x", tpl, start); err == nil {
			t.Errorf("%s: accepted", name)
		}
	}
	if got := template().TotalDays(); got != 66 {
		t.Fatalf("total days %d", got)
	}
}

func TestSaveAndLoadRoundTrip(t *testing.T) {
	s := mustNew(t)
	must(t, s.Hold(day(2), "pump"))
	must(t, s.Resume(day(5)))
	must(t, s.Tick(day(12)))
	path := filepath.Join(t.TempDir(), "cycle.json")
	must(t, Save(path, s))
	got, err := Load(path)
	must(t, err)
	if got.ProfileID != s.ProfileID || got.Index != s.Index || got.Elapsed != s.Elapsed ||
		got.Held != s.Held || got.Done != s.Done || !got.LastTick.Equal(s.LastTick) ||
		len(got.Journal) != len(s.Journal) {
		t.Fatalf("round trip changed state:\n got %+v\nwant %+v", got, s)
	}
	// A second save replaces the first.
	must(t, s.Tick(day(20)))
	must(t, Save(path, s))
	got, err = Load(path)
	must(t, err)
	if got.Elapsed != s.Elapsed {
		t.Fatal("second save did not replace the first")
	}
	// No temp files are left behind.
	entries, _ := os.ReadDir(filepath.Dir(path))
	if len(entries) != 1 {
		t.Fatalf("expected only the state file, found %d entries", len(entries))
	}
}

func TestLoadRejectsCorruptState(t *testing.T) {
	path := filepath.Join(t.TempDir(), "cycle.json")
	must(t, os.WriteFile(path, []byte(`{"template":{"stages":[]},"stage_index":0}`), 0o600))
	if _, err := Load(path); err == nil {
		t.Fatal("empty template accepted")
	}
	must(t, os.WriteFile(path, []byte(`{"template":{"stages":[{"name":"a","days":1}]},"stage_index":9}`), 0o600))
	if _, err := Load(path); err == nil {
		t.Fatal("out-of-range index accepted")
	}
}
