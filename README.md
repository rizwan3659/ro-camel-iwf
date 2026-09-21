# Ro / CAMEL charging interworking

An experimental Rust charging core for a CAMEL call-control adapter talking to a Diameter Ro OCS. The initial scope is a single voice service charged by elapsed seconds.

**This is a runnable charging-core prototype, not a complete SS7 stack or production charging gateway.** CAP events are normalized Rust/JSON types. There is no CAP ASN.1, TCAP, SCCP, M3UA/SCTP, live Diameter peer connection or automatic multi-node failover in this version. Do not connect this harness to a live charging network.

## What works

- Initial, update and termination credit requests with monotonic request numbers.
- Quota-based call decisions, zero-credit rejection, request/answer correlation and invalid-state rejection.
- Durable session state, event receipts and an outgoing-action outbox committed together using redb.
- Stable replay IDs, conflicting-replay rejection and recovery after process termination.
- Independent storage shards, bounded admission queues and graceful worker drain.
- A bounded Diameter frame/AVP encoder and decoder, minimal time-based CCR encoding, and validation of a narrow CCA subset.
- A local event harness, restart tests and a durable-core load benchmark.

## Run

Requires Rust 1.95 or newer. The project is an independent Cargo workspace.

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo run --release --locked -- lab ./data
```

Send one JSON event per line on standard input:

```json
{"session_id":"iwf.example;unique-call-1","event_id":"cap-invoke-1","event":{"InitialDp":{"subscriber":"1234567890"}}}
{"session_id":"iwf.example;unique-call-1","event_id":"cca-0","event":{"Answer":{"number":0,"success":true,"granted":60}}}
{"session_id":"iwf.example;unique-call-1","event_id":"cap-invoke-2","event":{"Usage":{"seconds":60}}}
{"session_id":"iwf.example;unique-call-1","event_id":"cca-1","event":{"Answer":{"number":1,"success":true,"granted":60}}}
{"session_id":"iwf.example;unique-call-1","event_id":"cap-invoke-3","event":{"Disconnect":{"seconds":12}}}
{"session_id":"iwf.example;unique-call-1","event_id":"cca-2","event":{"Answer":{"number":2,"success":true,"granted":0}}}
```

These are synthetic subscriber values. Usage is the incremental consumption of the current grant, not cumulative call duration. The CAP adapter must provide stable event IDs and globally unique session IDs, distinguish each report, normalize time units and handle disconnect/report races. An accepted event remains committed if its caller disappears.

The harness deliberately does not acknowledge output actions: printing JSON is not proof that a remote peer processed them. Stop it and inspect each database with:

```sh
cargo run --locked -- outbox ./data/shard-0.redb
```

The adapter-facing library exposes `Store::pending` and `Store::ack`. There is no live outbox dispatcher yet. A duplicate event returns its recorded actions; it does not re-enqueue acknowledged actions. Consumers must use action IDs to avoid repeating call-control side effects.

## Performance

```sh
cargo run --release --locked -- bench ./new-benchmark-dir 1000 4
```

The benchmark executes six events per synthetic call, including an acknowledgement transaction after each event. Four caller threads feed the requested number of independent durable shards. It reports calls/s, events/s and event latency percentiles, including queue and storage time. It excludes SS7, Diameter sockets, peer latency and replication. Use a new directory for each run. See [the recorded local results](docs/performance.md).

There is one serial writer per shard. The shard count is persisted and cannot be changed on restart. Queue overflow rejects work immediately. Session, outbox and receipt limits fail closed; retention/compaction is still required for sustained operation. Disk-full and storage failures propagate as errors.

## Availability and charging correctness

Local durability is implemented; clustered high availability is not. A file lock prevents two local owners from opening the same shard. That is **not** distributed fencing. Loss of the host/disk can lose availability and unreplicated data. An external owner election, quorum replication and peer routing are needed before a second node can safely take over.

The outbox provides at-least-once delivery. Exactly-once debit depends on OCS replay behaviour for `(Session-Id, CC-Request-Number)` and cannot be guaranteed by this process alone. A failed termination answer leaves the session pending for reconciliation. There is no retry timer, reconciliation worker or mid-request CAP disconnect handling yet. CAP adapters must enforce granted duration and stop calls when further credit is unavailable; the core alone cannot stop a call.

See [architecture and delivery plan](docs/architecture.md) for the remaining protocol and HA work.

## Protocol references

- [RFC 6733](https://www.rfc-editor.org/rfc/rfc6733.html): Diameter framing and base protocol.
- [RFC 8506](https://www.rfc-editor.org/rfc/rfc8506.html): credit-control state, request numbers and failover considerations.
- [3GPP TS 32.299](https://www.etsi.org/deliver/etsi_TS/132200_132299/132299/19.00.00_60/): Diameter charging applications.
- [3GPP TS 29.078](https://portal.3gpp.org/desktopmodules/Specifications/SpecificationDetails.aspx?specificationId=1597): CAMEL Application Part.

The generic CCR encoder is not a declaration of TS 32.299 conformance. Multi-service credit control, vendor-specific service information, final-unit actions and validity timers require an agreed operator profile and implementation. Unsupported CCA policies are rejected rather than silently interpreted.
