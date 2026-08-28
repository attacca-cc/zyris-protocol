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

```bash
cargo publish --workspace --exclude zyris-input --exclude zyris-capkit
```

Cargo works out the order and waits for the index between crates. **Prefer this over publishing one
crate at a time**, for the reason the next paragraph is about.

If you do run them one at a time, this is the order, and it is not the one the layering suggests:

```text
zyris-proto  zyris-macros  zyris-core  zyris-caps  zyris-p2p  zyris-attacca  zyris
zyris-fs  zyris-terminal  zyris-screen  zyris-transfer
```

**`zyris-transfer` goes last because of dependencies a default build does not have.** It names
`zyris-attacca` and `zyris-p2p` as *optional*, and cargo resolves an optional dependency against the
registry whether or not a feature turned it on. An earlier version of this file put `zyris-transfer`
seventh and `zyris-p2p` ninth; that run dies at crate seven with six names already permanent.
Measured with a throwaway crate carrying one optional dependency on a name that does not exist:

```text
error: failed to prepare local package for uploading
Caused by:
  no matching package named `…` found
```

So the rule is: anything naming an intra-workspace crate goes after it, `optional = true` included.

## Only the protocol crates?

**Not supportable as written, and the reason is in the manifests.** Every intra-workspace dependency
here is the `version` + `path` form — `zyris-fs/Cargo.toml` says
`zyris = { version = "0.2.0", package = "zyris-core", path = "../zyris-core" }`. Cargo strips `path`
when the crate comes from the registry and keeps it when the crate comes from git. So a consumer who
takes `zyris` from crates.io and `zyris-terminal` from git gets `zyris-core` and `zyris-caps` twice,
from two different sources, and the error names the same type on both sides of a trait bound it
plainly satisfies.

A release of `zyris` that cannot be combined with any implementation is not the smaller half of this
release. It is a different, worse one.

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
