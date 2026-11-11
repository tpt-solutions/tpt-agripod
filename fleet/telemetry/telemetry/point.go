// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"errors"
	"fmt"
	"math"
	"regexp"
	"time"
)

// Point is one sensor reading from an edge node.
type Point struct {
	Node   string    `json:"node"`
	Metric string    `json:"metric"`
	TS     time.Time `json:"ts"`
	Value  float64   `json:"value"`
}

var metricPattern = regexp.MustCompile(`^[a-z][a-z0-9_]{0,62}$`)

// Validate checks a point before it enters the pipeline. Timestamps must be UTC, because the
// units doc forbids local time on the wire.
func (p Point) Validate() error {
	if p.Node == "" {
		return errors.New("telemetry: node is empty")
	}
	if !metricPattern.MatchString(p.Metric) {
		return fmt.Errorf("telemetry: metric %q must match %s", p.Metric, metricPattern)
	}
	if p.TS.IsZero() {
		return errors.New("telemetry: timestamp is zero")
	}
	if p.TS.Location() != time.UTC {
		return fmt.Errorf("telemetry: timestamp %s is not UTC", p.TS.Format(time.RFC3339Nano))
	}
	if math.IsNaN(p.Value) || math.IsInf(p.Value, 0) {
		return errors.New("telemetry: value is not finite")
	}
	return nil
}

// Key identifies a point for deduplication: the same node, metric and instant.
type Key struct {
	Node   string
	Metric string
	TSNano int64
}

// Key returns the deduplication key of p.
func (p Point) Key() Key {
	return Key{Node: p.Node, Metric: p.Metric, TSNano: p.TS.UnixNano()}
}
