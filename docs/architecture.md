# Architecture and remaining delivery work

## Implemented boundary

```text
local JSON / normalized CAP event
          |
    bounded queue (stable session hash)
          |
    single writer per shard
          |
    charging transition
          |
    atomic durable commit: session + receipt + outbox
          |
    normalized CCR / call-control action
```

The Diameter codec is a separate library boundary; it is not wired to the local harness or a network peer. The state engine accepts normalized answers. A future network adapter must validate the decoded message, peer, session, request number and transport correlation before producing that event.

## Intended call mapping

| Normalized event | Charging request | Successful response |
|---|---|---|
| CAP InitialDP | CCR INITIAL, number 0 | Monitor call events, ApplyCharging, Continue |
| CAP ApplyChargingReport | CCR UPDATE, next number, incremental used time | ApplyCharging with new quota |
| CAP disconnect report | CCR TERMINATION, final used time | End dialogue after settlement |

This is an intended adapter mapping, not a binary CAP implementation. CAP operation sequence, application context/version, event monitoring, leg selection, timer units and disconnect races need a profile agreed with the SSF. The opposite direction (Diameter network to legacy CAMEL charging platform) is not implemented.

## Production gates

1. Specify the network direction, CAP version, supported call scenarios, OCS AVPs/service context, TPS/concurrent-session targets, p99 latency budget and permitted RPO/RTO.
2. Implement CAP ASN.1/BER with captured-message and negative-vector tests; add TCAP dialogue/invoke handling, SCCP routing and SIGTRAN M3UA/SCTP. Specify whether transport is native Rust or an external signalling gateway.
3. Implement Diameter peer state, CER/CEA, watchdogs, TLS/mTLS, CCR/CCA dispatcher, bounded outstanding-request windows, timers and retry identity preservation. Add the required TS 32.299 service-specific AVPs and multi-service/final-unit/validity policies.
4. Handle answer/usage/disconnect races, timeout settlement, quota exhaustion, late answers, duplicate CAP operations and all terminal cleanup. Add durable retry scheduling and reconciliation.
5. Replicate each shard through a consensus-backed log; fence previous owners with monotonically increasing epochs. Couple ownership to protocol routing. Do not promote a standby based only on health checks.
6. Bound retention with documented OCS replay windows; keep termination tombstones long enough for late events. Add metrics, health/readiness, encrypted transport/storage policy and access control. Keep subscriber IDs out of routine logs.
7. Run sustained mixed-call load, wire-level interoperability, fuzzing, disk-full, power-loss, OCS timeout, host-loss and split-brain tests. Publish hardware, workload, latency percentiles, error rate and durability settings with results.

## Recovery rules

- Never grant credit before an accepted OCS response is durable.
- Never create a new request number merely because the original send timed out.
- Never assume an action was processed because it was written to a socket.
- Never promise zero lost calls while protocol state and charging state are not replicated together.
- Current file-lock ownership and local durable recovery are useful single-node building blocks, not the clustered design above.
