// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"
)

// Sink receives batches of points. A returned error means the batch was not durably stored.
// The batcher retries it; it never drops a batch silently.
type Sink interface {
	Write(ctx context.Context, batch []Point) error
}

// ErrStopped is returned by Submit after the batcher has stopped.
var ErrStopped = errors.New("telemetry: batcher stopped")

// Config tunes batching, buffering and retries.
type Config struct {
	// MaxBatch is the most points written in one sink call.
	MaxBatch int
	// FlushEvery writes a partial batch this often, so quiet nodes still show up.
	FlushEvery time.Duration
	// Buffer is how many points may wait before Submit blocks. This is the back-pressure bound.
	Buffer int
	// MaxAttempts is how many times one batch is tried before the batcher stops.
	MaxAttempts int
	// Backoff returns the wait before retry number attempt (1-based).
	Backoff func(attempt int) time.Duration
}

// DefaultConfig is a reasonable starting point for a single fleet node.
func DefaultConfig() Config {
	return Config{
		MaxBatch:    500,
		FlushEvery:  5 * time.Second,
		Buffer:      10_000,
		MaxAttempts: 8,
		Backoff: func(attempt int) time.Duration {
			d := 100 * time.Millisecond << min(attempt-1, 10)
			return min(d, 30*time.Second)
		},
	}
}

// Batcher groups points into batches and writes them to a sink on one goroutine.
type Batcher struct {
	cfg     Config
	sink    Sink
	in      chan Point
	stopped chan struct{}

	mu     sync.RWMutex // guards closing of in against concurrent Submit
	closed bool

	errMu sync.Mutex
	err   error
	done  chan struct{}
}

// NewBatcher starts the writer goroutine. Call Close to flush and stop it.
func NewBatcher(ctx context.Context, sink Sink, cfg Config) (*Batcher, error) {
	if cfg.MaxBatch < 1 || cfg.Buffer < 1 || cfg.MaxAttempts < 1 || cfg.FlushEvery <= 0 || cfg.Backoff == nil {
		return nil, errors.New("telemetry: invalid config")
	}
	b := &Batcher{
		cfg:     cfg,
		sink:    sink,
		in:      make(chan Point, cfg.Buffer),
		stopped: make(chan struct{}),
		done:    make(chan struct{}),
	}
	go b.run(ctx)
	return b, nil
}

// Submit queues one point. It blocks while the buffer is full (back-pressure) and returns early
// if ctx is cancelled or the batcher has stopped.
func (b *Batcher) Submit(ctx context.Context, p Point) error {
	if err := p.Validate(); err != nil {
		return err
	}
	b.mu.RLock()
	defer b.mu.RUnlock()
	if b.closed {
		return ErrStopped
	}
	select {
	case b.in <- p:
		return nil
	case <-b.stopped:
		return b.stopErr()
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Close stops accepting points, flushes everything queued, and returns the first unrecoverable
// error, if any. ctx bounds how long the flush may take.
func (b *Batcher) Close(ctx context.Context) error {
	b.mu.Lock()
	if !b.closed {
		b.closed = true
		close(b.in)
	}
	b.mu.Unlock()
	select {
	case <-b.done:
		return b.recordedErr()
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (b *Batcher) stopErr() error {
	b.errMu.Lock()
	defer b.errMu.Unlock()
	if b.err != nil {
		return b.err
	}
	return ErrStopped
}

func (b *Batcher) fail(err error) {
	b.errMu.Lock()
	if b.err == nil {
		b.err = err
	}
	b.errMu.Unlock()
}

func (b *Batcher) run(ctx context.Context) {
	defer close(b.done)
	defer close(b.stopped)

	ticker := time.NewTicker(b.cfg.FlushEvery)
	defer ticker.Stop()
	batch := make([]Point, 0, b.cfg.MaxBatch)

	flush := func() bool {
		if len(batch) == 0 {
			return true
		}
		err := b.writeWithRetry(ctx, batch)
		batch = batch[:0]
		if err != nil {
			b.fail(err)
			return false
		}
		return true
	}

	for {
		select {
		case p, ok := <-b.in:
			if !ok {
				flush()
				return
			}
			batch = append(batch, p)
			if len(batch) >= b.cfg.MaxBatch && !flush() {
				return
			}
		case <-ticker.C:
			if !flush() {
				return
			}
		case <-ctx.Done():
			b.fail(ctx.Err())
			return
		}
	}
}

func (b *Batcher) writeWithRetry(ctx context.Context, batch []Point) error {
	var last error
	for attempt := 1; attempt <= b.cfg.MaxAttempts; attempt++ {
		err := b.sink.Write(ctx, batch)
		if err == nil {
			return nil
		}
		last = err
		if attempt == b.cfg.MaxAttempts {
			break
		}
		timer := time.NewTimer(b.cfg.Backoff(attempt))
		select {
		case <-timer.C:
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		}
	}
	return fmt.Errorf("telemetry: %d points not stored after %d attempts: %w", len(batch), b.cfg.MaxAttempts, last)
}

// recordedErr returns the first unrecoverable error, or nil after a clean shutdown.
func (b *Batcher) recordedErr() error {
	b.errMu.Lock()
	defer b.errMu.Unlock()
	return b.err
}
