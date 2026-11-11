// SPDX-License-Identifier: MIT OR Apache-2.0

package wire

import (
	"fmt"
	"strings"
)

// Topic kinds below the node prefix. These match docs/mqtt-topics.md.
const (
	KindProfilePush = "profile/push"
	KindProfileAck  = "profile/ack"
	KindTelemetry   = "telemetry"
	KindCommand     = "command"
	KindCommandAck  = "command/ack"
	KindStatus      = "status"
)

// SubscribeTelemetry matches telemetry from every node on every site.
const SubscribeTelemetry = "agripod/+/+/telemetry"

// SubscribeProfileAck matches profile acks from every node.
const SubscribeProfileAck = "agripod/+/+/profile/ack"

// SubscribeStatus matches status messages (and last-will) from every node.
const SubscribeStatus = "agripod/+/+/status"

var kinds = map[string]bool{
	KindProfilePush: true,
	KindProfileAck:  true,
	KindTelemetry:   true,
	KindCommand:     true,
	KindCommandAck:  true,
	KindStatus:      true,
}

// Topic builds `agripod/{site}/{node}/{kind}`. Site and node must be slugs, and kind one of the
// Kind constants.
func Topic(site, node, kind string) (string, error) {
	if !slugPattern.MatchString(site) {
		return "", fmt.Errorf("wire: site %q is not a slug", site)
	}
	if !slugPattern.MatchString(node) {
		return "", fmt.Errorf("wire: node %q is not a slug", node)
	}
	if !kinds[kind] {
		return "", fmt.Errorf("wire: unknown topic kind %q", kind)
	}
	return "agripod/" + site + "/" + node + "/" + kind, nil
}

// ParseTopic splits a concrete topic into site, node and kind. It rejects wildcards.
func ParseTopic(topic string) (site, node, kind string, err error) {
	parts := strings.SplitN(topic, "/", 4)
	if len(parts) != 4 || parts[0] != "agripod" {
		return "", "", "", fmt.Errorf("wire: topic %q is not agripod/{site}/{node}/{kind}", topic)
	}
	site, node, kind = parts[1], parts[2], parts[3]
	if strings.ContainsAny(topic, "+#") {
		return "", "", "", fmt.Errorf("wire: topic %q contains a wildcard", topic)
	}
	if !slugPattern.MatchString(site) || !slugPattern.MatchString(node) {
		return "", "", "", fmt.Errorf("wire: topic %q has a bad site or node", topic)
	}
	if !kinds[kind] {
		return "", "", "", fmt.Errorf("wire: topic %q has unknown kind %q", topic, kind)
	}
	return site, node, kind, nil
}
