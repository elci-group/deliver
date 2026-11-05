# DDOSILE

DDOSILE is a proposed permanent facade architecture for containing hostile
workloads. Version 0.0.1 currently provides a deterministic policy compiler,
request-state simulator, in-process bounded reservation ledger, and a signed
admission-grant model. It has no network listener, deployed broker, classifier,
or decoy worker.

The initial library encodes three fail-closed rules:

- Public origin access is rejected by policy validation.
- Resource shares may not exceed total deployment capacity.
- Consequential routes may not select synthetic execution.

`RequestFlow` also requires classification and reservation before modeled
execution, and releases its modeled reservation when execution completes or is
rejected. The policy compiler accepts a deliberately narrow TOML subset and
rejects unknown fields and invalid route values. Routes are compiled with
longest-prefix precedence and method matching.

The resource ledger uses separate production and synthetic budgets, reserves
operational headroom, caps per-pool active reservation counts, rejects zero-cost
reservations, and releases capacity through RAII tokens. The broker verifier
checks Ed25519 signatures, request and policy epoch binding, expiry, backend
health, an active production reservation, and single use within a bounded
replay cache. Grants use a fixed-width binary encoding for service transport.
Production execution in the request model requires the resulting permit.

These checks are in-process models. A production deployment still needs the
issuer key confined to the trusted broker/control plane, authenticated
transport, a shared atomic replay store across broker replicas, OS-enforced
resource quotas, and private network connectivity. The included ledger and
replay cache do not enforce limits across processes or hosts. Keep one verifier
instance (or clones sharing its replay state) per broker process; separate
processes need a shared replay service.

Check the example policy with `cargo run -- check policies/example.toml`.
Run the unit suite with `cargo test`.
