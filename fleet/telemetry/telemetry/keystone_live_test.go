//go:build keystone_live

// Run against a live TPT Keystone instance:
//
//	KEYSTONE_ADDR=127.0.0.1:5432 go test -tags keystone_live -count=1 ./telemetry/
//
// The test creates its own uniquely named table and does not touch existing data.

package telemetry

import (
	"context"
	"fmt"
	"os"
	"testing"
	"time"

	keystone "github.com/tpt/sdk-go"
)

func TestKeystoneLiveSchemaWriteAndIdempotentResend(t *testing.T) {
	addr := os.Getenv("KEYSTONE_ADDR")
	if addr == "" {
		t.Fatal("set KEYSTONE_ADDR to host:port of a running Keystone")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()

	table := fmt.Sprintf("telemetry_live_%d", time.Now().UnixNano())
	sess, err := NewKeystoneSession(addr, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer sess.Close()

	if err := InitSchema(ctx, sess, table, "1 day"); err != nil {
		t.Fatalf("schema: %v", err)
	}
	sink, err := NewKeystoneSink(sess, table, 500)
	if err != nil {
		t.Fatal(err)
	}

	base := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
	point := func(i int) Point {
		return Point{Node: "live-node", Metric: "vpd_kpa", TS: base.Add(time.Duration(i) * time.Minute), Value: float64(i)}
	}
	first := []Point{point(0), point(1), point(2)}
	if err := sink.Write(ctx, first); err != nil {
		t.Fatalf("first write: %v", err)
	}
	// A resend after a lost acknowledgement, plus one new point. Must store exactly one new row.
	if err := sink.Write(ctx, append(append([]Point{}, first...), point(3))); err != nil {
		t.Fatalf("resend: %v", err)
	}

	c, err := keystone.Connect(ctx, addr, nil)
	if err != nil {
		t.Fatalf("connect for count: %v", err)
	}
	defer c.Close()
	rows, err := c.Query(ctx, fmt.Sprintf("SELECT count(*) FROM %s", table))
	if err != nil {
		t.Fatalf("count query: %v", err)
	}
	defer rows.Close()
	if !rows.Next() {
		t.Fatalf("count returned no row: %v", rows.Err())
	}
	var n int64
	if err := rows.Scan(&n); err != nil {
		t.Fatalf("scan: %v", err)
	}
	if n != 4 {
		t.Fatalf("stored %d rows, want 4", n)
	}
}
