// SPDX-License-Identifier: MIT OR Apache-2.0

package fleet

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	mqtt "github.com/mochi-mqtt/server/v2"
	"github.com/mochi-mqtt/server/v2/hooks/auth"
	"github.com/mochi-mqtt/server/v2/listeners"
	gmqclient "github.com/yosssi/gmq/mqtt/client"

	"tpt-agripod-orchestrator/profile"
	"tpt-agripod-wire"
)

const exampleProfile = "../../../profiles/examples/example-plant-v0.json"

func startBroker(t *testing.T) string {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Skipf("cannot listen on localhost: %v", err)
	}
	addr := l.Addr().String()
	l.Close()
	server := mqtt.New(&mqtt.Options{})
	if err := server.AddHook(new(auth.AllowHook), nil); err != nil {
		t.Fatal(err)
	}
	if err := server.AddListener(listeners.NewTCP(listeners.Config{ID: "t1", Address: addr})); err != nil {
		t.Fatal(err)
	}
	go func() { _ = server.Serve() }()
	t.Cleanup(func() { _ = server.Close() })
	deadline := time.Now().Add(5 * time.Second)
	for {
		c, err := net.Dial("tcp", addr)
		if err == nil {
			c.Close()
			return addr
		}
		if time.Now().After(deadline) {
			t.Fatalf("broker did not start: %v", err)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

func loadProfile(t *testing.T) json.RawMessage {
	t.Helper()
	b, err := os.ReadFile(exampleProfile)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// fakeEdge stands in for an edge node. It validates pushes with the orchestrator's profile
// validator and answers with an ack. dropFirst makes it ignore the first push, to simulate a lost
// message.
type fakeEdge struct {
	t         *testing.T
	node      string
	cli       *gmqclient.Client
	dropFirst bool

	mu       sync.Mutex
	received []string // msg_ids of every push received
	dropped  bool
	silent   bool // ignore every push
}

func startFakeEdge(t *testing.T, addr, node string) *fakeEdge {
	t.Helper()
	e := &fakeEdge{t: t, node: node}
	e.cli = gmqclient.New(&gmqclient.Options{ErrorHandler: func(error) {}})
	if err := e.cli.Connect(&gmqclient.ConnectOptions{
		Network: "tcp", Address: addr, ClientID: []byte("edge-" + node), CleanSession: true, KeepAlive: 30,
	}); err != nil {
		t.Fatalf("edge connect: %v", err)
	}
	t.Cleanup(func() {
		_ = e.cli.Disconnect()
		e.cli.Terminate()
	})
	pushTopic, _ := wire.Topic("wgtn-bay-01", node, wire.KindProfilePush)
	if err := e.cli.Subscribe(&gmqclient.SubscribeOptions{SubReqs: []*gmqclient.SubReq{{
		TopicFilter: []byte(pushTopic), QoS: 1, Handler: gmqclient.MessageHandler(e.onPush),
	}}}); err != nil {
		t.Fatalf("edge subscribe: %v", err)
	}
	return e
}

func (e *fakeEdge) onPush(_, payload []byte) {
	push, err := wire.Decode[wire.ProfilePush](payload)
	if err != nil {
		e.t.Errorf("edge got an invalid push: %v", err)
		return
	}
	e.mu.Lock()
	e.received = append(e.received, push.MsgID)
	drop := e.silent || (e.dropFirst && !e.dropped)
	e.dropped = e.dropped || drop
	e.mu.Unlock()
	if drop {
		return
	}

	ack := wire.ProfileAck{
		Envelope: wire.Envelope{MsgID: mustUUID(e.t), TS: time.Now().UTC(), SiteID: "wgtn-bay-01", NodeID: e.node},
		AckOf:    push.MsgID,
	}
	p, perr := profile.Parse(push.Profile)
	if perr != nil {
		ack.Status = wire.ProfileRejected
		ack.ProfileID, ack.ProfileVersion = "unknown", "0.0.0"
		ack.Errors = []wire.AckError{{Path: "/", Message: perr.Error()}}
	} else {
		ack.Status = wire.ProfileApplied
		ack.ProfileID, ack.ProfileVersion = p.ProfileID, p.ProfileVersion
	}
	b, _ := json.Marshal(ack)
	topic, _ := wire.Topic("wgtn-bay-01", e.node, wire.KindProfileAck)
	if err := e.cli.Publish(&gmqclient.PublishOptions{QoS: 1, TopicName: []byte(topic), Message: b}); err != nil {
		e.t.Errorf("edge publish ack: %v", err)
	}
}

func (e *fakeEdge) pushIDs() []string {
	e.mu.Lock()
	defer e.mu.Unlock()
	return append([]string(nil), e.received...)
}

func mustUUID(t *testing.T) string {
	t.Helper()
	id, err := newUUID()
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func testPusher(t *testing.T, addr string, attempts int) (*Pusher, *Link) {
	t.Helper()
	link, err := Connect(addr, "orchestrator-test", 30, nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = link.Close() })
	p, err := NewPusher(Config{Site: "wgtn-bay-01", AckTimeout: 300 * time.Millisecond, MaxAttempts: attempts}, link)
	if err != nil {
		t.Fatal(err)
	}
	if err := link.Subscribe(wire.SubscribeProfileAck, func(topic, payload []byte) {
		_ = p.HandleAck(string(topic), payload)
	}); err != nil {
		t.Fatal(err)
	}
	time.Sleep(50 * time.Millisecond) // let the subscription land before the first push
	return p, link
}

func TestValidProfileIsAppliedOverTheBroker(t *testing.T) {
	addr := startBroker(t)
	startFakeEdge(t, addr, "nuc-01")
	p, _ := testPusher(t, addr, 3)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := p.Push(ctx, "nuc-01", "example-plant", "0.1.0", loadProfile(t)); err != nil {
		t.Fatalf("push: %v", err)
	}
	a, ok := p.Applied("nuc-01")
	if !ok || a.ProfileID != "example-plant" || a.ProfileVersion != "0.1.0" {
		t.Fatalf("applied record %+v, ok=%v", a, ok)
	}
}

func TestInvalidProfileIsRejectedAndNotRecorded(t *testing.T) {
	addr := startBroker(t)
	startFakeEdge(t, addr, "nuc-01")
	p, _ := testPusher(t, addr, 3)

	bad := strings.Replace(string(loadProfile(t)), `"max": 0.15,`, `"max": 0.30,`, 1)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	err := p.Push(ctx, "nuc-01", "example-plant", "0.1.0", json.RawMessage(bad))
	var rej *RejectedError
	if !errors.As(err, &rej) {
		t.Fatalf("expected a rejection, got %v", err)
	}
	if !strings.Contains(rej.Error(), "pump_duty") {
		t.Fatalf("rejection does not say why: %v", rej)
	}
	if _, ok := p.Applied("nuc-01"); ok {
		t.Fatal("a rejected profile was recorded as applied")
	}
}

func TestLostPushIsRetriedWithTheSameMsgID(t *testing.T) {
	addr := startBroker(t)
	edge := startFakeEdge(t, addr, "nuc-01")
	edge.dropFirst = true
	p, _ := testPusher(t, addr, 3)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := p.Push(ctx, "nuc-01", "example-plant", "0.1.0", loadProfile(t)); err != nil {
		t.Fatalf("push after a lost message: %v", err)
	}
	ids := edge.pushIDs()
	if len(ids) != 2 || ids[0] != ids[1] {
		t.Fatalf("retries must reuse the msg_id so the edge can dedup; got %v", ids)
	}
}

func TestNodeThatNeverAcksFailsAfterAllAttempts(t *testing.T) {
	addr := startBroker(t)
	edge := startFakeEdge(t, addr, "nuc-01")
	edge.silent = true
	p, _ := testPusher(t, addr, 2)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	err := p.Push(ctx, "nuc-01", "example-plant", "0.1.0", loadProfile(t))
	if !errors.Is(err, ErrNoAck) {
		t.Fatalf("expected ErrNoAck, got %v", err)
	}
	if n := len(edge.pushIDs()); n != 2 {
		t.Fatalf("sent %d times, want 2", n)
	}
}

func TestLastWillMarksANodeOfflineAfterACrash(t *testing.T) {
	addr := startBroker(t)
	statusTopic, _ := wire.Topic("wgtn-bay-01", "nuc-02", wire.KindStatus)
	will, _ := json.Marshal(wire.Status{
		Envelope: wire.Envelope{MsgID: mustUUID(t), TS: time.Now().UTC(), SiteID: "wgtn-bay-01", NodeID: "nuc-02"},
		State:    wire.StateOffline,
	})

	// The node connects with a retained last-will, announces itself online, then crashes.
	node := gmqclient.New(&gmqclient.Options{ErrorHandler: func(error) {}})
	if err := node.Connect(&gmqclient.ConnectOptions{
		Network: "tcp", Address: addr, ClientID: []byte("nuc-02"), CleanSession: true,
		// A short keep-alive: Terminate stops pings without closing the socket, so the broker
		// declares the node dead when the keep-alive expires, as it would after a real crash.
		KeepAlive: 1,
		WillTopic: []byte(statusTopic), WillMessage: will, WillQoS: 1, WillRetain: true,
	}); err != nil {
		t.Fatal(err)
	}
	online, _ := json.Marshal(wire.Status{
		Envelope: wire.Envelope{MsgID: mustUUID(t), TS: time.Now().UTC(), SiteID: "wgtn-bay-01", NodeID: "nuc-02"},
		State:    wire.StateOnline, ProfileID: "example-plant", ProfileVersion: "0.1.0", UptimeS: 10,
	})
	if err := node.Publish(&gmqclient.PublishOptions{QoS: 1, Retain: true, TopicName: []byte(statusTopic), Message: online}); err != nil {
		t.Fatal(err)
	}
	time.Sleep(50 * time.Millisecond)

	reg := NewRegistry()
	link, err := Connect(addr, "orchestrator-registry", 30, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer link.Close()
	if err := link.Subscribe(wire.SubscribeStatus, func(topic, payload []byte) {
		_ = reg.HandleStatus(string(topic), payload, time.Now())
	}); err != nil {
		t.Fatal(err)
	}

	waitFor(t, func() bool { s, ok := reg.Get("nuc-02"); return ok && s.State == wire.StateOnline })

	node.Terminate() // no DISCONNECT: the broker must publish the will

	waitFor(t, func() bool { s, ok := reg.Get("nuc-02"); return ok && s.State == wire.StateOffline })
}

func waitFor(t *testing.T, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for !cond() {
		if time.Now().After(deadline) {
			t.Fatal("condition not reached in time")
		}
		time.Sleep(10 * time.Millisecond)
	}
}

func TestAckFromTheWrongNodeIsRejected(t *testing.T) {
	p, err := NewPusher(Config{Site: "wgtn-bay-01", AckTimeout: time.Second, MaxAttempts: 1}, nopPublisher{})
	if err != nil {
		t.Fatal(err)
	}
	ack, _ := json.Marshal(wire.ProfileAck{
		Envelope:       wire.Envelope{MsgID: mustUUID(t), TS: time.Now().UTC(), SiteID: "wgtn-bay-01", NodeID: "nuc-02"},
		AckOf:          mustUUID(t),
		ProfileID:      "example-plant",
		ProfileVersion: "0.1.0",
		Status:         wire.ProfileApplied,
	})
	if err := p.HandleAck("agripod/wgtn-bay-01/nuc-01/profile/ack", ack); err == nil {
		t.Fatal("ack from a different node accepted")
	}
}

func TestUnmatchedAcksAreCountedNotDelivered(t *testing.T) {
	p, _ := NewPusher(Config{Site: "wgtn-bay-01", AckTimeout: time.Second, MaxAttempts: 1}, nopPublisher{})
	ack, _ := json.Marshal(wire.ProfileAck{
		Envelope:       wire.Envelope{MsgID: mustUUID(t), TS: time.Now().UTC(), SiteID: "wgtn-bay-01", NodeID: "nuc-01"},
		AckOf:          mustUUID(t),
		ProfileID:      "example-plant",
		ProfileVersion: "0.1.0",
		Status:         wire.ProfileApplied,
	})
	if err := p.HandleAck("agripod/wgtn-bay-01/nuc-01/profile/ack", ack); err != nil {
		t.Fatal(err)
	}
	if p.UnmatchedAcks() != 1 {
		t.Fatalf("unmatched = %d", p.UnmatchedAcks())
	}
}

func TestRegistrySilentNodes(t *testing.T) {
	reg := NewRegistry()
	now := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
	for _, node := range []string{"nuc-01", "nuc-02"} {
		topic, _ := wire.Topic("wgtn-bay-01", node, wire.KindStatus)
		payload, _ := json.Marshal(wire.Status{
			Envelope: wire.Envelope{MsgID: mustUUID(t), TS: now, SiteID: "wgtn-bay-01", NodeID: node},
			State:    wire.StateOnline,
		})
		if err := reg.HandleStatus(topic, payload, now); err != nil {
			t.Fatal(err)
		}
	}
	silent := reg.Silent(now.Add(2*time.Minute), time.Minute)
	if len(silent) != 2 {
		t.Fatalf("silent = %v", silent)
	}
	// Mismatched topic and payload node is refused.
	topic, _ := wire.Topic("wgtn-bay-01", "nuc-03", wire.KindStatus)
	payload, _ := json.Marshal(wire.Status{
		Envelope: wire.Envelope{MsgID: mustUUID(t), TS: now, SiteID: "wgtn-bay-01", NodeID: "nuc-01"},
		State:    wire.StateOnline,
	})
	if err := reg.HandleStatus(topic, payload, now); err == nil {
		t.Fatal("status with mismatched node accepted")
	}
}

type nopPublisher struct{}

func (nopPublisher) Publish(string, []byte) error { return nil }
