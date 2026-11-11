# 0015: The control loop runs regardless of the link; telemetry waits in a durable outbox

Status: Accepted
Date: 2026-11-11

## Context

The spec requires the edge to operate offline. A first version ran control cycles only while connected,
so a broker outage stopped measurement and control. That is wrong for a node that must keep its
safety behaviour independent of the network.

## Decision

- The node runs two tasks that do not wait on each other. The control task ticks on schedule. The
  link task keeps a connection, reconnecting after failures.
- Telemetry is written to a durable outbox (`outbox.rs`) before it is sent. Each message is removed
  only after the publish call succeeds. The outbox is capped by bytes; past the cap the oldest messages
  are dropped, and the gap shows in `seq`.
- The telemetry sequence resumes from a persisted high-water mark, so a restart never reuses a number.
- Live messages (status, acks) are not durable. They describe the present, so they are not replayed.
- Publishing respects minimq's send window. When it is full, the link polls for acknowledgements and
  retries, with a bounded wait and a bounded number of attempts. A permanent error is not retried.

## Consequences

- Measurement, control and telemetry buffering all continue through an outage. A test shows telemetry
  accumulating in order with the broker unreachable.
- Delivery is at least once while the session lives. A message published but not yet acknowledged when
  the process dies may be sent twice; the ingester drops duplicates by `msg_id`.
- Tested end to end (`tests/outage_drill.rs`): the broker process is killed while the node runs and started
  again. The outbox holds the outage telemetry on disk, drains after the restart, and live telemetry resumes.
- Acknowledgement is taken from the publish call, not from the broker's PUBACK, because the minimq API
  does not expose per-message completion here. Closing that gap needs a change in how the link waits.
