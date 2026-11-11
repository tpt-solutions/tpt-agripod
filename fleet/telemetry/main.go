// SPDX-License-Identifier: MIT OR Apache-2.0

// Command tpt-agripod-telemetry ingests sensor points as JSON Lines and writes them through
// the batcher to a sink (ADR 0005). Today it reads stdin and writes a file. The MQTT source and
// the Keystone sink are wired in once the MQTT client is added (ADR 0006, 0007) and the Keystone
// SDK adapter is in place (ADR 0009).
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"

	"tpt-agripod-telemetry/telemetry"
)

const version = "0.1.0"

func main() {
	in := flag.String("in", "-", "JSON Lines input, or - for stdin")
	out := flag.String("out", "points.jsonl", "file sink path (appended, fsynced per batch)")
	flag.Parse()
	fmt.Fprintf(os.Stderr, "tpt-agripod-telemetry %s\n", version)

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
