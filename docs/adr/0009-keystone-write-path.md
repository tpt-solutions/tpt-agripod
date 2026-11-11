# 0009: Keystone write path: batched INSERT now, COPY after the SDK gains it

Status: Accepted
Date: 2026-11-11

## Context

ADR 0005 says telemetry writes to Keystone use `COPY`. Checking the Keystone repository
(`tpt-solutions/tpt-keystone-db`, commit `53e2017`) changed three things:

1. **The server supports `COPY FROM STDIN`**, but only in text format over the simple query
   protocol (`docs/sql-reference.md`).
2. **The Go SDK (`sdk-go/`) has no COPY sub-protocol.** It has `Exec`, `Query` and
   `SimpleQuery`. It does not send `CopyInResponse`, `CopyData` or `CopyDone`.
3. **The SDK's error type carries only a message.** `ServerError` has a `Message` field and no
   SQLSTATE. A duplicate primary key is reported only as
   `duplicate key value violates unique constraint "<table>_pkey"`.

The server also supports `BEGIN`/`COMMIT`/`ROLLBACK`, and it rejects duplicate primary keys.
It has no `ON CONFLICT`.

The SDK's module path is `github.com/tpt/sdk-go`, but the repository is
`github.com/tpt-solutions/tpt-keystone-db/sdk-go`. A plain `go get` of the repository path will
not resolve.

## Decision

1. **Write with batched multi-row `INSERT` inside a transaction, not `COPY`.** Each chunk of at
   most 500 points (2,000 parameters, under the 65,535 protocol limit) is one `BEGIN`/`INSERT`/`COMMIT`.
2. **Idempotency comes from the primary key `(node, metric, ts)`.** A resent point collides with
   its stored copy. On a duplicate-key error the sink rolls back and writes the chunk's rows one
   at a time, treating each duplicate as already stored. Any other error is returned and the
   batcher retries it.
3. **Detect duplicates by message text**, because the SDK exposes no SQLSTATE. The match is a
   single documented substring, and it is isolated in `IsDuplicate`.
4. **Keep the sink independent of the SDK.** It depends on a small `Executor` interface. An
   adapter in the `sdk-go` module (or a vendored copy, see below) maps `Conn.Exec` onto it.
5. **Add `COPY` to the SDK as upstream work.** It is the path to bulk ingest. The change belongs
   in `tpt-keystone-db`, under its MIT OR Apache-2.0 licence. This repo does not carry a fork.
6. **The module path problem is open.** Options are: a `replace` directive to a pinned checkout,
   or an upstream fix to the module path. Both need the maintainer's decision. Until then the SDK
   is not imported.

## Consequences

- Telemetry writes work with the SDK as it is today. Throughput is lower than `COPY` would give.
- Duplicate detection depends on server message text. A server message change breaks the
  fallback, and the test `TestKeystoneSinkStoresOnceWhenTheSameBatchIsResent` would not catch it
  against the real server. A live test must confirm the message text.
- Timestamps are sent as RFC 3339 text parameters. Whether the server coerces text into
  `TIMESTAMPTZ` is not confirmed. The live test must confirm it, or the sink will need
  `CAST($n AS TIMESTAMPTZ)`.
- Retention is set on the `USING TIME` index. The value is a placeholder until the retention
  policy is decided.
- Follow-up: a live integration test against a Keystone instance, run in CI with a service
  container.
