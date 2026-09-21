# Initial performance measurement

Measured on the development Windows host on 2026-09-21 using rustc 1.95.0, an optimized release build, redb default immediate durability, four caller threads and four storage shards.

```sh
cargo run --release --locked -- bench target/bench-four-shards 1000 4
```

| Measurement | Result |
|---|---:|
| Completed synthetic calls | 1,000 |
| Events per call | 6 |
| Elapsed workload time | 40.23 s |
| Calls per second | 24.86 |
| Events per second | 149.14 |
| p50 event latency, including ack | 23.23 ms |
| p99 event latency, including ack | 78.76 ms |

Each event uses one durable state/outbox transaction and a second durable acknowledgement transaction. Setup and compilation are excluded; thread startup and joining are included. The workload is closed-loop: four callers each wait for completion before sending another event. It is not an overload or maximum-throughput test. No protocol sockets, remote OCS, CAP encoding, replication or actual calls are involved.

This baseline does **not** satisfy a high-load gateway target. No target was supplied, and no production throughput or availability claim is made. Storage flush cost is a likely bottleneck, but that needs profiling rather than assumption. Next experiments should compare storage devices and shard counts, introduce safe group commit with a latency budget, and use an open-loop workload reporting offered load, rejection rate, backlog and end-to-end latency. Never disable durability just to improve a published number.

The forced-process-exit test demonstrates recovery of an acknowledged local commit after killing the process. It does not simulate power loss, storage controller failure, network partition or replicated failover.
