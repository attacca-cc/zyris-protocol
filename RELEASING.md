# Releasing

Eight crates go to crates.io from this workspace, all of them.

## What publishes

| crate | why it is separate |
|---|---|
| `zyris-proto` | wire types; no dependency on anything here |
| `zyris-macros` | the `#[zyris::capability]` proc-macro |
| `zyris-core` | the node runtime and client |
| `zyris-caps` | the standard capability declarations — no OS, no runtime |
| `zyris-capkit` | the reference implementations, one feature each |
| `zyris-p2p` | node-to-node QUIC transport |
| `zyris-attacca` | the `attacca_api` declaration |
| `zyris` | the face — `cargo add zyris` |

`zyris-fs`, `zyris-terminal`, `zyris-transfer` and `zyris-screen` were published at 0.2.0 before
the implementations were merged into `zyris-capkit`. They are no longer in this workspace and will
not get another release.

**No git dependencies, and `cargo package` will not tell you if one appears.** crates.io does not
refuse a manifest naming a git source when the dependency also carries a `version` — cargo packages
it and **silently drops the `git` and `rev`**, resolving the registry crate of that name instead.
That is how `zyris-capkit`'s `input` would have shipped upstream `enigo` without the two patches it
needs, which is why the fork is published as
[`enigo-zyris`](https://github.com/ridanit-ruma/enigo-zyris) and depended on from the registry. So
**never pass `--no-verify` to a real publish**.

`every_crate_that_says_it_publishes_can` in `crates/zyris/tests/crate_boundary.rs` walks every
member's graph with all features on and fails if any of them reaches a git checkout.

## Rehearse

```bash
cargo package --workspace
```

Every crate is packaged against the ones beside it, which is the only way to rehearse before any of
them exists on the registry. **Per-crate `cargo package -p zyris-core` cannot work yet** — it
resolves siblings from crates.io and fails with `no matching package named zyris-macros found`.
That stops being true after the first release.

Drop `--no-verify` on the real rehearsal: verification builds each packaged crate from its tarball,
which is what catches a file the manifest forgot to include.

## Publish

```bash
cargo publish --workspace
```

Cargo works out the order and waits for the index between crates. **Prefer this over publishing one
crate at a time**, for the reason the next paragraph is about.

If you do run them one at a time, this is the order, and it is not the one the layering suggests:

```text
zyris-proto  zyris-macros  zyris-core  zyris-caps  zyris-p2p  zyris-attacca  zyris  zyris-capkit
```

**A crate with optional intra-workspace dependencies goes after them anyway.** Cargo resolves an
optional dependency against the registry whether or not a feature turned it on. An earlier version
of this file put the transfer crate seventh and `zyris-p2p` ninth; that run died at crate seven with
six names already permanent.
Measured with a throwaway crate carrying one optional dependency on a name that does not exist:

```text
error: failed to prepare local package for uploading
Caused by:
  no matching package named `…` found
```

So the rule is: anything naming an intra-workspace crate goes after it, `optional = true` included.

## Only the protocol crates?

**Not supportable as written, and the reason is in the manifests.** Every intra-workspace dependency
here is the `version` + `path` form — `zyris-attacca/Cargo.toml` says
`zyris = { version = "0.3.0", package = "zyris-core", path = "../zyris-core" }`. Cargo strips `path`
when the crate comes from the registry and keeps it when the crate comes from git. So a consumer who
takes `zyris` from crates.io and `zyris-capkit` from git gets `zyris-core` and `zyris-caps` twice,
from two different sources, and the error names the same type on both sides of a trait bound it
plainly satisfies.

A release of `zyris` that cannot be combined with any implementation is not the smaller half of this
release. It is a different, worse one.

## Who owns them

`cargo owner --add ridanit-ruma` — the personal account, alone, and it is worth saying why rather
than leaving it to look like an oversight.

**A crate name is never given back.** crates.io has one exception, RFC 3660: a crate can be deleted
if it is under 72 hours old, *or* if it has a **single owner**, fewer than 500 downloads a month,
and nothing on the registry depending on it. Adding a `github:attacca-cc:<team>` owner removes the
single-owner branch outright and leaves only the 72-hour window. So the order is not symmetric:
personal first and a team later keeps the escape hatch; a team first spends it before the first
mistake has had a chance to show.

That is the whole argument, and it costs something real — the repository lives under `attacca-cc`
while the crates would answer to one person, and that person is the bus factor for every permanent
name. Add the team once the release has settled.

**`github:ridanit-ruma:<team>` is not a thing.** crates.io team ownership takes a GitHub
*organization*; `ridanit-ruma` is a user account and has no teams. The bare login is the form.

## What a consumer gets

`cargo add zyris` builds on a machine with a Rust toolchain and nothing else — no C compiler, no
`cmake`, no system packages — and **cannot reach a `wss://` server**: rustls picks its cryptography
from crate features, none is on by default, and the dial is refused by `ConnectError::NoTlsProvider`
rather than left to panic. Reaching the network is `features = ["tls-ring"]`, and that is where the
C compiler is charged. `crates/zyris/tests/toolchain_cost.rs` measures all of it, including that
`enroll` implies `tls-ring` and therefore that `full` is not the cheap graph.

Say this in the release notes. It is the first thing a new consumer hits and the only part of the
build cost that is not obvious from the feature names.

The other thing worth a line there is the **display coordinate contract**, because it is the one
promise in this release that a consumer builds on top of and cannot check from the outside: every
number in a `Display`, every `Region` and both coordinates of `input.move_to` are in captured
pixels — the pixels a screenshot is made of — and where a platform's advertised geometry disagrees
with its picture, the picture wins. `zyris-caps`'s `Display` states it, and
`crates/zyris-capkit/docs/screen.md` records what is still known wrong (a second monitor on GNOME,
rotation, and the `input-libei` backend).

## Version

One `version` in `[workspace.package]`, so every crate moves together. Bump it there, and remember
the `version = "0.3.0"` on each intra-workspace dependency line — those are what a published crate
resolves against, and they do not follow the workspace field.
