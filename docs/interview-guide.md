# Explaining IMSCAP in a senior telecom engineering interview

## Positioning

IMSCAP is an ongoing Rust interworking project connecting a TAS using Diameter Ro to a prepaid charging platform using CAMEL CAP v2. The current repository demonstrates the charging state machine and an in-process simulator. It is not a production deployment or a complete SS7 stack.

Present professional experience separately from this portfolio project. Describe personal contributions accurately, including tool-assisted implementation when asked. Only claim code ownership that you can explain, modify and debug. Do not associate employer deployments, subscriber counts or traffic volumes with this repository.

## A short introduction to adapt

“I have 13 years of telecom engineering experience. This project explores a practical interworking problem: a TAS uses Diameter Ro for online charging, while an existing prepaid platform exposes CAMEL CAP v2. IMSCAP sits between them. The difficult part is preserving charging and call-control semantics across two different stateful protocols, particularly under retries, timeouts and failures.

“The current prototype demonstrates initial authorization, quota renewal, denial and termination. The TAS simulator exchanges encoded Diameter messages; the CAMEL side currently uses normalized operations. I am treating protocol interoperability, durable recovery and measured performance as separate engineering milestones. I would not describe it as production-ready until those milestones have evidence.”

## Explain one call in detail

1. TAS sends CCR-I. IMSCAP validates the supported AVP profile and starts an InitialDP intent toward the SCP.
2. In the simulator's supported profile, IMSCAP waits for both ApplyCharging and Continue before returning a successful CCA-I with granted seconds. Receiving just one is insufficient.
3. TAS reports consumption through CCR-U. IMSCAP produces an ApplyChargingReport intent and waits for the next SCP grant before answering.
4. TAS sends CCR-T with final usage. IMSCAP produces the final report and a disconnect-event intent. The simulator answers after normal dialogue closure; closure alone does not prove a financial debit.
5. A ReleaseCall received during a pending authorization produces a credit-denial answer. Other release scenarios still need explicit handling.

Be ready to explain why partial updates, answer/disconnect events, BCSM monitoring, tariff changes and unsolicited release need more mapping work. InitialDP cannot be implemented on the wire with only the subscriber field used by this simulation.

## Proposed production architecture — not implemented

```text
TAS peers
  |
Diameter peer handling, authentication, validation and admission limits
  |
Session routing -> bounded queues -> one state owner per session/shard
  |
Atomic commit of state + replay receipts + outgoing actions
  |
Outbox dispatcher -> CAP v2 / TCAP / SCCP / M3UA / SCTP -> SCP

Quorum-replicated state and fenced ownership underpin failover.
Metrics, protocol timers and reconciliation surround the processing path.
```

The existing opposite-direction storage prototype is not wired into IMSCAP. Moving its storage pattern into this path requires new recovery tests; reusing a module name is not evidence of correctness.

## Decisions that demonstrate senior engineering judgment

| Concern | Proposed decision | Tradeoff and evidence needed |
|---|---|---|
| Session concurrency | Route a session to one owner; run independent sessions in parallel | Avoid global locks, but measure hot shards and queue delay |
| Durable processing | Commit state, replay receipt and outbox before exposing actions | Adds storage latency; evaluate bounded group commit without acknowledging uncommitted work |
| Retransmission | Return the recorded decision for an identical request; reject changed payloads under the same identity | Requires bounded retention tied to retry windows and correct transport identifiers |
| Ambiguous remote execution | Preserve correlation and reconcile; do not assume a timed-out CAP operation was never executed | Local deduplication does not guarantee exactly-once remote charging |
| Overload | Bound connections, frame size, queued work, sessions and outstanding invokes | Capacity rejection must be explicit; unrestricted queues turn load into latency and memory failures |
| Failover | Replicate state and fence the old owner before takeover | A healthy standby is insufficient if TCAP routing/dialogue state cannot follow ownership |
| Network partition | Prefer a single authorized charging owner; stop unsafe progress when ownership cannot be established | Availability may decrease to prevent conflicting charging actions |
| CAP transport | Evaluate a mature signalling transport against a native Rust implementation | Native code offers control; a complete SS7 stack has a substantial interoperability and maintenance cost |
| Rust | Use ownership, bounded data structures and explicit state transitions | Rust does not itself solve deadlocks, overload, protocol errors or distributed consistency |
| Security | Authenticate configured peers, enforce protocol limits, minimize subscriber data in logs, fuzz parsers | Deployment-specific Diameter and SIGTRAN security must be designed separately |

## Failure questions to rehearse

**What happens if IMSCAP crashes after sending a CAP operation?**

The peer might have executed it even if no response was stored. Durable intent and dialogue/invoke correlation help recovery, but blind resend may be unsafe. The production design needs operation-specific recovery and reconciliation. The current simulator does not implement this recovery.

**Can a standby preserve every active call?**

Not with database replication alone. It also needs ownership fencing, recoverable dialogue/invoke state, protocol timers and routing that sends both TAS and SCP traffic to the correct owner. Define active-call survival separately from service availability and data-loss objectives.

**Why not use a large thread pool?**

It does not resolve ordering for one call. Use per-session serialization with concurrency across calls. Size queues and workers from measured service times and burst requirements; measure time waiting in queues as part of latency.

**What happens if the SCP is slow or unreachable?**

Use bounded outstanding work and explicit deadlines. A charging timeout cannot invent a quota. The approved charging policy must define the TAS-visible result and handling of already-granted credit. Delayed replies and cleanup need tests. These timers and policies are still pending.

**How fast is it?**

IMSCAP has no published end-to-end throughput result yet. The earlier 149-events/second result measures a different durable prototype and must not be used as an IMSCAP number. Present hardware, workload, duration, offered load, achieved throughput, latency percentiles and error rate together when results exist.

## Evidence required before calling it production-ready

1. Implement a documented CAP v2 profile with independent wire vectors, malformed-message tests and a separate peer implementation. Cover TCAP dialogue/invoke lifecycle and the selected SS7 transport.
2. Implement real Diameter peer lifecycle and the chosen TAS Ro profile, including error responses, reconnects, retries, watchdogs and bounded resources. Do not infer full Ro conformance from the minimal simulator AVPs.
3. Add persistent IMSCAP sessions, receipts and outbox, restart recovery, timeout handling and retention. Test crashes before/after each commit and send boundary, including uncertain remote execution.
4. Measure open-loop load so a slowing server does not silently reduce offered traffic. Include initial/update/termination mixes, concurrent calls, slow SCP replies, bursts, soak tests and disk pressure. Report p50/p95/p99, failures, queue depths, CPU, memory and storage latency.
5. Set throughput, concurrent-call capacity, latency, RPO and RTO targets with a stated deployment budget. Test host loss, network partitions, old-owner recovery and active-dialogue takeover against those targets.
6. Add operational metrics, readiness checks, configuration validation, deployment/rollback instructions, dependency review and incident runbooks. Validate log redaction and parser robustness.

These are completion gates, not claims about the current implementation.

## Five-minute repository demonstration

```sh
cargo run --locked --example imscap_sim
cargo test --locked --test imscap
```

Start with the network direction, then trace one request through `src/diameter.rs` and `src/imscap.rs`. Show the test that permits ApplyCharging and Continue in either order while withholding early credit. Show rejection of an out-of-sequence request and excessive usage. End with one unresolved failure case and explain the proposed recovery design.

The whole repository currently has 21 integration tests, only three of which are IMSCAP-specific test functions. The remaining tests cover the shared Diameter codec and the earlier opposite-direction experiment. Test count alone is not a production-quality measure.

Reference for Diameter credit-control behavior: [RFC 8506](https://www.rfc-editor.org/rfc/rfc8506.html). See [the implemented simulator profile](imscap-profile.md) for current boundaries.
