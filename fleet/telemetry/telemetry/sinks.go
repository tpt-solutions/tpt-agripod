// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"sync"
)

// FileSink appends points to a file as JSON Lines and fsyncs after each batch, so an acknowledged
// batch survives a crash. It is the offline sink and the reference for tests.
type FileSink struct {
	mu sync.Mutex
	f  *os.File
	w  *bufio.Writer
}

// OpenFileSink opens (or creates) path for appending.
func OpenFileSink(path string) (*FileSink, error) {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o640)
	if err != nil {
		return nil, fmt.Errorf("telemetry: open %s: %w", path, err)
	}
	return &FileSink{f: f, w: bufio.NewWriter(f)}, nil
}

// Write encodes each point as one JSON line, then flushes and syncs.
func (s *FileSink) Write(ctx context.Context, batch []Point) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	enc := json.NewEncoder(s.w)
	for _, p := range batch {
		if err := enc.Encode(p); err != nil {
			return fmt.Errorf("telemetry: encode: %w", err)
		}
	}
	if err := s.w.Flush(); err != nil {
		return fmt.Errorf("telemetry: flush: %w", err)
	}
	if err := s.f.Sync(); err != nil {
		return fmt.Errorf("telemetry: sync: %w", err)
	}
	return nil
}

// Close flushes and closes the file.
func (s *FileSink) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.w.Flush(); err != nil {
		_ = s.f.Close()
		return err
	}
	return s.f.Close()
}

// MemorySink keeps points in memory. Tests use it, and it can inject failures.
type MemorySink struct {
	mu     sync.Mutex
	points []Point
	// FailNext makes the next n Write calls fail with the error.
	FailNext int
	Err      error
}

// Write appends the batch, unless a failure has been injected.
func (m *MemorySink) Write(_ context.Context, batch []Point) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.FailNext > 0 {
		m.FailNext--
		return m.Err
	}
	m.points = append(m.points, batch...)
	return nil
}

// Points returns a copy of everything stored so far.
func (m *MemorySink) Points() []Point {
	m.mu.Lock()
	defer m.mu.Unlock()
	return append([]Point(nil), m.points...)
}
