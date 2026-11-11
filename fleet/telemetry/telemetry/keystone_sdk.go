// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"context"
	"errors"
	"fmt"
	"sync"

	keystone "github.com/tpt/sdk-go"
)

// KeystoneSession runs statements on one Keystone connection and reconnects when the connection
// breaks. It implements Executor over the TPT Keystone Go SDK (vendored at fleet/third_party,
// pinned to commit 53e2017).
//
// A server error (for example a duplicate key) leaves the connection usable, so it does not
// trigger a reconnect. A broken connection (network failure, cancelled operation) is closed and
// replaced on the next call.
type KeystoneSession struct {
	addr   string
	params map[string]string

	mu   sync.Mutex
	conn *keystone.Conn
}

// NewKeystoneSession returns a session that will connect to addr on first use.
func NewKeystoneSession(addr string, params map[string]string) (*KeystoneSession, error) {
	if addr == "" {
		return nil, errors.New("telemetry: keystone address is empty")
	}
	return &KeystoneSession{addr: addr, params: params}, nil
}

var _ Executor = (*KeystoneSession)(nil)

// Exec runs one statement, connecting first if there is no live connection.
func (s *KeystoneSession) Exec(ctx context.Context, sql string, args ...any) (int64, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.conn == nil || s.conn.Broken() {
		s.dropLocked()
		c, err := keystone.Connect(ctx, s.addr, s.params)
		if err != nil {
			return 0, fmt.Errorf("telemetry: connect keystone %s: %w", s.addr, err)
		}
		s.conn = c
	}
	_, n, err := s.conn.Exec(ctx, sql, args...)
	if err != nil && s.conn.Broken() {
		s.dropLocked()
	}
	return n, err
}

// Close closes the connection, if one is open.
func (s *KeystoneSession) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.dropLocked()
}

func (s *KeystoneSession) dropLocked() error {
	if s.conn == nil {
		return nil
	}
	err := s.conn.Close()
	s.conn = nil
	return err
}

// InitSchema creates the sensor table and its Chronos index. Run once per database. It is not
// idempotent: running it twice fails on the existing table, which is the intended signal.
func InitSchema(ctx context.Context, ex Executor, table, retention string) error {
	stmts, err := SchemaStatements(table, retention)
	if err != nil {
		return err
	}
	for _, stmt := range stmts {
		if _, err := ex.Exec(ctx, stmt); err != nil {
			return fmt.Errorf("telemetry: schema: %w", err)
		}
	}
	return nil
}
