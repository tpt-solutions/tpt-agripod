// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"context"
	"errors"
	"fmt"
	"regexp"
	"strings"
)

// Executor runs one SQL statement with positional parameters and reports rows affected.
// KeystoneSession implements it over the TPT Keystone SDK (keystone_sdk.go).
type Executor interface {
	Exec(ctx context.Context, sql string, args ...any) (rowsAffected int64, err error)
}

// SeriesTable is the default Keystone table for sensor points.
const SeriesTable = "sensor_series"

// DefaultRetention is how long raw points are kept on the Chronos index (ADR 0012).
const DefaultRetention = "90 days"

// tableName is the only form accepted for a table name. The name is placed in SQL text, so it is
// checked before use.
var tableName = regexp.MustCompile(`^[a-z_][a-z0-9_]{0,62}$`)

// SchemaStatements returns the DDL for a series table: the table keyed on (node, metric, ts), and
// its Chronos (USING TIME) index with the given retention.
func SchemaStatements(table, retention string) ([]string, error) {
	if !tableName.MatchString(table) {
		return nil, fmt.Errorf("telemetry: table name %q is not a plain identifier", table)
	}
	if retention == "" || strings.ContainsAny(retention, "'\\") {
		return nil, fmt.Errorf("telemetry: retention %q is not a plain interval", retention)
	}
	return []string{
		fmt.Sprintf(`CREATE TABLE %s (
  node TEXT NOT NULL,
  metric TEXT NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  value FLOAT8 NOT NULL,
  PRIMARY KEY (node, metric, ts)
)`, table),
		fmt.Sprintf(`CREATE INDEX ON %s USING TIME(ts) WITH (interval = '1 hour', retention = '%s', value = 'value')`,
			table, retention),
	}, nil
}

// KeystoneSink writes batches to TPT Keystone.
//
// Each chunk is one transaction holding one multi-row INSERT. Keystone rejects a duplicate
// primary key, and the Go SDK does not expose SQLSTATE, so duplicates are matched on the server
// message. If a chunk hits a duplicate, the sink rolls back and writes its rows one at a time,
// skipping the duplicates. Re-sent points therefore land once. Other errors are returned, and
// the batcher retries them.
//
// COPY is not used yet. The server supports COPY FROM STDIN, but the Go SDK has no COPY
// sub-protocol. Adding it to the SDK is the path to bulk ingest (ADR 0009).
type KeystoneSink struct {
	ex      Executor
	table   string
	maxRows int
}

// NewKeystoneSink returns a sink that writes at most maxRows points per INSERT into table.
// Each row uses four parameters, so maxRows must keep 4*maxRows under the protocol's 65535 limit.
func NewKeystoneSink(ex Executor, table string, maxRows int) (*KeystoneSink, error) {
	if !tableName.MatchString(table) {
		return nil, fmt.Errorf("telemetry: table name %q is not a plain identifier", table)
	}
	if maxRows < 1 || maxRows > 16_000 {
		return nil, fmt.Errorf("telemetry: maxRows %d out of range 1..16000", maxRows)
	}
	if ex == nil {
		return nil, errors.New("telemetry: keystone sink needs an executor")
	}
	return &KeystoneSink{ex: ex, table: table, maxRows: maxRows}, nil
}

const duplicateMarker = "duplicate key value violates unique constraint"

// IsDuplicate reports whether err is Keystone's duplicate-primary-key error.
func IsDuplicate(err error) bool {
	return err != nil && strings.Contains(err.Error(), duplicateMarker)
}

// Write stores the batch, in chunks of at most maxRows.
func (k *KeystoneSink) Write(ctx context.Context, batch []Point) error {
	for start := 0; start < len(batch); start += k.maxRows {
		end := min(start+k.maxRows, len(batch))
		if err := k.writeChunk(ctx, batch[start:end]); err != nil {
			return err
		}
	}
	return nil
}

func (k *KeystoneSink) writeChunk(ctx context.Context, chunk []Point) error {
	if len(chunk) == 0 {
		return nil
	}
	if _, err := k.ex.Exec(ctx, "BEGIN"); err != nil {
		return fmt.Errorf("telemetry: begin: %w", err)
	}
	_, err := k.ex.Exec(ctx, insertSQL(k.table, len(chunk)), insertArgs(chunk)...)
	if err == nil {
		if _, cerr := k.ex.Exec(ctx, "COMMIT"); cerr != nil {
			return fmt.Errorf("telemetry: commit: %w", cerr)
		}
		return nil
	}
	// The statement failed, so nothing in it was stored. Roll back before doing anything else.
	_, _ = k.ex.Exec(ctx, "ROLLBACK") // best effort: the next statement reports a broken session anyway
	if !IsDuplicate(err) {
		return fmt.Errorf("telemetry: insert %d points: %w", len(chunk), err)
	}
	return k.writeRowsSkippingDuplicates(ctx, chunk)
}

func (k *KeystoneSink) writeRowsSkippingDuplicates(ctx context.Context, chunk []Point) error {
	for _, p := range chunk {
		_, err := k.ex.Exec(ctx, insertSQL(k.table, 1), insertArgs([]Point{p})...)
		if err != nil && !IsDuplicate(err) {
			return fmt.Errorf("telemetry: insert point %s/%s: %w", p.Node, p.Metric, err)
		}
	}
	return nil
}

func insertSQL(table string, rows int) string {
	var b strings.Builder
	fmt.Fprintf(&b, "INSERT INTO %s (node, metric, ts, value) VALUES ", table)
	for i := range rows {
		if i > 0 {
			b.WriteString(", ")
		}
		fmt.Fprintf(&b, "($%d, $%d, $%d, $%d)", 4*i+1, 4*i+2, 4*i+3, 4*i+4)
	}
	return b.String()
}

func insertArgs(points []Point) []any {
	args := make([]any, 0, 4*len(points))
	for _, p := range points {
		args = append(args, p.Node, p.Metric, p.TS.Format("2006-01-02T15:04:05.999999999Z07:00"), p.Value)
	}
	return args
}
