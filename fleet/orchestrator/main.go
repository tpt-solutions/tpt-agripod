// SPDX-License-Identifier: MIT OR Apache-2.0

// Command tpt-agripod-orchestrator is the fleet state machine that manages the grow cycle and
// pushes Crop Profiles to edge nodes (spec section 4.3).
//
// Pushing profiles needs the MQTT client (ADR 0007), which is not wired in yet. This command
// validates profiles and runs grow cycles on disk.
//
//	orchestrator validate -profile FILE
//	orchestrator init     -profile FILE -template FILE -state FILE [-now RFC3339]
//	orchestrator tick     -state FILE [-now RFC3339]
//	orchestrator hold     -state FILE -reason TEXT [-now RFC3339]
//	orchestrator resume   -state FILE [-now RFC3339]
//	orchestrator override -state FILE -stage NAME -reason TEXT [-now RFC3339]
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"time"

	"tpt-agripod-orchestrator/cycle"
	"tpt-agripod-orchestrator/profile"
)

const version = "0.1.0"

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintf(os.Stderr, "tpt-agripod-orchestrator %s\nusage: orchestrator {validate|init|tick|hold|resume|override} [flags]\n", version)
		os.Exit(2)
	}
	if err := run(os.Args[1], os.Args[2:]); err != nil {
		fmt.Fprintf(os.Stderr, "tpt-agripod-orchestrator: %v\n", err)
		os.Exit(1)
	}
}

func run(cmd string, args []string) error {
	fs := flag.NewFlagSet(cmd, flag.ContinueOnError)
	profilePath := fs.String("profile", "", "crop profile JSON")
	templatePath := fs.String("template", "", "cycle template JSON (stage list)")
	statePath := fs.String("state", "", "cycle state file")
	reason := fs.String("reason", "", "reason for hold or override")
	stage := fs.String("stage", "", "stage name for override")
	nowFlag := fs.String("now", "", "time of the operation, RFC 3339 UTC (default: current time)")
	if err := fs.Parse(args); err != nil {
		return err
	}
	now, err := parseNow(*nowFlag)
	if err != nil {
		return err
	}

	switch cmd {
	case "validate":
		if *profilePath == "" {
			return fmt.Errorf("validate needs -profile")
		}
		p, err := profile.Load(*profilePath)
		if err != nil {
			return err
		}
		fmt.Printf("ok: %s %s (%d controls)\n", p.ProfileID, p.ProfileVersion, len(p.Controls))
		return nil

	case "init":
		if *profilePath == "" || *templatePath == "" || *statePath == "" {
			return fmt.Errorf("init needs -profile, -template and -state")
		}
		p, err := profile.Load(*profilePath)
		if err != nil {
			return err
		}
		tpl, err := loadTemplate(*templatePath)
		if err != nil {
			return err
		}
		s, err := cycle.New(p.ProfileID, tpl, now)
		if err != nil {
			return err
		}
		return save(*statePath, s)
	}

	if *statePath == "" {
		return fmt.Errorf("%s needs -state", cmd)
	}
	s, err := cycle.Load(*statePath)
	if err != nil {
		return err
	}
	switch cmd {
	case "tick":
		err = s.Tick(now)
	case "hold":
		err = s.Hold(now, *reason)
	case "resume":
		err = s.Resume(now)
	case "override":
		err = s.Override(now, *stage, *reason)
	default:
		return fmt.Errorf("unknown command %q", cmd)
	}
	if err != nil {
		return err
	}
	if err := save(*statePath, s); err != nil {
		return err
	}
	status(s)
	return nil
}

func parseNow(s string) (time.Time, error) {
	if s == "" {
		return time.Now().UTC(), nil
	}
	t, err := time.Parse(time.RFC3339, s)
	if err != nil {
		return time.Time{}, fmt.Errorf("-now: %w", err)
	}
	return t.UTC(), nil
}

func loadTemplate(path string) (cycle.Template, error) {
	var tpl cycle.Template
	data, err := os.ReadFile(path)
	if err != nil {
		return tpl, err
	}
	if err := json.Unmarshal(data, &tpl); err != nil {
		return tpl, fmt.Errorf("template %s: %w", path, err)
	}
	return tpl, tpl.Validate()
}

func save(path string, s *cycle.State) error {
	return cycle.Save(path, s)
}

func status(s *cycle.State) {
	if s.Done {
		fmt.Printf("%s: complete\n", s.ProfileID)
		return
	}
	state := "active"
	if s.Held {
		state = "held"
	}
	fmt.Printf("%s: stage %q (%.0f%%), %s\n", s.ProfileID, s.StageName(), 100*s.Progress(), state)
}
