// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

var t0 = time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)

func pt(node, metric string, sec int) Point {
	return Point{Node: node, Metric: metric, TS: t0.Add(time.Duration(sec) * time.Second), Value: float64(sec)}
}

func fastConfig() Config {
	cfg := DefaultConfig()
	cfg.FlushEvery = time.Hour
	cfg.Backoff = func(int) time.Duration { return 0 }
	return cfg
}

func TestPointValidation(t *testing.T) {
	good := pt("node-1", "vpd_kpa", 0)
	if err := good.Validate(); err != nil {
		t.Fatalf("valid point rejected: %v", err)
	}
	local := good
	local.TS = good.TS.In(time.FixedZone("NZST", 12*3600))
	bad := map[string]Point{
		"empty node":   {Node: "", Metric: "vpd_kpa", TS: t0, Value: 1},
		"bad metric":   {Node: "n", Metric: "VPD-kPa", TS: t0, Value: 1},
		"zero time":    {Node: "n", Metric: "vpd_kpa", Value: 1},
		"local time":   local,
		"NaN value":    {Node: "n", Metric: "vpd_kpa", TS: t0, Value: math.NaN()},
		"inf value":    {Node: "n", Metric: "vpd_kpa", TS: t0, Value: math.Inf(1)},
		"metric digit": {Node: "n", Metric: "1vpd", TS: t0, Value: 1},
	}
	for name, p := range bad {
		if err := p.Validate(); err == nil {
			t.Errorf("%s: accepted %+v", name, p)
		}
	}
}

func TestBatcherWritesEverythingOnClose(t *testing.T) {
	sink := &MemorySink{}
	b, err := NewBatcher(context.Background(), sink, fastConfig())
	if err != nil {
		t.Fatal(err)
	}
	for i := range 1234 {
		if err := b.Submit(context.Background(), pt("n", "m", i)); err != nil {
			t.Fatal(err)
		}
	}
	if err := b.Close(context.Background()); err != nil {
		t.Fatalf("close: %v", err)
	}
	if got := len(sink.Points()); got != 1234 {
		t.Fatalf("stored %d points, want 1234", got)
	}
}

func TestPartialBatchFlushesOnTheInterval(t *testing.T) {
	sink := &MemorySink{}
	cfg := fastConfig()
	cfg.FlushEvery = 10 * time.Millisecond
	b, err := NewBatcher(context.Background(), sink, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close(context.Background())
	if err := b.Submit(context.Background(), pt("n", "m", 0)); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for len(sink.Points()) == 0 {
		if time.Now().After(deadline) {
			t.Fatal("single point never flushed")
		}
		time.Sleep(time.Millisecond)
	}
}

// blockingSink holds each write until released, so the test can fill the buffer.
type blockingSink struct {
	entered chan struct{}
	release chan struct{}
}

func (s *blockingSink) Write(_ context.Context, _ []Point) error {
	s.entered <- struct{}{}
	<-s.release
	return nil
}

func TestBackPressureBlocksSubmitWhenTheBufferIsFull(t *testing.T) {
	sink := &blockingSink{entered: make(chan struct{}, 10), release: make(chan struct{})}
	cfg := fastConfig()
	cfg.MaxBatch = 1
	cfg.Buffer = 2
	b, err := NewBatcher(context.Background(), sink, cfg)
	if err != nil {
		t.Fatal(err)
	}
	bg := context.Background()
	if err := b.Submit(bg, pt("n", "m", 0)); err != nil {
		t.Fatal(err)
	}
	<-sink.entered // the writer is now stuck in the sink, holding point 0
	if err := b.Submit(bg, pt("n", "m", 1)); err != nil {
		t.Fatal(err)
	}
	if err := b.Submit(bg, pt("n", "m", 2)); err != nil {
		t.Fatal(err)
	}
	// Buffer holds 2 points and the writer is stuck: this must block until the context ends.
	ctx, cancel := context.WithTimeout(bg, 50*time.Millisecond)
	defer cancel()
	if err := b.Submit(ctx, pt("n", "m", 3)); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("expected back-pressure timeout, got %v", err)
	}
	close(sink.release)
	if err := b.Close(bg); err != nil {
		t.Fatalf("close: %v", err)
	}
}

// flakySink fails the first n writes with err, then succeeds.
type flakySink struct {
	mu   sync.Mutex
	fail int
	err  error
	got  []Point
}

func (s *flakySink) Write(_ context.Context, batch []Point) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.fail > 0 {
		s.fail--
		return s.err
	}
	s.got = append(s.got, batch...)
	return nil
}

func TestTransientSinkFailuresAreRetriedNotDropped(t *testing.T) {
	sink := &flakySink{fail: 2, err: errors.New("connection reset")}
	cfg := fastConfig()
	cfg.MaxAttempts = 5
	b, err := NewBatcher(context.Background(), sink, cfg)
	if err != nil {
		t.Fatal(err)
	}
	for i := range 3 {
		_ = b.Submit(context.Background(), pt("n", "m", i))
	}
	if err := b.Close(context.Background()); err != nil {
		t.Fatalf("close after transient failures: %v", err)
	}
	if len(sink.got) != 3 {
		t.Fatalf("stored %d, want 3", len(sink.got))
	}
}

func TestPersistentSinkFailureIsReportedAndStopsTheBatcher(t *testing.T) {
	sink := &flakySink{fail: 1000, err: errors.New("disk full")}
	cfg := fastConfig()
	cfg.MaxAttempts = 3
	b, err := NewBatcher(context.Background(), sink, cfg)
	if err != nil {
		t.Fatal(err)
	}
	_ = b.Submit(context.Background(), pt("n", "m", 0))
	err = b.Close(context.Background())
	if err == nil || !strings.Contains(err.Error(), "not stored after 3 attempts") {
		t.Fatalf("expected a loud failure, got %v", err)
	}
	if err := b.Submit(context.Background(), pt("n", "m", 1)); err == nil {
		t.Fatal("submit after failure must not be accepted")
	}
}

func TestSubmitValidatesBeforeQueueing(t *testing.T) {
	b, err := NewBatcher(context.Background(), &MemorySink{}, fastConfig())
	if err != nil {
		t.Fatal(err)
	}
	if err := b.Submit(context.Background(), Point{Node: "n", Metric: "x y", TS: t0, Value: 1}); err == nil {
		t.Fatal("invalid point was queued")
	}
	_ = b.Close(context.Background())
}

func TestFileSinkRoundTrip(t *testing.T) {
	path := filepath.Join(t.TempDir(), "points.jsonl")
	s, err := OpenFileSink(path)
	if err != nil {
		t.Fatal(err)
	}
	want := []Point{pt("a", "vpd_kpa", 0), pt("b", "power_w", 5)}
	if err := s.Write(context.Background(), want); err != nil {
		t.Fatal(err)
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	f, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	var got []Point
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		var p Point
		if err := json.Unmarshal(sc.Bytes(), &p); err != nil {
			t.Fatal(err)
		}
		got = append(got, p)
	}
	if len(got) != len(want) {
		t.Fatalf("read %d lines, want %d", len(got), len(want))
	}
	for i := range want {
		if !got[i].TS.Equal(want[i].TS) || got[i].Node != want[i].Node || got[i].Value != want[i].Value {
			t.Errorf("line %d: got %+v, want %+v", i, got[i], want[i])
		}
	}
}

// fakeKeystone enforces the primary key the way Keystone does: a statement that duplicates a key
// fails as a whole, storing nothing. It also counts transaction control statements.
type fakeKeystone struct {
	mu        sync.Mutex
	rows      map[string]float64
	commits   int
	rollbacks int
	inserts   int
	failNext  error
}

func newFakeKeystone() *fakeKeystone {
	return &fakeKeystone{rows: map[string]float64{}}
}

func (f *fakeKeystone) Exec(_ context.Context, sql string, args ...any) (int64, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	switch sql {
	case "BEGIN":
		return 0, nil
	case "COMMIT":
		f.commits++
		return 0, nil
	case "ROLLBACK":
		f.rollbacks++
		return 0, nil
	}
	if f.failNext != nil {
		err := f.failNext
		f.failNext = nil
		return 0, err
	}
	if !strings.HasPrefix(sql, "INSERT INTO "+SeriesTable) {
		return 0, fmt.Errorf("fake: unexpected statement %q", sql)
	}
	if strings.Count(sql, "$") != len(args) || len(args)%4 != 0 {
		return 0, fmt.Errorf("fake: %d placeholders for %d args", strings.Count(sql, "$"), len(args))
	}
	f.inserts++
	seen := map[string]bool{}
	n := len(args) / 4
	for i := range n {
		key := fmt.Sprint(args[4*i], "|", args[4*i+1], "|", args[4*i+2])
		if _, ok := f.rows[key]; ok || seen[key] {
			return 0, fmt.Errorf("keystone: server error: duplicate key value violates unique constraint \"sensor_series_pkey\"")
		}
		seen[key] = true
	}
	for i := range n {
		key := fmt.Sprint(args[4*i], "|", args[4*i+1], "|", args[4*i+2])
		f.rows[key] = args[4*i+3].(float64)
	}
	return int64(n), nil
}

func TestKeystoneSinkStoresOnceWhenTheSameBatchIsResent(t *testing.T) {
	ks := newFakeKeystone()
	sink, err := NewKeystoneSink(ks, 500)
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	a := []Point{pt("n", "m", 0), pt("n", "m", 1), pt("n", "m", 2)}
	if err := sink.Write(ctx, a); err != nil {
		t.Fatal(err)
	}
	// A retry after a lost acknowledgement: the same three points plus one new one.
	if err := sink.Write(ctx, append(append([]Point{}, a...), pt("n", "m", 3))); err != nil {
		t.Fatalf("resend must succeed: %v", err)
	}
	if len(ks.rows) != 4 {
		t.Fatalf("stored %d rows, want 4 (no duplicates, no losses)", len(ks.rows))
	}
	if ks.rollbacks != 1 {
		t.Fatalf("rollbacks = %d, want 1 for the resent chunk", ks.rollbacks)
	}
}

func TestKeystoneSinkRaisesOtherErrorsWithoutTheDuplicatePath(t *testing.T) {
	ks := newFakeKeystone()
	ks.failNext = errors.New("connection reset by peer")
	sink, _ := NewKeystoneSink(ks, 500)
	err := sink.Write(context.Background(), []Point{pt("n", "m", 0)})
	if err == nil || IsDuplicate(err) {
		t.Fatalf("expected a plain error, got %v", err)
	}
	if ks.rollbacks != 1 || len(ks.rows) != 0 {
		t.Fatalf("rollbacks=%d rows=%d", ks.rollbacks, len(ks.rows))
	}
}

func TestKeystoneSinkChunksLargeBatches(t *testing.T) {
	ks := newFakeKeystone()
	sink, _ := NewKeystoneSink(ks, 500)
	batch := make([]Point, 1200)
	for i := range batch {
		batch[i] = pt("n", "m", i)
	}
	if err := sink.Write(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	if ks.commits != 3 || len(ks.rows) != 1200 {
		t.Fatalf("commits=%d rows=%d, want 3 and 1200", ks.commits, len(ks.rows))
	}
}

func TestKeystoneSinkRejectsBadChunkSizes(t *testing.T) {
	if _, err := NewKeystoneSink(newFakeKeystone(), 0); err == nil {
		t.Error("maxRows 0 accepted")
	}
	if _, err := NewKeystoneSink(newFakeKeystone(), 16_001); err == nil {
		t.Error("maxRows above the parameter limit accepted")
	}
}

func TestInsertSQLNumbersEveryParameter(t *testing.T) {
	got := insertSQL(2)
	want := "INSERT INTO sensor_series (node, metric, ts, value) VALUES ($1, $2, $3, $4), ($5, $6, $7, $8)"
	if got != want {
		t.Fatalf("got %q\nwant %q", got, want)
	}
}

func TestBatcherIntoKeystoneSurvivesATransientInsertFailure(t *testing.T) {
	ks := newFakeKeystone()
	ks.failNext = errors.New("timeout waiting for lock")
	sink, _ := NewKeystoneSink(ks, 500)
	cfg := fastConfig()
	cfg.MaxAttempts = 3
	b, err := NewBatcher(context.Background(), sink, cfg)
	if err != nil {
		t.Fatal(err)
	}
	for i := range 5 {
		_ = b.Submit(context.Background(), pt("n", "m", i))
	}
	if err := b.Close(context.Background()); err != nil {
		t.Fatalf("close: %v", err)
	}
	if len(ks.rows) != 5 {
		t.Fatalf("stored %d rows, want 5", len(ks.rows))
	}
}

func TestSchemaStatementsCarryTheRetentionPolicy(t *testing.T) {
	stmts := SchemaStatements("90 days")
	if len(stmts) != 2 {
		t.Fatalf("got %d statements", len(stmts))
	}
	if !strings.Contains(stmts[0], "PRIMARY KEY (node, metric, ts)") {
		t.Error("primary key must cover node, metric and ts for idempotent writes")
	}
	if !strings.Contains(stmts[1], "retention = '90 days'") || !strings.Contains(stmts[1], "USING TIME(ts)") {
		t.Errorf("chronos index: %s", stmts[1])
	}
}
