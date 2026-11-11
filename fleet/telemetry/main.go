// SPDX-License-Identifier: MIT OR Apache-2.0

// Command tpt-agripod-telemetry ingests sensor points as JSON Lines and writes them through
// the batcher to a sink (ADR 0005). With -in it reads a file or stdin. With -broker it subscribes
// to telemetry over MQTT (ADR 0007) until interrupted. The Keystone sink is not selectable from
// the command yet: the SDK adapter is not in place (ADR 0009).
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

func main() {
	in := flag.String("in", "-", "JSON Lines input, or - for stdin")
	out := flag.String("out", "points.jsonl", "file sink path (appended, fsynced per batch)")
	broker := flag.String("broker", "", "MQTT broker host:port; if set, ingest telemetry from MQTT instead of -in")
	flag.Parse()
	fmt.Fprintf(os.Stderr, "tpt-agripod-telemetry %s\n", version)

	if *broker != "" {
		if err := runMQTT(*broker, *out); err != nil {
			fmt.Fprintf(os.Stderr, "tpt-agripod-telemetry: %v\n", err)
			os.Exit(1)
		}
		return
	}
	if err := run(*in, *out); err != nil {
		fmt.Fprintf(os.Stderr, "tpt-agripod-telemetry: %v\n", err)
		os.Exit(1)
	}
}

func run(inPath, outPath string) error {
	r, closeIn, err := openInput(inPath)
	if err != nil {
		return err
	}
	defer closeIn()

	sink, err := telemetry.OpenFileSink(outPath)
	if err != nil {
		return err
	}
	defer sink.Close()

	ctx := context.Background()
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
		_ = b.Close(ctx)
		return fmt.Errorf("read input: %w", err)
	}
	if err := b.Close(ctx); err != nil {
		return err
	}
	fmt.Fprintf(os.Stderr, "stored %d points, rejected %d\n", accepted, rejected)
	return nil
}

func openInput(path string) (io.Reader, func(), error) {
	if path == "-" {
		return os.Stdin, func() {}, nil
	}
	f, err := os.Open(path)
	if err != nil {
		return nil, nil, err
	}
	return f, func() { _ = f.Close() }, nil
}

// runMQTT ingests telemetry from the broker until SIGINT or SIGTERM. Shutdown closes the MQTT
// session first, so no new messages arrive, then flushes the batcher.
func runMQTT(broker, outPath string) error {
	sink, err := telemetry.OpenFileSink(outPath)
	if err != nil {
		return err
	}
	defer sink.Close()

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

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
