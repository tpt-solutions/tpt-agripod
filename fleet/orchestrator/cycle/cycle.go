// SPDX-License-Identifier: MIT OR Apache-2.0

// Package cycle is the grow-cycle state machine (spec section 4.3).
//
// A cycle is an ordered list of stages with durations in days. Stage time is active time: it
// accrues only while the cycle is not held. Every transition, hold, resume and override is written
// to an audit journal, and an override must carry a reason.
package cycle

import (
	"errors"
	"fmt"
	"time"
)

// Stage is one part of a grow cycle.
type Stage struct {
	Name string `json:"name"`
	Days int    `json:"days"`
}

// Template is the stage list a cycle follows. Durations are placeholders until a crop profile
// carries them (the profile schema has no stage list yet).
type Template struct {
	Stages []Stage `json:"stages"`
}

// Validate checks that the template is usable: at least one stage, positive durations, unique names.
func (t Template) Validate() error {
	if len(t.Stages) == 0 {
		return errors.New("cycle: template has no stages")
	}
	seen := map[string]bool{}
	for i, s := range t.Stages {
		if s.Name == "" {
			return fmt.Errorf("cycle: stage %d has no name", i)
		}
		if s.Days <= 0 {
			return fmt.Errorf("cycle: stage %q has %d days; must be positive", s.Name, s.Days)
		}
		if seen[s.Name] {
			return fmt.Errorf("cycle: stage name %q is repeated", s.Name)
		}
		seen[s.Name] = true
	}
	return nil
}

// TotalDays is the sum of all stage durations.
func (t Template) TotalDays() int {
	total := 0
	for _, s := range t.Stages {
		total += s.Days
	}
	return total
}

// Entry is one line of the audit journal.
type Entry struct {
	At     time.Time `json:"at"`
	Kind   string    `json:"kind"`
	Detail string    `json:"detail"`
}

// State is the persisted state of one grow cycle. Exported fields are the on-disk format.
type State struct {
	ProfileID string        `json:"profile_id"`
	Template  Template      `json:"template"`
	Index     int           `json:"stage_index"`
	Elapsed   time.Duration `json:"stage_elapsed_ns"`
	Held      bool          `json:"held"`
	Done      bool          `json:"done"`
	LastTick  time.Time     `json:"last_tick"`
	Journal   []Entry       `json:"journal"`
}

// New starts a cycle at its first stage at time now.
func New(profileID string, t Template, now time.Time) (*State, error) {
	if profileID == "" {
		return nil, errors.New("cycle: profile id is empty")
	}
	if err := t.Validate(); err != nil {
		return nil, err
	}
	s := &State{ProfileID: profileID, Template: t, LastTick: now.UTC()}
	s.log(now, "start", fmt.Sprintf("profile %s, stage %q", profileID, t.Stages[0].Name))
	return s, nil
}

// StageName is the current stage, or "" once the cycle is done.
func (s *State) StageName() string {
	if s.Done || s.Index >= len(s.Template.Stages) {
		return ""
	}
	return s.Template.Stages[s.Index].Name
}

// Progress is the fraction of the current stage completed, 0 to 1.
func (s *State) Progress() float64 {
	if s.Done {
		return 1
	}
	dur := time.Duration(s.Template.Stages[s.Index].Days) * 24 * time.Hour
	return float64(s.Elapsed) / float64(dur)
}

// Tick accrues active time up to now and moves through as many stages as the time covers.
// Time that passes while held does not count. Calling Tick with a time before the last tick
// is an error, so a clock step backwards cannot shorten a stage.
func (s *State) Tick(now time.Time) error {
	if err := s.accrue(now); err != nil {
		return err
	}
	for !s.Done {
		dur := time.Duration(s.Template.Stages[s.Index].Days) * 24 * time.Hour
		if s.Elapsed < dur {
			break
		}
		finished := s.Template.Stages[s.Index].Name
		s.Elapsed -= dur
		s.Index++
		if s.Index == len(s.Template.Stages) {
			s.Done = true
			s.Elapsed = 0
			s.log(now, "complete", fmt.Sprintf("last stage %q finished", finished))
			break
		}
		s.log(now, "advance", fmt.Sprintf("%q finished, now %q", finished, s.Template.Stages[s.Index].Name))
	}
	return nil
}

// Hold pauses stage time. The reason is recorded.
func (s *State) Hold(now time.Time, reason string) error {
	if reason == "" {
		return errors.New("cycle: hold needs a reason")
	}
	if s.Done {
		return errors.New("cycle: cycle is complete")
	}
	if err := s.accrue(now); err != nil {
		return err
	}
	if s.Held {
		return errors.New("cycle: already held")
	}
	s.Held = true
	s.log(now, "hold", reason)
	return nil
}

// Resume restarts stage time after a hold.
func (s *State) Resume(now time.Time) error {
	if err := s.accrue(now); err != nil {
		return err
	}
	if !s.Held {
		return errors.New("cycle: not held")
	}
	s.Held = false
	s.log(now, "resume", "")
	return nil
}

// Override jumps to the named stage, which may be earlier or later. The reason is required,
// and the stage clock restarts at zero.
func (s *State) Override(now time.Time, stage, reason string) error {
	if reason == "" {
		return errors.New("cycle: override needs a reason")
	}
	if err := s.accrue(now); err != nil {
		return err
	}
	idx := -1
	for i, st := range s.Template.Stages {
		if st.Name == stage {
			idx = i
		}
	}
	if idx < 0 {
		return fmt.Errorf("cycle: no stage named %q", stage)
	}
	from := s.StageName()
	s.Index = idx
	s.Elapsed = 0
	s.Done = false
	s.log(now, "override", fmt.Sprintf("%q -> %q: %s", from, stage, reason))
	return nil
}

func (s *State) accrue(now time.Time) error {
	now = now.UTC()
	if now.Before(s.LastTick) {
		return fmt.Errorf("cycle: time went backwards (%s before %s)", now.Format(time.RFC3339), s.LastTick.Format(time.RFC3339))
	}
	if !s.Held && !s.Done {
		s.Elapsed += now.Sub(s.LastTick)
	}
	s.LastTick = now
	return nil
}

func (s *State) log(at time.Time, kind, detail string) {
	s.Journal = append(s.Journal, Entry{At: at.UTC(), Kind: kind, Detail: detail})
}
