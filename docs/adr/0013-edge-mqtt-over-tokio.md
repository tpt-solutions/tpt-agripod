# 0013: Edge MQTT runs minimq over tokio, with embassy-time's generic timer queue

Status: Accepted. Refines ADR 0006: no std-embedded-nal, and a specific timer setup.
Date: 2026-11-11

## Context

ADR 0006 chose minimq and expected std-embedded-nal for Linux TCP. Building it showed two things:

- minimq 0.13 is sans-IO and async. It takes an established `embedded_io_async` 0.7 stream, so
  no NAL adapter is needed. tokio's TcpStream, wrapped by `embedded-io-adapters` 0.7 (`tokio-1`),
  is the transport.
- minimq uses embassy-time for keep-alive and timeouts. On std, embassy-time's executor timer
  queue (`integrated-timers`) needs an embassy executor, and tokio does not provide one. Linking
  fails with an unresolved `__embassy_time_queue_item_from_waker`.

## Decision

- Transport: tokio TcpStream, through `embedded-io-adapters` 0.7.
- Timer queue: embassy-time with `std` and `generic-queue-64`. Its waker queue does not need an
  executor, and the std driver wakes it from a thread.
- Runtime: tokio (`rt`, `net`, `time`, `macros`, `io-util`, `sync`), MIT. The whole tree is MIT,
  Apache-2.0, BSD-3, Unlicense or Unicode-3.0, and cargo-deny passes.

## Consequences

- The node's logic (`node::Node`) never touches the network. `node::run` is the only transport code,
  so the protocol rules are tested without a broker. The broker tests need `AGRIPOD_TEST_BROKER`,
  and the CI job `edge-broker` runs them against `fleet/testbroker`.
- Keep-alive timing: minimq's Terminate-like behaviour is not relevant on the node. The node relies
  on the broker's keep-alive detection for last-will, and the tests use a 30 s keep-alive as configured.
- Replacing tokio or embassy-time means revisiting this record.
