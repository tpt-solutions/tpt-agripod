// SPDX-License-Identifier: MIT OR Apache-2.0

package fleet

import (
	"fmt"
	"sort"
	"sync"
	"time"

	"tpt-agripod-wire"
)

// NodeState is what the orchestrator knows about one node.
type NodeState struct {
	Node           string
	State          string
	ProfileID      string
	ProfileVersion string
	UptimeS        int64
	// LastSeen is when the orchestrator received the node's latest status, by its own clock.
	LastSeen time.Time
}

// Registry tracks node status from the status topic, including retained last-will messages.
type Registry struct {
	mu    sync.Mutex
	nodes map[string]NodeState
}

// NewRegistry returns an empty registry.
func NewRegistry() *Registry {
	return &Registry{nodes: map[string]NodeState{}}
}

// HandleStatus processes one message from a status topic. now is the orchestrator's receive time.
// A node that drops off without a clean disconnect is reported by its last-will message, which the
// broker publishes on the same topic.
func (r *Registry) HandleStatus(topic string, payload []byte, now time.Time) error {
	_, node, kind, err := wire.ParseTopic(topic)
	if err != nil || kind != wire.KindStatus {
		return fmt.Errorf("fleet: not a status topic: %q", topic)
	}
	st, err := wire.Decode[wire.Status](payload)
	if err != nil {
		return err
	}
	if st.NodeID != node {
		return fmt.Errorf("fleet: status topic node %q does not match payload node %q", node, st.NodeID)
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	r.nodes[node] = NodeState{
		Node:           node,
		State:          st.State,
		ProfileID:      st.ProfileID,
		ProfileVersion: st.ProfileVersion,
		UptimeS:        st.UptimeS,
		LastSeen:       now.UTC(),
	}
	return nil
}

// Get returns one node's state.
func (r *Registry) Get(node string) (NodeState, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	s, ok := r.nodes[node]
	return s, ok
}

// Snapshot returns every known node, sorted by name.
func (r *Registry) Snapshot() []NodeState {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]NodeState, 0, len(r.nodes))
	for _, s := range r.nodes {
		out = append(out, s)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Node < out[j].Node })
	return out
}

// Silent returns the nodes that have not reported within timeout of now and are not already
// marked offline. The caller decides what to do; the registry does not change their state, because
// a silent node is not necessarily a dead one.
func (r *Registry) Silent(now time.Time, timeout time.Duration) []string {
	r.mu.Lock()
	defer r.mu.Unlock()
	var out []string
	for _, s := range r.nodes {
		if s.State != wire.StateOffline && now.Sub(s.LastSeen) > timeout {
			out = append(out, s.Node)
		}
	}
	sort.Strings(out)
	return out
}
