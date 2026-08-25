# zyris-hello

The smallest complete Zyris node, and the reference to copy when writing your own. It announces one
capability (`hello`) with one tool (`greet`, which returns a random greeting) and consumes the
`attacca_api` capability Attacca announces back — both over the same websocket.

It depends on no `attacca-*` crate. That is deliberate: a node needs only the Zyris stack. The
`attacca_api` capability it consumes comes from `zyris-attacca`, which is part of that stack and
carries the declaration alone — no server, no database, no deployment.

## Running it

Against the hosted deployment there is nothing to configure at all — no URL, no token, no name:

```bash
cargo run -p zyris-hello
```

Against a local server, point it somewhere else:

```bash
# 1. Attacca, in one terminal
cargo run -p attacca-server

# 2. This node, in another terminal. Still no token needed.
export ZYRIS_SERVER_URL=ws://127.0.0.1:8080/zyris/v1/ws
cargo run -p zyris-hello
```

With no credential saved, the node names itself after this machine and enrolls itself. It prints a
short code and waits:

```
--------------------------------------------------------------
  Authorize this node

  1. Open        http://127.0.0.1:5173/settings/zyris/device
  2. Enter code  WXQR-7KBD

  Waiting for approval. Press Ctrl-C to cancel.
--------------------------------------------------------------
```

Type that code into Attacca on whatever device has a browser — your laptop, your phone, the machine
you SSH'd from. You will see what this node says it is, where Attacca saw the request come from, and
which scopes it asked for; you choose which to grant. Press Authorize and the node connects:

```
INFO zyris_hello: authorized account=you@example.com
INFO zyris_hello: registered this node node_id=... slug=build-box
INFO zyris_hello: connected node_id=...
INFO zyris_hello: server announced attacca_api; this node can call back into Attacca
INFO zyris_hello: attacca_api.list_agents ok count=3 first=Researcher
```

The account credential is written to `~/.config/zyris-hello/credential.json` (mode `0600`) by
**this crate**, in about twenty lines at the bottom of `src/main.rs`. The library never chooses a
path: `enroll` hands the credential back as a value and `Account::restore` takes one back, so
replacing `read_credential`/`write_credential` with a keychain, a k8s Secret or a database row is
the whole change. Subsequent runs load it and connect without printing anything.

`ATTACCA_PUBLIC_URL` must be set on the server for the verification URL to be printable; without
it the device endpoints report themselves unavailable rather than printing an address that goes
nowhere.

The node's card in the dashboard flips to connected and lists `hello v1 > greet`. Ask an agent to
call `zyris__{slug}__hello__greet` with `{"name": "Ada"}` — the slug is on the node card, derived
from the name you approved — and it will answer from this process. `Ctrl-C` closes the connection
gracefully and the card flips back to offline.

## Configuration

| Variable | Default | Notes |
|---|---|---|
| `ZYRIS_SERVER_URL` | `zyris::DEFAULT_SERVER_URL` (`wss://attacca.cc/api/zyris/v1/ws`) | Point at a local server with `ws://127.0.0.1:8080/zyris/v1/ws`. The enrollment endpoints are derived from this by truncating at `/zyris/`, so a path prefix like `/api` is carried along and a node cannot enroll against one deployment while connecting to another. |
| `ZYRIS_NODE_NAME` | this machine's hostname | The name proposed at enrollment, and the name this node registers itself under. If an account already has a node by that name the server appends `-2`, so two machines called `build-box` stay individually addressable instead of one silently shadowing the other. Renaming does **not** rename a node that already exists — it registers another one. |
| `ZYRIS_HELLO_CREDENTIAL` | `$XDG_CONFIG_HOME/zyris-hello/credential.json` | Where **this crate** writes the account credential. There is no library default to inherit: the path is chosen in `credential_path()` and is yours to change. Point it somewhere writable under `systemd` with `ProtectHome=yes`. |
| `RUST_LOG` | `zyris_hello=info,zyris=info` | Standard `tracing` filter. The authorization block is printed by `authorize()` in this crate, not by the library and not through `tracing`, so it survives `RUST_LOG=error` — and a node with a screen draws it instead of printing it. |

## What to copy

There are only two files, and one of them is a capability.

- `src/greeter.rs` — **the part that is actually yours.** `#[zyris::capability(name = .., version =
  ..)]` on a trait generates the descriptor, the `HelloServer<T>` you hand to the node builder, and a
  `HelloClient` for consumers. Doc comments become the tool and field descriptions the model reads,
  so write them for the model.
- `src/main.rs` — the wiring, and every decision the library stopped making for you:

  ```rust
  let account = Account::restore(&server, credential)
      .on_rotate(|rotated| async move { write_credential(&rotated).map_err(..) })
      .build();

  let token = account.register_node(NodeSpec { name, platform, scopes }).await?;

  let link = Node::builder()
      .kind(NodeKind::Service)
      .capability(HelloServer(greeter))
      .on_connect(|conn| async move { report_server_capabilities(&conn).await })
      .build()?
      .connect(&server, &token)
      .await?;

  link.wait_closed().await?;
  ```

  Four decisions are visible there and all four are yours: where the credential is read from, what
  the rotation callback does with a new one, what scopes the node token carries, and when the
  process ends. `connect` owns the dial loop and the backoff and nothing else — no signal handler,
  no exit code, no config directory. The account credential is the only thing that rotates; the
  `znt_` a node dials with is static, which is why one credential can mint several of them and run
  several nodes at once.

  `report_server_capabilities` in the same file is the **consume** half. A node is not only a tool
  provider: the server announces `attacca_api` on the same websocket, so this process can drive
  agents and sessions while serving `greet`, and `on_connect` is where it picks that client up. The
  client comes from [`zyris-attacca`](../zyris-attacca), which declares that capability with the
  same `#[zyris::capability]` macro the provider side uses — a consumer's half of a capability is an
  ordinary trait, and this one is already written.

  Consuming a capability nobody has published a crate for works the same way, minus the import:
  declare a trait naming the methods you call. Matching is by `(name, version)` and the announced
  tool list is never compared, so one method out of a server's nine, and two fields out of a
  struct's four, still resolve against the real announcement. Declare the slice you call; serde
  ignores the rest.

For your own supervision tree, or a connection per tenant, keep `register_node` and dial each
token separately: node identity comes from the token, so two tokens are two nodes and neither
displaces the other. A node provisioned without a person skips the account layer entirely — hand
`connect` the `znt_` string out of your secret manager, which is all it ever wanted.

## Choosing between enrollment and a static token

Enrollment is right for an interactive install: a machine you are sitting at, or one you SSH'd into,
where there is a person who can approve it. It never asks you to copy a secret between two machines.

A static `znt_` is right for anything provisioned without a human — image-baked nodes, CI, and
**shared service accounts**. Be clear-eyed about the last one: a credential file cannot be protected
from anyone who can `sudo -u` the account that owns it. If several people administer the account
running this node, put a static token in your secret manager instead of enrolling.

That path is shorter than this crate's, not longer: `Node::connect` takes any bearer string, so a
node handed a `znt_` skips `enroll`, `Account` and `register_node` altogether and never reads a
credential file. This crate does not do it, because a reference node that never prints a code
would not show the half that needs showing — read the last two arguments of `connect` and delete
everything above them.

## The dependencies

`Cargo.toml` here names the whole protocol stack once, and the reference implementations once:

```toml
zyris = { version = "0.2", features = ["attacca", "caps", "enroll"] }
zyris-capkit = { path = "../zyris-capkit" }
```

`zyris::caps` and `zyris::attacca` *are* the `zyris-caps` and `zyris-attacca` crates, reached
through the face rather than named again — one version number to keep straight instead of four.
Naming them directly is not a mistake and links nothing twice; it is just more to keep in step.

The features above the runtime are `caps`, `attacca` and `p2p`, plus `enroll`, with `full` for all
four. **`zyris-capkit` is deliberately not among them.** What a node offers is the node's decision,
so `PtyTerminal` on the second line is an example of making that choice rather than the protocol
making it — and the crate is unpublished, so a node that wants it names it out of this repository,
as this one does. Copying this crate means copying that line and then replacing it. This node's
`desktop` and `transfer` features are one line each on top of capkit's own.

Two things that will bite you if you deviate:

- Pin `schemars` to the same version `zyris` re-exports. The macro expands to
  `::zyris::schemars::schema_for!`, so your types must implement *that* crate's `JsonSchema`.
- Keep `zyris`'s default features on. `Node::connect` and `Link` live behind `client` and
  `zyris::machine_name` behind `hostname`; both are default. `enroll` is *not*: a node handed a
  static `znt_` pays for neither the device grant nor the account layer, so `zyris-hello` opts
  into it explicitly.

## Tests

`cargo test -p zyris-hello` exercises the whole announce path over an in-memory duplex — no server
and no database needed. See `tests/greet_roundtrip.rs` for the pattern.
