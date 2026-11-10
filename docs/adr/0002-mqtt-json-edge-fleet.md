# 0002: MQTT with JSON payloads between edge and fleet

Status: Accepted
Date: 2026-11-11

## Context

The edge node must run offline and reconnect cleanly. The fleet side needs a pub/sub transport with last-will support. Payloads must be readable in the field and easy to inspect. The licence policy excludes Apache-only stacks such as gRPC, protobuf and NATS.

## Decision

Use MQTT for transport and JSON for payloads. The topic map and message shapes are in [../mqtt-topics.md](../mqtt-topics.md). Crop Profiles use the JSON Schema in [../../profiles/crop-profile.schema.json](../../profiles/crop-profile.schema.json).

## Consequences

- Payloads are larger than binary encodings. Telemetry is batched to compensate.
- Schema validation happens on both ends, and the edge rejects unsafe profiles.
- The MQTT client and broker must satisfy the licence policy. Eclipse Paho is EPL/EDL and is excluded unless re-licensed. See the open questions in `todo.md`.
