// SPDX-License-Identifier: MIT OR Apache-2.0

// Package fleet is the orchestrator's link to edge nodes: pushing Crop Profiles and tracking
// what each node reports (docs/mqtt-topics.md).
package fleet

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"time"

	"tpt-agripod-wire"
)

// Publisher sends one payload on a topic at QoS 1. The MQTT adapter satisfies it.
type Publisher interface {
	Publish(topic string, payload []byte) error
}

// Config tunes profile pushes.
type Config struct {
	// Site is the site id every message carries.
	Site string
	// AckTimeout is how long one attempt waits for the ack before the push is sent again.
	AckTimeout time.Duration
	// MaxAttempts is how many times the push is sent before giving up.
	MaxAttempts int
}

// RejectedError is returned when a node refuses a profile. The node keeps its previous profile.
type RejectedError struct {
	Node    string
	Profile string
	Errors  []wire.AckError
}

func (e *RejectedError) Error() string {
	msg := fmt.Sprintf("fleet: node %s rejected profile %s", e.Node, e.Profile)
	if len(e.Errors) > 0 {
		msg += ": " + e.Errors[0].Path + ": " + e.Errors[0].Message
	}
	return msg
}

// ErrNoAck is returned when no ack arrives after every attempt.
var ErrNoAck = errors.New("fleet: no profile ack after all attempts")

// Applied is the profile a node last acknowledged as applied.
type Applied struct {
	ProfileID      string
	ProfileVersion string
	At             time.Time
}

// Pusher sends profiles and matches acks to them.
type Pusher struct {
	cfg Config
	pub Publisher

	mu      sync.Mutex
	pending map[string]chan wire.ProfileAck // msg_id -> waiter
	applied map[string]Applied              // node -> last applied
	unknown int                             // acks that matched nothing
}

// NewPusher validates cfg and returns a pusher.
func NewPusher(cfg Config, pub Publisher) (*Pusher, error) {
	if _, err := wire.Topic(cfg.Site, "probe", wire.KindStatus); err != nil {
		return nil, fmt.Errorf("fleet: site: %w", err)
	}
	if cfg.AckTimeout <= 0 || cfg.MaxAttempts < 1 {
		return nil, errors.New("fleet: ack timeout must be positive and attempts at least 1")
	}
	return &Pusher{
		cfg:     cfg,
		pub:     pub,
		pending: map[string]chan wire.ProfileAck{},
		applied: map[string]Applied{},
	}, nil
}

// Push sends profile to node and waits for its ack. Each attempt re-sends the same msg_id, so the
// edge can treat a repeat as the same request. The call returns nil only when the node applied the
// profile, and an error describing the rejection otherwise.
func (p *Pusher) Push(ctx context.Context, node, profileID, profileVersion string, profile json.RawMessage) error {
	msgID, err := newUUID()
	if err != nil {
		return err
	}
	msg := wire.ProfilePush{
		Envelope: wire.Envelope{MsgID: msgID, TS: time.Now().UTC(), SiteID: p.cfg.Site, NodeID: node},
		Profile:  profile,
	}
	if err := msg.Validate(); err != nil {
		return err
	}
	topic, err := wire.Topic(p.cfg.Site, node, wire.KindProfilePush)
	if err != nil {
		return err
	}
	payload, err := json.Marshal(msg)
	if err != nil {
		return fmt.Errorf("fleet: encode push: %w", err)
	}

	waiter := make(chan wire.ProfileAck, 1)
	p.mu.Lock()
	p.pending[msgID] = waiter
	p.mu.Unlock()
	defer func() {
		p.mu.Lock()
		delete(p.pending, msgID)
		p.mu.Unlock()
	}()

	for attempt := 1; attempt <= p.cfg.MaxAttempts; attempt++ {
		if err := p.pub.Publish(topic, payload); err != nil {
			return fmt.Errorf("fleet: publish push: %w", err)
		}
		timer := time.NewTimer(p.cfg.AckTimeout)
		select {
		case ack := <-waiter:
			timer.Stop()
			return p.finish(node, profileID, profileVersion, ack)
		case <-timer.C:
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		}
	}
	return fmt.Errorf("%w (node %s, profile %s %s)", ErrNoAck, node, profileID, profileVersion)
}

func (p *Pusher) finish(node, profileID, profileVersion string, ack wire.ProfileAck) error {
	// A rejection always surfaces as a rejection. A node that could not parse the profile may not
	// be able to echo its id, so the id is only checked for an applied profile.
	if ack.Status == wire.ProfileRejected {
		return &RejectedError{Node: node, Profile: profileID + " " + profileVersion, Errors: ack.Errors}
	}
	if ack.ProfileID != profileID || ack.ProfileVersion != profileVersion {
		return fmt.Errorf("fleet: node %s applied %s %s, expected %s %s",
			node, ack.ProfileID, ack.ProfileVersion, profileID, profileVersion)
	}
	p.mu.Lock()
	p.applied[node] = Applied{ProfileID: profileID, ProfileVersion: profileVersion, At: time.Now().UTC()}
	p.mu.Unlock()
	return nil
}

// HandleAck processes one message from a profile/ack topic. Acks that match no push in flight are
// counted and ignored: they are late duplicates, or from another orchestrator.
func (p *Pusher) HandleAck(topic string, payload []byte) error {
	_, node, kind, err := wire.ParseTopic(topic)
	if err != nil || kind != wire.KindProfileAck {
		return fmt.Errorf("fleet: not a profile ack topic: %q", topic)
	}
	ack, err := wire.Decode[wire.ProfileAck](payload)
	if err != nil {
		return err
	}
	if ack.NodeID != node {
		return fmt.Errorf("fleet: ack topic node %q does not match payload node %q", node, ack.NodeID)
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	waiter, ok := p.pending[ack.AckOf]
	if !ok {
		p.unknown++
		return nil
	}
	select {
	case waiter <- ack:
	default: // a previous ack already answered this push
	}
	return nil
}

// Applied returns the profile a node last acknowledged, and whether there is one.
func (p *Pusher) Applied(node string) (Applied, bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	a, ok := p.applied[node]
	return a, ok
}

// UnmatchedAcks is how many acks matched no push in flight.
func (p *Pusher) UnmatchedAcks() int {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.unknown
}

// newUUID returns a random version 4 UUID in canonical form.
func newUUID() (string, error) {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", fmt.Errorf("fleet: random id: %w", err)
	}
	b[6] = (b[6] & 0x0f) | 0x40
	b[8] = (b[8] & 0x3f) | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16]), nil
}
