// SPDX-License-Identifier: MIT OR Apache-2.0

package cycle

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// Save writes state to path atomically: write a temp file in the same directory, fsync it,
// rename it over the target, then fsync the directory. A crash leaves either the old or the new
// state, never a torn file.
func Save(path string, s *State) error {
	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return fmt.Errorf("cycle: encode: %w", err)
	}
	dir := filepath.Dir(path)
	tmp, err := os.CreateTemp(dir, ".cycle-*.tmp")
	if err != nil {
		return fmt.Errorf("cycle: temp file: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op after a successful rename

	if _, err := tmp.Write(append(data, '\n')); err != nil {
		tmp.Close()
		return fmt.Errorf("cycle: write: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return fmt.Errorf("cycle: sync: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("cycle: close: %w", err)
	}
	if err := os.Rename(tmpName, path); err != nil {
		return fmt.Errorf("cycle: rename: %w", err)
	}
	if d, err := os.Open(dir); err == nil {
		_ = d.Sync() // best effort: not supported on every platform
		d.Close()
	}
	return nil
}

// Load reads state written by Save.
func Load(path string) (*State, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("cycle: read: %w", err)
	}
	var s State
	if err := json.Unmarshal(data, &s); err != nil {
		return nil, fmt.Errorf("cycle: decode %s: %w", path, err)
	}
	if err := s.Template.Validate(); err != nil {
		return nil, err
	}
	if s.Index < 0 || s.Index > len(s.Template.Stages) {
		return nil, fmt.Errorf("cycle: stage index %d out of range", s.Index)
	}
	return &s, nil
}
