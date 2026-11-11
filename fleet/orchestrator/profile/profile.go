// SPDX-License-Identifier: MIT OR Apache-2.0

// Package profile loads and validates Crop Profiles (profiles/crop-profile.schema.json).
//
// This is a hand-written check of the schema's rules, not a full JSON Schema validator. The
// decoder rejects unknown fields as the schema does (additionalProperties: false). The checks
// cover the required fields, enums, patterns, numeric ranges, and the control names and limits
// the edge will accept. A profile that passes here can still be refused by the edge, so the edge
// stays the authority.
package profile

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"regexp"
)

// PumpDutyMax is the largest pump duty a profile may request. It must match the pump contract in
// edge/contracts/pump.contract. A test in this package keeps the two in step.
const PumpDutyMax = 0.15

var (
	idPattern      = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{1,62}$`)
	versionPattern = regexp.MustCompile(`^[0-9]+\.[0-9]+\.[0-9]+$`)
	controlPattern = regexp.MustCompile(`^[a-z][a-z0-9_]{1,62}$`)
)

// Range is a [min, max] band. Both ends are required by the schema, so pointers detect absence.
type Range struct {
	Min *float64 `json:"min"`
	Max *float64 `json:"max"`
}

// NPK is the nutrient ratio.
type NPK struct {
	N *float64 `json:"n"`
	P *float64 `json:"p"`
	K *float64 `json:"k"`
}

// Targets are the environmental targets. Every field is required by the schema.
type Targets struct {
	VPD          *Range   `json:"vpd_kpa"`
	DLI          *float64 `json:"dli_mol_m2_day"`
	Photoperiod  *float64 `json:"photoperiod_h"`
	RootZoneTemp *Range   `json:"root_zone_temp_c"`
	NPK          *NPK     `json:"npk_ratio,omitempty"`
	EC           *Range   `json:"ec_s_m,omitempty"`
	PH           *Range   `json:"ph,omitempty"`
}

// DripSchedule is the irrigation schedule.
type DripSchedule struct {
	IntervalMin *float64 `json:"interval_min"`
	VolumeL     *float64 `json:"volume_l"`
}

// Control is one named setpoint the edge applies.
type Control struct {
	Value       *float64 `json:"value"`
	Unit        string   `json:"unit"`
	Min         *float64 `json:"min,omitempty"`
	Max         *float64 `json:"max,omitempty"`
	ContractRef string   `json:"contract_ref,omitempty"`
}

// Profile is one Crop Profile.
type Profile struct {
	SchemaVersion  string             `json:"schema_version"`
	Kind           string             `json:"kind"`
	ProfileID      string             `json:"profile_id"`
	ProfileVersion string             `json:"profile_version"`
	Name           string             `json:"name"`
	Description    string             `json:"description,omitempty"`
	CartridgeTypes []string           `json:"cartridge_types"`
	Targets        *Targets           `json:"targets"`
	DripSchedule   *DripSchedule      `json:"drip_schedule,omitempty"`
	Controls       map[string]Control `json:"controls"`
}

// knownControls are the control names the edge can map to a controller and a contract.
var knownControls = map[string]bool{
	"pump_duty":   true,
	"led_dimming": true,
}

// Parse decodes and validates a profile. Unknown fields are an error.
func Parse(data []byte) (*Profile, error) {
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	var p Profile
	if err := dec.Decode(&p); err != nil {
		return nil, fmt.Errorf("profile: decode: %w", err)
	}
	if dec.More() {
		return nil, errors.New("profile: trailing data after the JSON object")
	}
	if err := p.Validate(); err != nil {
		return nil, err
	}
	return &p, nil
}

// Load reads and validates a profile file.
func Load(path string) (*Profile, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("profile: read: %w", err)
	}
	return Parse(data)
}

// Validate checks the profile against the schema rules and the edge's limits.
func (p *Profile) Validate() error {
	var errs []error
	add := func(format string, args ...any) { errs = append(errs, fmt.Errorf(format, args...)) }

	if p.SchemaVersion != "0" {
		add("schema_version must be \"0\", got %q", p.SchemaVersion)
	}
	switch p.Kind {
	case "plant":
	case "culture":
		add("kind \"culture\" is reserved and not supported in schema v0")
	default:
		add("kind must be \"plant\", got %q", p.Kind)
	}
	if !idPattern.MatchString(p.ProfileID) {
		add("profile_id %q must match %s", p.ProfileID, idPattern)
	}
	if !versionPattern.MatchString(p.ProfileVersion) {
		add("profile_version %q must be MAJOR.MINOR.PATCH", p.ProfileVersion)
	}
	if n := len(p.Name); n < 1 || n > 120 {
		add("name must be 1 to 120 characters")
	}
	if len(p.Description) > 1000 {
		add("description is longer than 1000 characters")
	}
	if len(p.CartridgeTypes) == 0 {
		add("cartridge_types needs at least one entry")
	}
	seen := map[string]bool{}
	for _, c := range p.CartridgeTypes {
		if c != "A" && c != "B" && c != "C" {
			add("cartridge type %q must be A, B or C", c)
		}
		if seen[c] {
			add("cartridge type %q is repeated", c)
		}
		seen[c] = true
	}
	if p.Targets == nil {
		add("targets is required")
	} else {
		errs = append(errs, p.Targets.validate()...)
	}
	if p.DripSchedule != nil {
		if p.DripSchedule.IntervalMin == nil || *p.DripSchedule.IntervalMin <= 0 {
			add("drip_schedule.interval_min must be positive")
		}
		if p.DripSchedule.VolumeL == nil || *p.DripSchedule.VolumeL < 0 {
			add("drip_schedule.volume_l must be zero or positive")
		}
	}
	if len(p.Controls) == 0 {
		add("controls needs at least one entry")
	}
	for name, c := range p.Controls {
		errs = append(errs, validateControl(name, c)...)
	}
	return errors.Join(errs...)
}

func (t *Targets) validate() []error {
	var errs []error
	add := func(format string, args ...any) { errs = append(errs, fmt.Errorf(format, args...)) }
	checkRange := func(name string, r *Range, lo, hi float64) {
		if r == nil || r.Min == nil || r.Max == nil {
			add("targets.%s needs both min and max", name)
			return
		}
		if !finite(*r.Min) || !finite(*r.Max) || *r.Min < lo || *r.Max > hi || *r.Min > *r.Max {
			add("targets.%s must satisfy %g <= min <= max <= %g", name, lo, hi)
		}
	}
	checkRange("vpd_kpa", t.VPD, 0, 5)
	checkRange("root_zone_temp_c", t.RootZoneTemp, 0, 60)
	if t.EC != nil {
		checkRange("ec_s_m", t.EC, 0, 10)
	}
	if t.PH != nil {
		checkRange("ph", t.PH, 0, 14)
	}
	if t.DLI == nil || !finite(*t.DLI) || *t.DLI < 0 || *t.DLI > 80 {
		add("targets.dli_mol_m2_day must be 0 to 80")
	}
	if t.Photoperiod == nil || !finite(*t.Photoperiod) || *t.Photoperiod < 0 || *t.Photoperiod > 24 {
		add("targets.photoperiod_h must be 0 to 24")
	}
	if t.NPK != nil {
		for name, v := range map[string]*float64{"n": t.NPK.N, "p": t.NPK.P, "k": t.NPK.K} {
			if v == nil || !finite(*v) || *v < 0 {
				add("targets.npk_ratio.%s must be zero or positive", name)
			}
		}
	}
	return errs
}

func validateControl(name string, c Control) []error {
	var errs []error
	add := func(format string, args ...any) { errs = append(errs, fmt.Errorf(format, args...)) }
	if !controlPattern.MatchString(name) {
		add("control name %q must be lower_snake_case", name)
	}
	if !knownControls[name] {
		add("control %q is unknown to the edge", name)
	}
	if c.Value == nil || !finite(*c.Value) {
		add("control %q needs a finite value", name)
	}
	if c.Unit == "" {
		add("control %q needs a unit", name)
	}
	if c.Min != nil && c.Max != nil && *c.Min > *c.Max {
		add("control %q has min above max", name)
	}
	if c.Value != nil && finite(*c.Value) {
		if c.Min != nil && *c.Value < *c.Min {
			add("control %q value %g is below min %g", name, *c.Value, *c.Min)
		}
		if c.Max != nil && *c.Value > *c.Max {
			add("control %q value %g is above max %g", name, *c.Value, *c.Max)
		}
	}
	if name == "pump_duty" && c.Max != nil && *c.Max > PumpDutyMax {
		add("control pump_duty max %g exceeds the edge pump contract limit %g", *c.Max, PumpDutyMax)
	}
	return errs
}

func finite(v float64) bool {
	return !math.IsNaN(v) && !math.IsInf(v, 0)
}
