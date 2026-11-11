// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"

	"tpt-agripod-wire"
)

// captureSubmitter records what the ingester hands over.
type captureSubmitter struct {
	points []Point
	err    error
}

func (c *captureSubmitter) Submit(_ context.Context, p Point) error {
	if c.err != nil {
		return c.err
	}
	c.points = append(c.points, p)
	return nil
}

const (
	msgA = "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d10"
	msgB = "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d11"
	msgC = "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d12"
)

// telemetryJSON builds one telemetry payload from node nuc-01.
func telemetryJSON(t *testing.T, msgID string, seq uint64, samples ...wire.Sample) []byte {
	t.Helper()
	m := wire.Telemetry{
		Envelope: wire.Envelope{MsgID: msgID, TS: t0, SiteID: "wgtn-bay-01", NodeID: "nuc-01"},
		Seq:      seq,
		Samples:  samples,
	}
	b, err := json.Marshal(m)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func good(channel string, value float64) wire.Sample {
	return wire.Sample{Channel: channel, TS: t0, Value: value, Unit: "frac", Quality: wire.QualityGood}
}

func newTestIngester(t *testing.T, out Submitter) *Ingester {
	t.Helper()
	in, err := NewIngester(context.Background(), out, 100)
	if err != nil {
		t.Fatal(err)
	}
	return in
}

const telemetryTopic = "agripod/wgtn-bay-01/nuc-01/telemetry"

func TestGoodSamplesBecomePoints(t *testing.T) {
	out := &captureSubmitter{}
	in := newTestIngester(t, out)
	if err := in.Handle(telemetryTopic, telemetryJSON(t, msgA, 1, good("rh", 0.6), good("air_temp", 20))); err != nil {
		t.Fatal(err)
	}
	if len(out.points) != 2 || out.points[0].Metric != "rh" || out.points[0].Node != "nuc-01" {
		t.Fatalf("points: %+v", out.points)
	}
	if st := in.Stats(); st.Accepted != 1 || st.Points != 2 {
		t.Fatalf("stats: %+v", st)
	}
}

func TestQualityOtherThanGoodIsSkipped(t *testing.T) {
	out := &captureSubmitter{}
	in := newTestIngester(t, out)
	stale := good("soil_moisture", 0.3)
	stale.Quality = wire.QualityStale
	if err := in.Handle(telemetryTopic, telemetryJSON(t, msgA, 1, good("rh", 0.6), stale)); err != nil {
		t.Fatal(err)
	}
	if len(out.points) != 1 {
		t.Fatalf("stored %d points, want 1", len(out.points))
	}
	if st := in.Stats(); st.SkippedQuality != 1 {
		t.Fatalf("stats: %+v", st)
	}
}

func TestDeviceSamplesDoNotCollideOnTheKey(t *testing.T) {
	out := &captureSubmitter{}
	in := newTestIngester(t, out)
	z1 := good("soil_moisture", 0.31)
	z1.DeviceID = "esp32-z1"
	z2 := good("soil_moisture", 0.35)
	z2.DeviceID = "esp32-z2"
	if err := in.Handle(telemetryTopic, telemetryJSON(t, msgA, 1, z1, z2)); err != nil {
		t.Fatal(err)
	}
	if out.points[0].Node == out.points[1].Node {
		t.Fatalf("two devices share node key %q", out.points[0].Node)
	}
	if out.points[0].Node != "nuc-01/esp32-z1" {
		t.Fatalf("device key %q", out.points[0].Node)
	}
}

func TestDuplicateMessagesAreNotReprocessed(t *testing.T) {
	out := &captureSubmitter{}
	in := newTestIngester(t, out)
	payload := telemetryJSON(t, msgA, 1, good("rh", 0.6))
	for range 3 {
		if err := in.Handle(telemetryTopic, payload); err != nil {
			t.Fatal(err)
		}
	}
	if len(out.points) != 1 {
		t.Fatalf("stored %d points from 3 deliveries, want 1", len(out.points))
	}
	if st := in.Stats(); st.Duplicates != 2 {
		t.Fatalf("stats: %+v", st)
	}
}

func TestSequenceGapsAndReordersAreCounted(t *testing.T) {
	out := &captureSubmitter{}
	in := newTestIngester(t, out)
	ids := []string{msgA, msgB, msgC, "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d13"}
	seqs := []uint64{5, 6, 9, 3} // gap of 2 after 6, then a reorder to 3
	for i, seq := range seqs {
		if err := in.Handle(telemetryTopic, telemetryJSON(t, ids[i], seq, good("rh", 0.5))); err != nil {
			t.Fatal(err)
		}
	}
	st := in.Stats()
	if st.SeqGaps != 2 || st.SeqReorders != 1 {
		t.Fatalf("gaps=%d reorders=%d, want 2 and 1", st.SeqGaps, st.SeqReorders)
	}
}

func TestTopicAndPayloadMismatchesAreRejected(t *testing.T) {
	out := &captureSubmitter{}
	in := newTestIngester(t, out)
	if err := in.Handle("agripod/wgtn-bay-01/nuc-02/telemetry", telemetryJSON(t, msgA, 1, good("rh", 0.5))); err == nil {
		t.Error("node mismatch accepted")
	}
	if err := in.Handle("agripod/wgtn-bay-01/nuc-01/status", telemetryJSON(t, msgA, 1, good("rh", 0.5))); err == nil {
		t.Error("wrong topic kind accepted")
	}
	if err := in.Handle(telemetryTopic, []byte(`{"msg_id":"x"}`)); err == nil {
		t.Error("invalid payload accepted")
	}
	if st := in.Stats(); st.Rejected != 3 || st.Points != 0 {
		t.Fatalf("stats: %+v", st)
	}
}

func TestSubmitterErrorsAreReturned(t *testing.T) {
	out := &captureSubmitter{err: errors.New("pipeline stopped")}
	in := newTestIngester(t, out)
	err := in.Handle(telemetryTopic, telemetryJSON(t, msgA, 1, good("rh", 0.5)))
	if err == nil || !strings.Contains(err.Error(), "pipeline stopped") {
		t.Fatalf("got %v", err)
	}
}

func TestDedupMemoryIsBounded(t *testing.T) {
	ids := newIDSet(2)
	ids.add("a")
	ids.add("b")
	ids.add("c") // evicts "a"
	if ids.contains("a") || !ids.contains("b") || !ids.contains("c") {
		t.Fatalf("set contents wrong: %v", ids.order)
	}
	if len(ids.set) != 2 {
		t.Fatalf("set size %d", len(ids.set))
	}
}

func TestEveryMessageIsCountedOnce(t *testing.T) {
	out := &captureSubmitter{}
	in := newTestIngester(t, out)
	for i := range 20 {
		id := fmt.Sprintf("0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6%03d", i)
		_ = in.Handle(telemetryTopic, telemetryJSON(t, id, uint64(i+1), good("rh", 0.5)))
	}
	st := in.Stats()
	if st.Messages != 20 || st.Accepted != 20 {
		t.Fatalf("stats: %+v", st)
	}
}
