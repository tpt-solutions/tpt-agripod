// SPDX-License-Identifier: MIT OR Apache-2.0

// Command tpt-agripod-telemetry ingests sensor points and writes them through the batcher to a
// sink (ADR 0005).
//
//	telemetry -init-schema -keystone HOST:PORT [-table T] [-retention D]
//	telemetry -in FILE|- [-out FILE | -keystone HOST:PORT [-table T]]
//	telemetry -broker HOST:PORT [-out FILE | -keystone HOST:PORT [-table T]]
//
// -in reads JSON Lines and exits at end of input. -broker subscribes to telemetry over MQTT
// (ADR 0010) until interrupted, then flushes and exits.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
	"syscall"
	"time"

	"tpt-agripod-telemetry/telemetry"
)

const version = "0.1.0"

type options struct {
	in, out, broker, keystone, table, retention string
	initSchema                                  bool
}

func main() {
	var o options
	flag.StringVar(&o.in, "in", "", "JSON Lines input file, or - for stdin")
	flag.StringVar(&o.broker, "broker", "", "MQTT broker host:port; ingest telemetry until interrupted")
	flag.StringVar(&o.out, "out", "points.jsonl", "file sink path (used when -keystone is not set)")
	flag.StringVar(&o.keystone, "keystone", "", "Keystone host:port; write to Keystone instead of a file")
	flag.StringVar(&o.table, "table", telemetry.SeriesTable, "Keystone table name")
	flag.StringVar(&o.retention, "retention", telemetry.DefaultRetention, "raw retention for -init-schema")
	flag.BoolVar(&o.initSchema, "init-schema", false, "create the Keystone table and index, then exit")
	flag.Parse()

	fmt.Fprintf(os.Stderr, "tpt-agripod-telemetry %s\n", version)
	if err := run(o); err != nil {
		fmt.Fprintf(os.Stderr, "tpt-agripod-telemetry: %v\n", err)
		os.Exit(1)
	}
}

func run(o options) error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	if o.initSchema {
		if o.keystone == "" {
			return fmt.Errorf("-init-schema needs -keystone")
		}
		sess, err := telemetry.NewKeystoneSession(o.keystone, nil)
		if err != nil {
			return err
		}
		defer sess.Close()
		if err := telemetry.InitSchema(ctx, sess, o.table, o.retention); err != nil {
			return err
		}
		fmt.Fprintf(os.Stderr, "created %s (retention %s)\n", o.table, o.retention)
		return nil
	}

	sink, closeSink, err := buildSink(ctx, o)
	if err != nil {
		return err
	}
	defer closeSink()

	switch {
	case o.broker != "":
		return ingestMQTT(ctx, o.broker, sink)
	case o.in != "":
		return ingestFile(ctx, o.in, sink)
	default:
		return fmt.Errorf("choose a source: -in FILE or -broker HOST:PORT (or -init-schema)")
	}
}

// buildSink returns the configured sink and a function that releases it.
func buildSink(ctx context.Context, o options) (telemetry.Sink, func(), error) {
	if o.keystone != "" {
		sess, err := telemetry.NewKeystoneSession(o.keystone, nil)
		if err != nil {
			return nil, nil, err
		}
		sink, err := telemetry.NewKeystoneSink(sess, o.table, 500)
		if err != nil {
			_ = sess.Close()
			return nil, nil, err
		}
		return sink, func() { _ = sess.Close() }, nil
	}
	fs, err := telemetry.OpenFileSink(o.out)
	if err != nil {
		return nil, nil, err
	}
	return fs, func() { _ = fs.Close() }, nil
}

func ingestFile(ctx context.Context, path string, sink telemetry.Sink) error {
	var r io.Reader = os.Stdin
	if path != "-" {
		f, err := os.Open(path)
		if err != nil {
			return err
		}
		defer f.Close()
		r = f
	}

	b, err := telemetry.NewBatcher(ctx, sink, telemetry.DefaultConfig())
	if err != nil {
		return err
	}
	accepted, rejected := 0, 0
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for line := 1; sc.Scan(); line++ {
		var p telemetry.Point
		if err := json.Unmarshal(sc.Bytes(), &p); err != nil {
			rejected++
			fmt.Fprintf(os.Stderr, "line %d: %v\n", line, err)
			continue
		}
		if err := b.Submit(ctx, p); err != nil {
			if err == telemetry.ErrStopped {
				break
			}
			rejected++
			fmt.Fprintf(os.Stderr, "line %d: %v\n", line, err)
			continue
		}
		accepted++
	}
	if err := sc.Err(); err != nil {
		_ = b.Close(context.Background())
		return fmt.Errorf("read input: %w", err)
	}
	if err := b.Close(context.Background()); err != nil {
		return err
	}
	fmt.Fprintf(os.Stderr, "stored %d points, rejected %d\n", accepted, rejected)
	return nil
}

// ingestMQTT ingests telemetry from the broker until ctx is cancelled. Shutdown closes the MQTT
// session first, so no new messages arrive, then flushes the batcher.
func ingestMQTT(ctx context.Context, broker string, sink telemetry.Sink) error {
	b, err := telemetry.NewBatcher(ctx, sink, telemetry.DefaultConfig())
	if err != nil {
		return err
	}
	ing, err := telemetry.NewIngester(ctx, b, 100_000)
	if err != nil {
		return err
	}
	src, err := telemetry.StartSource(telemetry.SourceConfig{
		Address:   broker,
		ClientID:  fmt.Sprintf("telemetry-%d", os.Getpid()),
		KeepAlive: 30,
		OnError:   func(err error) { fmt.Fprintf(os.Stderr, "mqtt: %v\n", err) },
	}, ing)
	if err != nil {
		_ = b.Close(context.Background())
		return err
	}
	fmt.Fprintf(os.Stderr, "ingesting telemetry from %s; interrupt to stop\n", broker)
	<-ctx.Done()

	_ = src.Close()
	closeCtx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if err := b.Close(closeCtx); err != nil {
		return err
	}
	st := ing.Stats()
	fmt.Fprintf(os.Stderr, "messages %d, accepted %d, duplicates %d, rejected %d, points %d, seq gaps %d\n",
		st.Messages, st.Accepted, st.Duplicates, st.Rejected, st.Points, st.SeqGaps)
	return nil
}
