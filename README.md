# IMSCAP — Diameter Ro to CAMEL

Experimental Rust implementation of the interworking function between a TAS and a CAP v2 prepaid charging platform.

```text
TAS  <-- Diameter Ro CCR/CCA -->  IMSCAP  <-- CAMEL CAP v2 -->  SCP
```

The current milestone is an **in-process simulator**, with binary Diameter messages and normalized CAMEL operations. CAP v2 wire encoding, SS7 transport, network Diameter peers, durable IMSCAP dispatch and clustered failover are not implemented yet.

## Run the simulation

```sh
cargo run --locked --example imscap_sim
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The simulated call requests 60 seconds, consumes that grant, requests another grant, and terminates after 12 more seconds. IMSCAP generates InitialDP, charging-report and disconnect intents and returns correlated Diameter answers. It waits for both ApplyCharging and Continue before initial authorization.

The state machine rejects out-of-sequence requests, overlapping requests, excessive usage and unsupported partial updates. Tests cover authorization ordering, grant rejection, termination, credit denial and Diameter parsing/correlation. The simulator uses synthetic subscriber data.

See [the simulator profile and remaining implementation](docs/imscap-profile.md). The chosen AVP subset is a lab profile; no vendor interoperability or full Ro/CAP conformance is claimed.

## Performance and availability

High load and high availability are requirements, not measured properties of this milestone. IMSCAP needs persistent replay handling, bounded dispatch, protocol timers, replicated ownership and failure testing before deployment.

The repository retains an earlier CAMEL-to-Ro experiment in `charging`, `store` and `runtime`. Its `lab`, `outbox` and `bench` commands exercise that earlier direction. [Its documentation](docs/legacy-camel-to-ro.md) and [benchmark](docs/performance.md) are historical prototype results and do not measure IMSCAP.
