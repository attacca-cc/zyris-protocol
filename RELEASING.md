# Releasing

Eleven crates go to crates.io from this workspace and two do not. Nothing is published yet, so the
first release is also the first time this procedure runs end to end.

## What publishes

| crate | why it is separate |
|---|---|
| `zyris-proto` | wire types; no dependency on anything here |
| `zyris-macros` | the `#[zyris::capability]` proc-macro |
| `zyris-core` | the node runtime and client |
| `zyris-caps` | the standard capability declarations — no OS, no runtime |
| `zyris-fs` `zyris-terminal` `zyris-transfer` `zyris-screen` | one reference implementation each |
| `zyris-p2p` | node-to-node QUIC transport |
| `zyris-attacca` | the `attacca_api` declaration |
| `zyris` | the face — `cargo add zyris` |

**`zyris-input` and `zyris-capkit` carry `publish = false`.** `zyris-input` pins
[a fork of `enigo`](https://github.com/ridanit-ruma/enigo-zyris) carrying two patches upstream does
not have, and crates.io refuses any manifest whose graph names a git source; the way out is
upstreaming those patches, not working around the rule. `zyris-capkit` is the re-export shell the
old single-crate consumers move through, and it inherits the same problem by naming `zyris-input`.

`every_crate_that_says_it_publishes_can` in `crates/zyris/tests/crate_boundary.rs` holds both halves
of that: no crate without the opt-out reaches a git checkout, and `zyris-input` still reaches one —
so if the fork ever lands upstream, that test fails and tells you to drop the opt-out.

## Rehearse

```bash
cargo package --workspace --exclude zyris-input --exclude zyris-capkit
```

Every crate is packaged against the ones beside it, which is the only way to rehearse before any of
them exists on the registry. **Per-crate `cargo package -p zyris-core` cannot work yet** — it
resolves siblings from crates.io and fails with `no matching package named zyris-macros found`.
That stops being true after the first release.

Drop `--no-verify` on the real rehearsal: verification builds each packaged crate from its tarball,
which is what catches a file the manifest forgot to include.

## Publish

Order matters — each crate has to be on the registry before anything that names it.

```bash
for c in zyris-proto zyris-macros zyris-core zyris-caps \
         zyris-fs zyris-terminal zyris-transfer zyris-screen \
         zyris-p2p zyris-attacca zyris; do
  cargo publish -p "$c"
done
```

The registry index takes a moment to catch up between crates; if the next one fails to resolve the
one before it, wait and retry rather than reaching for `--no-verify`.

**Publishing only the protocol crates is a supported variation.** Stopping after `zyris-caps` and
`zyris-p2p`/`zyris-attacca`/`zyris` leaves the four reference implementations git-only, which is the
shape a release that wants to say "this is a protocol, not a toolkit" would take. Going the other
way later is one publish; a name once taken cannot be given back, so the reversible order is
protocol first.

## What a consumer gets

`cargo add zyris` builds on a machine with a Rust toolchain and nothing else — no C compiler, no
`cmake`, no system packages — and **cannot reach a `wss://` server**: rustls picks its cryptography
from crate features, none is on by default, and the dial is refused by `ConnectError::NoTlsProvider`
rather than left to panic. Reaching the network is `features = ["tls-ring"]`, and that is where the
C compiler is charged. `crates/zyris/tests/toolchain_cost.rs` measures all of it, including that
`enroll` implies `tls-ring` and therefore that `full` is not the cheap graph.

Say this in the release notes. It is the first thing a new consumer hits and the only part of the
build cost that is not obvious from the feature names.

## Version

One `version` in `[workspace.package]`, so every crate moves together. Bump it there, and remember
the `version = "0.2.0"` on each intra-workspace dependency line — those are what a published crate
resolves against, and they do not follow the workspace field.
