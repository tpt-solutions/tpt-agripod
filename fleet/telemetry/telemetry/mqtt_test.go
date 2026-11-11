// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"context"
	"fmt"
	"net"
	"testing"
	"time"

	mqtt "github.com/mochi-mqtt/server/v2"
	"github.com/mochi-mqtt/server/v2/hooks/auth"
	"github.com/mochi-mqtt/server/v2/listeners"
	"github.com/yosssi/gmq/mqtt/client"
)

// startBroker runs an in-process MQTT broker on a free localhost port. It stops at test end.
func startBroker(t *testing.T) string {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Skipf("cannot listen on localhost: %v", err)
	}
	addr := l.Addr().String()
	l.Close()

	server := mqtt.New(&mqtt.Options{InlineClient: false})
	if err := server.AddHook(new(auth.AllowHook), nil); err != nil {
		t.Fatal(err)
	}
	if err := server.AddListener(listeners.NewTCP(listeners.Config{ID: "t1", Address: addr})); err != nil {
		t.Fatal(err)
	}
	go func() { _ = server.Serve() }()
	t.Cleanup(func() { _ = server.Close() })

	// Wait until the broker accepts connections.
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

func publisher(t *testing.T, addr string) *client.Client {
	t.Helper()
	cli := client.New(&client.Options{ErrorHandler: func(error) {}})
	if err := cli.Connect(&client.ConnectOptions{
		Network:      "tcp",
		Address:      addr,
		ClientID:     []byte("test-publisher"),
		CleanSession: true,
		KeepAlive:    30,
	}); err != nil {
		t.Fatalf("publisher connect: %v", err)
	}
	t.Cleanup(func() {
		_ = cli.Disconnect()
		cli.Terminate()
	})
	return cli
}

// TestEndToEndTelemetryThroughTheBroker publishes telemetry over real MQTT and checks that the
// points reach a sink, duplicates are dropped, and a non-good sample is skipped.
func TestEndToEndTelemetryThroughTheBroker(t *testing.T) {
	addr := startBroker(t)
	sink := &MemorySink{}
	ctx := context.Background()
	batcher, err := NewBatcher(ctx, sink, fastConfig())
	if err != nil {
		t.Fatal(err)
	}
	ing, err := NewIngester(ctx, batcher, 1000)
	if err != nil {
		t.Fatal(err)
	}
	var onErr []error
	src, err := StartSource(SourceConfig{
		Address:   addr,
		ClientID:  "telemetry-test",
		KeepAlive: 30,
		OnError:   func(e error) { onErr = append(onErr, e) },
	}, ing)
	if err != nil {
		t.Fatalf("start source: %v", err)
	}
	defer src.Close()

	pub := publisher(t, addr)
	msg := func(id string, seq int, quality string) []byte {
		return []byte(fmt.Sprintf(`{"msg_id":%q,"ts":"2026-11-11T02:15:00Z","site_id":"wgtn-bay-01","node_id":"nuc-01","seq":%d,"samples":[`+
			`{"channel":"rh","ts":"2026-11-11T02:15:00Z","value":0.6,"unit":"frac","quality":"good"},`+
			`{"channel":"air_temp","ts":"2026-11-11T02:15:00Z","value":20.5,"unit":"degC","quality":%q}]}`, id, seq, quality))
	}
	topic := []byte("agripod/wgtn-bay-01/nuc-01/telemetry")
	publish := func(p []byte) {
		if err := pub.Publish(&client.PublishOptions{QoS: 1, TopicName: topic, Message: p}); err != nil {
			t.Fatalf("publish: %v", err)
		}
	}
	id1 := "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d10"
	id2 := "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d11"
	publish(msg(id1, 1, "good"))
	publish(msg(id1, 1, "good")) // duplicate delivery
	publish(msg(id2, 2, "stale"))

	// Wait until the ingester has seen all three deliveries.
	deadline := time.Now().Add(5 * time.Second)
	for ing.Stats().Messages < 3 {
		if time.Now().After(deadline) {
			t.Fatalf("broker delivered %d of 3 messages; errors: %v", ing.Stats().Messages, onErr)
		}
		time.Sleep(10 * time.Millisecond)
	}
	if err := batcher.Close(ctx); err != nil {
		t.Fatalf("batcher: %v", err)
	}
	st := ing.Stats()
	if st.Duplicates != 1 || st.SkippedQuality != 1 {
		t.Fatalf("stats: %+v", st)
	}
	// Two points from id1 (rh and air_temp), plus rh from id2.
	if got := len(sink.Points()); got != 3 {
		t.Fatalf("stored %d points, want 3", got)
	}
}
