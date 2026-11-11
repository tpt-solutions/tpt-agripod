// SPDX-License-Identifier: MIT OR Apache-2.0

// Package wire holds the MQTT message shapes and topic names of docs/mqtt-topics.md, shared by
// the orchestrator and the telemetry service. Each message carries the common envelope. Decoding
// is permissive about extra fields, so a newer edge can add fields. Validation is strict about
// the fields the topic map defines.
package wire

import (
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"regexp"
	"time"
)

// Envelope is the common header on every message.
type Envelope struct {
	MsgID  string    `json:"msg_id"`
	TS     time.Time `json:"ts"`
	SiteID string    `json:"site_id"`
	NodeID string    `json:"node_id"`
}

// Sample is one reading inside a telemetry batch.
type Sample struct {
	Channel  string    `json:"channel"`
	DeviceID string    `json:"device_id,omitempty"`
	TS       time.Time `json:"ts"`
	Value    float64   `json:"value"`
	Unit     string    `json:"unit"`
	Quality  string    `json:"quality"`
}

// Telemetry is a batch of samples from one node. Seq is monotonic per node, so gaps are visible.
type Telemetry struct {
	Envelope
	Seq     uint64   `json:"seq"`
	Samples []Sample `json:"samples"`
}

// ProfilePush carries a Crop Profile to an edge node. The profile is kept raw here and validated
// by the consumer.
type ProfilePush struct {
	Envelope
	Profile json.RawMessage `json:"profile"`
}

// AckError points at one problem in a rejected profile.
type AckError struct {
	Path    string `json:"path"`
	Message string `json:"message"`
}

// ProfileAck is the edge's answer to a profile push.
type ProfileAck struct {
	Envelope
	AckOf          string     `json:"ack_of"`
	ProfileID      string     `json:"profile_id"`
	ProfileVersion string     `json:"profile_version"`
	Status         string     `json:"status"`
	Errors         []AckError `json:"errors,omitempty"`
}

// Command is an operator or state-machine command. TTLs bounds how late it may run.
type Command struct {
	Envelope
	TTLs       int     `json:"ttl_s"`
	ActuatorID string  `json:"actuator_id"`
	Action     string  `json:"action"`
	Value      float64 `json:"value"`
}

// CommandAck reports what the edge did with a command.
type CommandAck struct {
	Envelope
	AckOf  string `json:"ack_of"`
	Status string `json:"status"`
	Reason string `json:"reason,omitempty"`
}

// Status is the heartbeat and online state. The last-will message is also a Status.
type Status struct {
	Envelope
	State          string `json:"state"`
	ProfileID      string `json:"profile_id,omitempty"`
	ProfileVersion string `json:"profile_version,omitempty"`
	UptimeS        int64  `json:"uptime_s"`
}

// Quality values for samples.
const (
	QualityGood       = "good"
	QualityStale      = "stale"
	QualityOutOfRange = "out_of_range"
	QualityMissing    = "missing"
)

// Status values for profile acks.
const (
	ProfileApplied  = "applied"
	ProfileRejected = "rejected"
)

// Status values for command acks.
const (
	CommandExecuted = "executed"
	CommandRejected = "rejected"
	CommandExpired  = "expired"
)

// Node states.
const (
	StateOnline  = "online"
	StateSafe    = "safe_state"
	StateOffline = "offline"
	maxSamples   = 500
	maxAckErrors = 100
)

var (
	slugPattern = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{1,62}$`)
	uuidPattern = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)
	chanPattern = regexp.MustCompile(`^[a-z][a-z0-9_]{0,62}$`)
)

// Validate checks the envelope. Every message type calls it.
func (e Envelope) Validate() error {
	if !uuidPattern.MatchString(e.MsgID) {
		return fmt.Errorf("wire: msg_id %q is not a UUID", e.MsgID)
	}
	if err := checkTime("ts", e.TS); err != nil {
		return err
	}
	if !slugPattern.MatchString(e.SiteID) {
		return fmt.Errorf("wire: site_id %q is not a slug", e.SiteID)
	}
	if !slugPattern.MatchString(e.NodeID) {
		return fmt.Errorf("wire: node_id %q is not a slug", e.NodeID)
	}
	return nil
}

func checkTime(field string, t time.Time) error {
	if t.IsZero() {
		return fmt.Errorf("wire: %s is missing", field)
	}
	if t.Location() != time.UTC {
		return fmt.Errorf("wire: %s must be UTC (RFC 3339 with Z)", field)
	}
	return nil
}

// Validate checks a telemetry batch.
func (m Telemetry) Validate() error {
	if err := m.Envelope.Validate(); err != nil {
		return err
	}
	if n := len(m.Samples); n < 1 || n > maxSamples {
		return fmt.Errorf("wire: telemetry has %d samples; want 1 to %d", n, maxSamples)
	}
	for i, s := range m.Samples {
		if !chanPattern.MatchString(s.Channel) {
			return fmt.Errorf("wire: sample %d channel %q is not lower_snake_case", i, s.Channel)
		}
		if err := checkTime(fmt.Sprintf("sample %d ts", i), s.TS); err != nil {
			return err
		}
		if math.IsNaN(s.Value) || math.IsInf(s.Value, 0) {
			return fmt.Errorf("wire: sample %d value is not finite", i)
		}
		if s.Unit == "" {
			return fmt.Errorf("wire: sample %d has no unit", i)
		}
		switch s.Quality {
		case QualityGood, QualityStale, QualityOutOfRange, QualityMissing:
		default:
			return fmt.Errorf("wire: sample %d quality %q is not one of good, stale, out_of_range, missing", i, s.Quality)
		}
	}
	return nil
}

// Validate checks a profile push envelope. The profile itself is validated by its consumer.
func (m ProfilePush) Validate() error {
	if err := m.Envelope.Validate(); err != nil {
		return err
	}
	if len(m.Profile) == 0 {
		return errors.New("wire: profile/push has no profile")
	}
	return nil
}

// Validate checks a profile ack.
func (m ProfileAck) Validate() error {
	if err := m.Envelope.Validate(); err != nil {
		return err
	}
	if !uuidPattern.MatchString(m.AckOf) {
		return fmt.Errorf("wire: ack_of %q is not a UUID", m.AckOf)
	}
	switch m.Status {
	case ProfileApplied, ProfileRejected:
	default:
		return fmt.Errorf("wire: profile ack status %q is not applied or rejected", m.Status)
	}
	if len(m.Errors) > maxAckErrors {
		return fmt.Errorf("wire: profile ack lists %d errors; max %d", len(m.Errors), maxAckErrors)
	}
	if m.Status == ProfileRejected && len(m.Errors) == 0 {
		return errors.New("wire: a rejected profile ack must say why")
	}
	return nil
}

// Validate checks a command.
func (m Command) Validate() error {
	if err := m.Envelope.Validate(); err != nil {
		return err
	}
	if m.TTLs <= 0 {
		return fmt.Errorf("wire: command ttl_s %d must be positive", m.TTLs)
	}
	if !slugPattern.MatchString(m.ActuatorID) {
		return fmt.Errorf("wire: actuator_id %q is not a slug", m.ActuatorID)
	}
	if m.Action == "" {
		return errors.New("wire: command has no action")
	}
	if math.IsNaN(m.Value) || math.IsInf(m.Value, 0) {
		return errors.New("wire: command value is not finite")
	}
	return nil
}

// Validate checks a command ack.
func (m CommandAck) Validate() error {
	if err := m.Envelope.Validate(); err != nil {
		return err
	}
	if !uuidPattern.MatchString(m.AckOf) {
		return fmt.Errorf("wire: ack_of %q is not a UUID", m.AckOf)
	}
	switch m.Status {
	case CommandExecuted, CommandRejected, CommandExpired:
	default:
		return fmt.Errorf("wire: command ack status %q is not executed, rejected or expired", m.Status)
	}
	return nil
}

// Validate checks a status message.
func (m Status) Validate() error {
	if err := m.Envelope.Validate(); err != nil {
		return err
	}
	switch m.State {
	case StateOnline, StateSafe, StateOffline:
	default:
		return fmt.Errorf("wire: status state %q is not online, safe_state or offline", m.State)
	}
	if m.UptimeS < 0 {
		return errors.New("wire: uptime_s is negative")
	}
	return nil
}

// Decode unmarshals JSON into v and then validates it with Validate, if v implements it.
func Decode[T interface{ Validate() error }](data []byte) (T, error) {
	var v T
	if err := json.Unmarshal(data, &v); err != nil {
		return v, fmt.Errorf("wire: decode: %w", err)
	}
	if err := v.Validate(); err != nil {
		return v, err
	}
	return v, nil
}
