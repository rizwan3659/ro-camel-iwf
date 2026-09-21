# IMSCAP simulator profile

## Agreed direction

TAS is the Diameter Ro client. IMSCAP is the Ro server and acts as the CAMEL service switching side toward a CAP v2 charging platform (SCP). CAP v2 is the target protocol; the current simulator does not encode CAP bytes.

## Implemented simulation

The TAS simulator encodes actual Diameter CCR frames. IMSCAP decodes those frames into the `imscap` state machine. A normalized SCP simulator supplies charging decisions, which IMSCAP turns into encoded CCAs. The TAS decodes and validates the answers.

| TAS input | Toward SCP | Decision toward TAS |
|---|---|---|
| CCR-I, number 0 | InitialDP intent | CCA-I only after both ApplyCharging and Continue |
| CCR-U, next number | ApplyChargingReport intent, call active | CCA-U after next ApplyCharging |
| CCR-T, next number | Final ApplyChargingReport and disconnect-event intents | CCA-T after simulated normal dialogue close |
| Pending initial/update plus ReleaseCall | Close local session | CCA result 4012, no granted units |

Dialogue close in this simulator is a completion policy, not proof of a remote financial debit. A TCAP abort must never be mapped to successful normal close.

The simulator uses one MSISDN Subscription-Id and top-level Requested/Used/Granted-Service-Unit containing CC-Time. It requires Auth-Application-Id 4, Session-Id, origin identities, Destination-Realm, Service-Context-Id, CC-Request-Type and CC-Request-Number. Termination includes Termination-Cause. Unsupported AVPs, vendor extensions, MSCC, money/octet units and duplicate singleton AVPs are rejected. This narrow profile is not a complete TS 32.299 IMS Ro implementation.

Only whole-grant updates are supported. Incremental used seconds must not exceed the current quota. Partial updates, tariff changes and grants larger than the requested quota require explicit mapping rules. IMSCAP never silently truncates the SCP grant. Call-answer events, requested BCSM events, originating/terminating legs, Connect, unsolicited release, final-unit policies and timeouts remain unimplemented.

## CAP v2 adapter work

Implement the CAP v2 application context and ASN.1 BER operation arguments using the applicable GSM 09.78 specification. InitialDP needs configured serviceKey and call attributes (calling/called party, event type, location as applicable); a subscriber alone is insufficient. Implement RequestReportBCSMEvent, EventReportBCSM, ApplyCharging, ApplyChargingReport, Continue and ReleaseCall, with TCAP dialogue/invoke correlation and error handling. Convert time units explicitly; normalized whole seconds here are not CAP wire units. Add independent known-byte vectors rather than testing only codec round trips.

Below CAP, implement TCAP, SCCP and M3UA/SCTP or integrate a documented external signalling transport. This project does not yet have either transport. The target is a generic configurable CAP v2 profile; interoperability with arbitrary charging platforms cannot be inferred from the in-process simulator.

## Durability and scale

`imscap::transition` is a pure, serializable state machine. Its caller must atomically commit state, deduplication receipts and outgoing actions before dispatch. Key Ro receipts by authenticated peer, Session-Id and request number, reject changed payloads and replay answers using the retransmitted request's Hop-by-Hop identifier. CAP invoke IDs need separate dialogue-scoped receipts. These integrations are not present yet.

The older `store` and `runtime` modules serve the opposite-direction prototype only. Their crash tests and benchmark results are not evidence of IMSCAP durability or throughput. IMSCAP still needs bounded queues, durable outbox dispatch, timers, retention, distributed replication/fencing and load/failure testing.

Reference: [Diameter credit control, RFC 8506](https://www.rfc-editor.org/rfc/rfc8506.html).
