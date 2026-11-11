// SPDX-License-Identifier: MIT OR Apache-2.0

// Command tpt-agripod-orchestrator is the fleet state machine that manages the grow cycle and
// pushes Crop Profiles to edge nodes (spec section 4.3).
//
// The push command sends a profile to an edge node over MQTT (ADR 0007) and waits for the ack.
// The other commands validate profiles and run grow cycles on disk.
//
//	orchestrator validate -profile FILE
//	orchestrator push     -broker HOST:PORT -site SITE -node NODE -profile FILE [-timeout D] [-attempts N]
//	orchestrator init     -profile FILE -template FILE -state FILE [-now RFC3339]
//	orchestrator tick     -state FILE [-now RFC3339]
//	orchestrator hold     -state FILE -reason TEXT [-now RFC3339]
//	orchestrator resume   -state FILE [-now RFC3339]
//	orchestrator override -state FILE -stage NAME -reason TEXT [-now RFC3339]
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"time"

	"tpt-agripod-orchestrator/cycle"
	"tpt-agripod-orchestrator/fleet"
	"tpt-agripod-orchestrator/profile"
	"tpt-agripod-wire"
)

const version = "0.1.0"

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintf(os.Stderr, "tpt-agripod-orchestrator %s\nusage: orchestrator {validate|push|init|tick|hold|resume|override} [flags]\n", version)
		os.Exit(2)
	}
	if err := run(os.Args[1], os.Args[2:]); err != nil {
		fmt.Fprintf(os.Stderr, "tpt-agripod-orchestrator: %v\n", err)
		os.Exit(1)
	}
}

func run(cmd string, args []string) error {
	if cmd == "push" {
		return pushCmd(args)
	}
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

// loadTemplate reads a stage template. A file marked "_status": "example" is refused: its
// durations are illustrative, and a cycle must be built from the agreed crop schedule.
func loadTemplate(path string) (cycle.Template, error) {
	var tpl cycle.Template
	var file struct {
		Status string `json:"_status"`
		cycle.Template
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return tpl, err
	}
	if err := json.Unmarshal(data, &file); err != nil {
		return tpl, fmt.Errorf("template %s: %w", path, err)
	}
	if file.Status == "example" {
		return tpl, fmt.Errorf("template %s is the example template; copy it and set the agreed durations (remove _status)", path)
	}
	tpl = file.Template
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

// pushCmd validates a profile locally, sends it to one node, and waits for the node's ack. A
// profile that fails local validation is never sent.
func pushCmd(args []string) error {
	fs := flag.NewFlagSet("push", flag.ContinueOnError)
	broker := fs.String("broker", "", "MQTT broker, host:port")
	site := fs.String("site", "", "site id")
	node := fs.String("node", "", "node id")
	profilePath := fs.String("profile", "", "crop profile JSON")
	timeout := fs.Duration("timeout", 10*time.Second, "how long to wait for each ack")
	attempts := fs.Int("attempts", 3, "how many times to send the push")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *broker == "" || *site == "" || *node == "" || *profilePath == "" {
		return fmt.Errorf("push needs -broker, -site, -node and -profile")
	}
	p, err := profile.Load(*profilePath)
	if err != nil {
		return fmt.Errorf("refusing to send: %w", err)
	}
	raw, err := os.ReadFile(*profilePath)
	if err != nil {
		return err
	}

	link, err := fleet.Connect(*broker, fmt.Sprintf("orchestrator-push-%d", os.Getpid()), 30, func(err error) {
		fmt.Fprintf(os.Stderr, "mqtt: %v\n", err)
	})
	if err != nil {
		return err
	}
	defer link.Close()

	pusher, err := fleet.NewPusher(fleet.Config{Site: *site, AckTimeout: *timeout, MaxAttempts: *attempts}, link)
	if err != nil {
		return err
	}
	// The subscription is in place before the push is sent, so the ack cannot be missed.
	if err := link.Subscribe(wire.SubscribeProfileAck, func(topic, payload []byte) {
		_ = pusher.HandleAck(string(topic), payload)
	}); err != nil {
		return err
	}
	if err := pusher.Push(context.Background(), *node, p.ProfileID, p.ProfileVersion, raw); err != nil {
		return err
	}
	fmt.Printf("applied: node %s, profile %s %s\n", *node, p.ProfileID, p.ProfileVersion)
	return nil
}
