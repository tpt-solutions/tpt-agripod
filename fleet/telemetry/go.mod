// SPDX-License-Identifier: MIT OR Apache-2.0

module tpt-agripod-telemetry

go 1.22

require (
	github.com/mochi-mqtt/server/v2 v2.7.9
	github.com/yosssi/gmq v0.0.1
	tpt-agripod-wire v0.0.0
)

require (
	github.com/gorilla/websocket v1.5.0 // indirect
	github.com/rs/xid v1.4.0 // indirect
	github.com/tpt/sdk-go v0.0.0
	gopkg.in/yaml.v3 v3.0.1 // indirect
)

replace tpt-agripod-wire => ../wire

replace github.com/tpt/sdk-go => ../third_party/keystone-sdk-go
