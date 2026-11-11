// SPDX-License-Identifier: MIT OR Apache-2.0

package wire

import (
	"strings"
	"testing"
)

const goodEnv = `"msg_id":"0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d10","ts":"2026-11-11T02:15:00Z","site_id":"wgtn-bay-01","node_id":"nuc-01"`

func TestTelemetryExampleFromTheTopicMapDecodes(t *testing.T) {
	js := `{` + goodEnv + `,"seq":10452,"samples":[
	  {"channel":"air_temp","ts":"2026-11-11T02:14:58Z","value":21.4,"unit":"degC","quality":"good"},
	  {"channel":"rh","ts":"2026-11-11T02:14:58Z","value":0.68,"unit":"frac","quality":"good"},
	  {"channel":"soil_moisture","device_id":"esp32-z1","ts":"2026-11-11T02:14:40Z","value":0.31,"unit":"frac","quality":"stale"}
	]}`
	m, err := Decode[Telemetry]([]byte(js))
	if err != nil {
		t.Fatalf("documented example rejected: %v", err)
	}
	if m.Seq != 10452 || len(m.Samples) != 3 || m.Samples[2].DeviceID != "esp32-z1" {
		t.Fatalf("decoded %+v", m)
	}
}

func TestStatusAndCommandExamplesDecode(t *testing.T) {
	if _, err := Decode[Status]([]byte(`{` + goodEnv + `,"state":"safe_state","profile_id":"lettuce-v1","profile_version":"1.0.0","uptime_s":86400}`)); err != nil {
		t.Fatalf("status: %v", err)
	}
	if _, err := Decode[Command]([]byte(`{` + goodEnv + `,"ttl_s":30,"actuator_id":"pump-main","action":"set_duty","value":0.10}`)); err != nil {
		t.Fatalf("command: %v", err)
	}
	if _, err := Decode[CommandAck]([]byte(`{` + goodEnv + `,"ack_of":"0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d11","status":"rejected","reason":"contract violated: pump_duty_cycle <= 0.15"}`)); err != nil {
		t.Fatalf("command ack: %v", err)
	}
}

func TestProfileAckRules(t *testing.T) {
	ack := `{` + goodEnv + `,"ack_of":"0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d11","profile_id":"lettuce-v1","profile_version":"1.0.0",`
	if _, err := Decode[ProfileAck]([]byte(ack + `"status":"applied"}`)); err != nil {
		t.Fatalf("applied: %v", err)
	}
	if _, err := Decode[ProfileAck]([]byte(ack + `"status":"rejected"}`)); err == nil {
		t.Error("rejected without errors accepted")
	}
	if _, err := Decode[ProfileAck]([]byte(ack + `"status":"rejected","errors":[{"path":"/targets/vpd_kpa","message":"above max"}]}`)); err != nil {
		t.Errorf("rejected with reason: %v", err)
	}
	if _, err := Decode[ProfileAck]([]byte(ack + `"status":"maybe"}`)); err == nil {
		t.Error("unknown status accepted")
	}
}

func TestEnvelopeRules(t *testing.T) {
	cases := map[string]string{
		"not a uuid":      `"msg_id":"nope","ts":"2026-11-11T02:15:00Z","site_id":"s-1","node_id":"n-1"`,
		"local time":      `"msg_id":"0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d10","ts":"2026-11-11T02:15:00+12:00","site_id":"s-1","node_id":"n-1"`,
		"missing ts":      `"msg_id":"0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d10","site_id":"s-1","node_id":"n-1"`,
		"upper-case node": `"msg_id":"0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d10","ts":"2026-11-11T02:15:00Z","site_id":"s-1","node_id":"NUC-01"`,
	}
	for name, env := range cases {
		js := `{` + env + `,"seq":1,"samples":[{"channel":"rh","ts":"2026-11-11T02:15:00Z","value":0.5,"unit":"frac","quality":"good"}]}`
		if _, err := Decode[Telemetry]([]byte(js)); err == nil {
			t.Errorf("%s: accepted", name)
		}
	}
}

func TestTelemetrySampleRules(t *testing.T) {
	sample := func(s string) string {
		return `{` + goodEnv + `,"seq":1,"samples":[` + s + `]}`
	}
	good := `{"channel":"rh","ts":"2026-11-11T02:15:00Z","value":0.5,"unit":"frac","quality":"good"}`
	if _, err := Decode[Telemetry]([]byte(sample(good))); err != nil {
		t.Fatalf("good sample rejected: %v", err)
	}
	bad := map[string]string{
		"bad quality": strings.Replace(good, `"good"`, `"fine"`, 1),
		"no unit":     strings.Replace(good, `"frac"`, `""`, 1),
		"bad channel": strings.Replace(good, `"rh"`, `"RH"`, 1),
		"local ts":    strings.Replace(good, `"2026-11-11T02:15:00Z"`, `"2026-11-11T02:15:00+12:00"`, 1),
	}
	for name, s := range bad {
		if _, err := Decode[Telemetry]([]byte(sample(s))); err == nil {
			t.Errorf("%s: accepted", name)
		}
	}
	if _, err := Decode[Telemetry]([]byte(`{` + goodEnv + `,"seq":1,"samples":[]}`)); err == nil {
		t.Error("empty batch accepted")
	}
}

func TestCommandRules(t *testing.T) {
	if _, err := Decode[Command]([]byte(`{` + goodEnv + `,"ttl_s":0,"actuator_id":"pump-main","action":"set_duty","value":0.1}`)); err == nil {
		t.Error("zero ttl accepted")
	}
	if _, err := Decode[Command]([]byte(`{` + goodEnv + `,"ttl_s":30,"actuator_id":"Pump Main","action":"set_duty","value":0.1}`)); err == nil {
		t.Error("bad actuator id accepted")
	}
}

func TestStatusRules(t *testing.T) {
	if _, err := Decode[Status]([]byte(`{` + goodEnv + `,"state":"sleeping","uptime_s":1}`)); err == nil {
		t.Error("unknown state accepted")
	}
	if _, err := Decode[Status]([]byte(`{` + goodEnv + `,"state":"online","uptime_s":-5}`)); err == nil {
		t.Error("negative uptime accepted")
	}
}

func TestUnknownFieldsAreTolerated(t *testing.T) {
	// The topic map is additive: a newer edge may add fields, and older consumers must still read it.
	js := `{` + goodEnv + `,"state":"online","uptime_s":1,"new_field":{"x":1}}`
	if _, err := Decode[Status]([]byte(js)); err != nil {
		t.Fatalf("extra field broke decoding: %v", err)
	}
}

func TestTopics(t *testing.T) {
	topic, err := Topic("wgtn-bay-01", "nuc-01", KindTelemetry)
	if err != nil || topic != "agripod/wgtn-bay-01/nuc-01/telemetry" {
		t.Fatalf("topic %q %v", topic, err)
	}
	if _, err := Topic("Wgtn", "nuc-01", KindTelemetry); err == nil {
		t.Error("bad site accepted")
	}
	if _, err := Topic("s-1", "n-1", "bogus"); err == nil {
		t.Error("bad kind accepted")
	}
	site, node, kind, err := ParseTopic("agripod/s-1/n-1/command/ack")
	if err != nil || site != "s-1" || node != "n-1" || kind != KindCommandAck {
		t.Fatalf("parse: %s %s %s %v", site, node, kind, err)
	}
	for _, bad := range []string{"agripod/+/n-1/telemetry", "agripod/s-1/n-1/bogus", "other/s-1/n-1/status", "agripod/s-1/n-1"} {
		if _, _, _, err := ParseTopic(bad); err == nil {
			t.Errorf("%q accepted", bad)
		}
	}
}
