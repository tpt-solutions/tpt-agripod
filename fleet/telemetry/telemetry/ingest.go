// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"context"
	"errors"
	"fmt"
	"sync"

	"tpt-agripod-wire"
)

// Submitter takes points into the pipeline. The Batcher satisfies it.
type Submitter interface {
	Submit(ctx context.Context, p Point) error
}

// Stats counts what the ingester has seen. It is a snapshot.
type Stats struct {
	Messages       int // telemetry messages received
	Accepted       int // messages processed (not duplicates, not rejected)
	Duplicates     int // messages whose msg_id was already processed
	Rejected       int // messages that failed decoding or validation, or came from a mismatched topic
	Points         int // good-quality samples submitted
	SkippedQuality int // samples dropped for quality stale, out_of_range or missing
	SeqGaps        int // sequence numbers missing from a node's stream
	SeqReorders    int // sequence numbers at or below the last seen for a node
}

// Ingester turns telemetry messages into points. It is independent of the MQTT transport, so its
// rules are tested without a broker.
type Ingester struct {
	out    Submitter
	ctx    context.Context
	mu     sync.Mutex
	stats  Stats
	seen   *idSet
	last   map[string]uint64 // node -> last seq
	hasSeq map[string]bool
}

// NewIngester returns an ingester that submits to out. dedupCapacity bounds how many msg_ids are
// remembered; older ids are forgotten, so a duplicate older than that window is processed again.
func NewIngester(ctx context.Context, out Submitter, dedupCapacity int) (*Ingester, error) {
	if dedupCapacity < 1 {
		return nil, errors.New("telemetry: dedup capacity must be positive")
	}
	return &Ingester{
		out:    out,
		ctx:    ctx,
		seen:   newIDSet(dedupCapacity),
		last:   map[string]uint64{},
		hasSeq: map[string]bool{},
	}, nil
}

// Stats returns a snapshot of the counters.
func (in *Ingester) Stats() Stats {
	in.mu.Lock()
	defer in.mu.Unlock()
	return in.stats
}

// Handle processes one telemetry message received on topic. Errors are returned for the caller to
// log. Rejected messages are counted, not retried, since retrying a malformed message cannot help.
//
// A message is acknowledged to the broker when it is received, not when its points are stored.
// Points lost in a crash are visible as sequence gaps, and recovering them is the edge's
// offline-buffer job.
func (in *Ingester) Handle(topic string, payload []byte) error {
	in.mu.Lock()
	defer in.mu.Unlock()
	in.stats.Messages++

	_, node, kind, err := wire.ParseTopic(topic)
	if err != nil || kind != wire.KindTelemetry {
		in.stats.Rejected++
		return fmt.Errorf("telemetry: not a telemetry topic: %q", topic)
	}
	m, err := wire.Decode[wire.Telemetry](payload)
	if err != nil {
		in.stats.Rejected++
		return err
	}
	if m.NodeID != node {
		in.stats.Rejected++
		return fmt.Errorf("telemetry: topic node %q does not match payload node %q", node, m.NodeID)
	}
	if in.seen.contains(m.MsgID) {
		in.stats.Duplicates++
		return nil
	}
	in.seen.add(m.MsgID)
	in.trackSeq(m.NodeID, m.Seq)

	for _, s := range m.Samples {
		if s.Quality != wire.QualityGood {
			in.stats.SkippedQuality++
			continue
		}
		nodeKey := m.NodeID
		if s.DeviceID != "" {
			// Device samples carry the device in the node key, so two devices reporting the same
			// channel at the same instant do not collide on the primary key.
			nodeKey = m.NodeID + "/" + s.DeviceID
		}
		p := Point{Node: nodeKey, Metric: s.Channel, TS: s.TS, Value: s.Value}
		if err := in.out.Submit(in.ctx, p); err != nil {
			return fmt.Errorf("telemetry: submit point: %w", err)
		}
		in.stats.Points++
	}
	in.stats.Accepted++
	return nil
}

// trackSeq counts gaps and reorders in a node's sequence numbers. A node restart that resets seq
// shows up as a reorder, which is a visible signal, not a silent one.
func (in *Ingester) trackSeq(node string, seq uint64) {
	if in.hasSeq[node] {
		prev := in.last[node]
		switch {
		case seq == prev+1:
		case seq > prev+1:
			in.stats.SeqGaps += int(seq - prev - 1)
		default:
			in.stats.SeqReorders++
		}
	}
	in.last[node] = seq
	in.hasSeq[node] = true
}

// idSet is a bounded set of message ids. When full, it forgets the oldest id.
type idSet struct {
	cap   int
	order []string
	set   map[string]struct{}
}

func newIDSet(capacity int) *idSet {
	return &idSet{cap: capacity, set: make(map[string]struct{}, capacity)}
}

func (s *idSet) contains(id string) bool {
	_, ok := s.set[id]
	return ok
}

func (s *idSet) add(id string) {
	if s.contains(id) {
		return
	}
	if len(s.order) == s.cap {
		delete(s.set, s.order[0])
		s.order = s.order[1:]
	}
	s.order = append(s.order, id)
	s.set[id] = struct{}{}
}
