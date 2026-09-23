# Zyris Protocol v1

Zyris is an expandable computer protocol. A **Zyris Node** is any machine that dials into a
server over a websocket, announces typed **capabilities** (named, versioned sets of tools),
and may simultaneously consume capabilities the peer announces.

The protocol is direction-symmetric: after the handshake, either peer may send requests,
open streams, and announce capabilities. "Node" and "server" below refer to the dialing and
accepting peers respectively; every rule applies to both directions unless stated.

[Attacca](https://attacca.cc) is the reference deployment and the origin of the protocol. It is
itself a Zyris Node: it is the sole announcer of the `attacca_api` capability, which lets nodes
drive agents, sessions, and live turn streams over the same connection. This document is the wire
reference; where it describes something a deployment chooses rather than something the wire
requires — credential formats, enrollment, cross-pod routing — it says so and uses Attacca as the
worked example.

## 0. Writing a node

`crates/zyris/examples/hello.rs` is a complete, runnable node in one file — one capability with
one tool, plus enrollment and the connection. Start there; this document is the
normative wire reference, not a tutorial. Run it with

```bash
cargo run -p zyris --example hello --features enroll
```

and point it at a local server with `ZYRIS_SERVER_URL`.

## 1. Transport and framing

One websocket per connection. Endpoint: `wss://<host>/zyris/v1/ws`. TLS is mandatory; the
URL path pins the protocol **major** version (a breaking envelope change becomes
`/zyris/v2/ws`).

Two frame classes ride the socket. Every websocket **binary** frame begins with a one-byte
kind tag:

| Tag    | Name        | Layout after the tag |
|--------|-------------|----------------------|
| `0x00` | CONTROL     | msgpack-encoded `Envelope` |
| `0x01` | STREAM_DATA | `stream_id: u32 BE`, `chunk_seq: u32 BE`, raw payload bytes |
| `0x02`–`0x0F` | reserved | receivers MUST close with `4408` on unknown tags |

Serialization modes:

- **msgpack** (default): CONTROL frames are binary tag `0x00`. Blob bytes never pass
  through msgpack.
- **json** (negotiated at handshake): CONTROL envelopes ride as websocket **text** frames
  (no tag byte — the websocket opcode discriminates); STREAM_DATA stays binary `0x01`.
  JSON mode exists for debuggability and browser-adjacent clients.

`chunk_seq` starts at 0 per stream and increments by 1 per chunk. On a direct hop TCP makes
it trivially contiguous; it exists so a relayed hop (§9) can detect loss: any gap MUST fail
the stream with `stream_lagged` — never deliver bytes past a gap.

Limits are server-declared in `hello_ack` (defaults): max CONTROL frame 8 MiB (headroom above
the 4 MiB inline-blob ceiling), max STREAM_DATA payload 256 KiB, `max_inflight_reqs` 64,
`initial_stream_credit` 256 KiB.
Violations are protocol errors: `payload_too_large` on the offending request, or connection
close `4409` for credit violations.

## 2. Envelope

A single tagged union, tag field `t`. Shown as JSON; msgpack on the wire by default.

```jsonc
{ "t": "req",  "id": 17, "method": "terminal.exec", "params": { }, "stream": { "id": 42 } }
{ "t": "res",  "id": 17, "result": { } }
{ "t": "err",  "id": 17, "error": { "code": "invalid_params", "message": "…", "retriable": false, "data": null } }
{ "t": "note", "method": "webrtc.signal", "params": { } }
{ "t": "prog", "id": 17, "payload": { } }
{ "t": "cancel", "id": 17 }
{ "t": "s_credit", "stream": 42, "bytes": 262144 }
{ "t": "s_end",    "stream": 42, "trailer": { "sha256": "…" } }
{ "t": "s_err",    "stream": 42, "error": { "code": "io_error", "message": "…", "retriable": true } }
{ "t": "s_cancel", "stream": 42 }
```

- `method` is `{capability}.{tool}` for capability tools, or a `zyris.*` / `webrtc.*`
  protocol method (§5, §8).
- `id` is a monotonically increasing `u64`, allocated by the sender; the two directions are
  independent id spaces. A `res`/`err`/`prog`/`cancel` always references the receiver's view
  of the originating `req`.
- `req.stream` is present iff the tool's declared transfer mechanism involves a stream; the
  request **sender** allocates the stream id (§4).
- `prog` frames are optional, per-tool-schema progress updates; zero or more precede the
  final `res`/`err`.
- `cancel` is best-effort and idempotent. The canceled request MUST still terminate with
  exactly one `res` or `err` (normally `err canceled`). `cancel` for an unknown or finished
  id is silently ignored.
- Exceeding `max_inflight_reqs` yields `err overloaded` (retriable).

### 2.1 Errors

`code` is a string; `retriable` is explicit per instance (a callee may mark a normally
retriable failure permanent). Registered protocol codes:

```
parse_error            unsupported_version    unauthorized         forbidden_scope
method_not_found       invalid_params         capability_not_announced
capability_rejected    capability_unavailable node_offline*        connection_lost*
stream_lagged*         credit_violation       payload_too_large    overloaded*
canceled               timeout*               internal
```

`*` = conventionally retriable. Capabilities define additional codes; they MUST NOT collide
with the registry above.

## 3. Connection lifecycle

```
CLOSED ──dial──▶ AUTHENTICATING ──upgrade ok──▶ HELLO ──hello/hello_ack──▶ READY
   ▲                   │ HTTP 401                  │ unsupported_version → CLOSED (4400)
   │                   ▼                           │
   └────────────── CLOSED ◀── CLOSING ◀── zyris.closing / ws close / heartbeat timeout
        reconnect w/ resume_token within grace ⇒ presence continuity (§3.4)
```

### 3.1 Auth

The dialer sends `Authorization: Bearer <credential>` on the upgrade request. Auth failure refuses
the upgrade with HTTP `401` before either side has spoken the protocol — there is no websocket to
close yet. What a valid credential *looks like* is the deployment's choice; the wire only requires
that it ride in that header.

The socket authenticates once at the upgrade and nothing re-checks it, so a deployment that
revokes credentials mid-connection needs an in-band way to act on it, since a credential itself
never expires. The recommended shape, and Attacca's: the periodic heartbeat also reports whether
the node was revoked, and the peer holding the socket closes it with websocket code `4401` when it
is. Without that, revocation takes effect only on whichever replica happens to see the revoke call.

Attacca's scheme, as the worked example: one credential kind, a long-lived `zc_` credential issued
to one (system, program) pair (§7), hashed at rest and carrying the scopes chosen when it was
issued. It never expires; it is revoked from the web UI. Anything else — the retired `zna_` access,
`znr_` refresh and `znt_` node tokens included — is simply unknown: the upgrade is refused with
HTTP 401, exactly like a revoked or mistyped `zc_`, and `zyris` reports `ConnectError::Unauthorized`.
A client reads that as "discard the stored credential and enroll again"; there is no separate
re-enroll signal.

### 3.2 Handshake

The first frame in each direction is the handshake, **always msgpack** regardless of the
outcome of negotiation.

```jsonc
// dialer → acceptor
{ "t": "hello",
  "protocol": { "major": 1, "minors_supported": [0] },
  "serialization": ["msgpack", "json"],
  "agent": "zyris/0.2.0 (myrepo; service)",
  "kind": "service",                                          // optional
  "node_name": "myrepo",                                      // optional; see below
  "features": ["cancel", "attachments", "video-webrtc", "video-mjpeg"],
  "resume": { "conn_id": "…", "resume_token": "…" } }        // optional

// acceptor → dialer
{ "t": "hello_ack",
  "protocol": { "major": 1, "minor": 0 },
  "serialization": "msgpack",
  "conn_id": "…", "resume_token": "…",
  "node_id": "…",
  "node": { "system": "laptop", "program": "zyris-code", "name": "myrepo-2" },   // optional
  "heartbeat": { "interval_s": 20, "timeout_s": 45 },
  "limits": { "max_control_frame": 1048576, "max_chunk": 262144,
              "max_inflight_reqs": 64, "initial_stream_credit": 262144 },
  "resumed": false,
  "features": ["cancel", "attachments"] }
```

Version mismatch ⇒ `err unsupported_version` then websocket close `4400`. A peer MUST NOT
send frames gated behind a feature the other side did not advertise. `features` is absent
on an acceptor that predates the field, which reads as advertising nothing — the safe
answer, since a feature is usable only when it appears in *both* lists.

`node_name` is the name the connection asks to be known by. It is optional on the wire, so a peer
that predates it still parses, but a deployment that names nodes may require it: Attacca answers a
non-`cli` hello without one with `err invalid_params` and closes. `kind: "cli"` is a consumer that
registers no node, and its `node_name` is ignored.

`node` is where the acceptor put the connection: three slugs, written as the path
`system/program/name` (`zyris::NodeAddress::path`). It is absent for a `cli` dialer and from an
acceptor that predates the field. The name is the requested one when it is free among the *live*
nodes at that `system/program` path — counting every credential that shares the program name, not
only this one — and the lowest free `-2`, `-3`… otherwise, so read it back rather than assuming it.
zyris-core never sends `Hello.resume`: instead, a `node_name` that matches a row of the *same*
credential released within the grace, or gone stale, takes that row back by name, keeping its
`node_id` and `node` — so a crash does not turn `desktop` into `desktop-2`. Any other connect is a
new node with a new `node_id`.

`accept_with`'s `decide` callback has no built-in timeout: an acceptor that does slow work there
(a database lookup to pick the node's name, say) has to bound it itself, or a slow decision hangs
the handshake indefinitely.

### 3.3 Heartbeat and close

Websocket ping/pong is the liveness primitive: the acceptor pings every `interval_s`;
either side closes after `timeout_s` of silence. Graceful close: send
`note zyris.closing { reason }`, stop issuing new requests, finish or cancel in-flight
work, then websocket close `1000`. A server shutting down for a rolling restart sends
`reason: "server_restart"` so nodes reconnect immediately instead of backing off.

### 3.4 Reconnect is deliberately lossy

On disconnect, every in-flight request on both sides fails locally with `connection_lost`
(retriable) and every open stream is dead. Reconnecting with a valid `resume_token` within
the grace window (30 s) restores **identity continuity only**: the node is never marked
offline, and its capability announcement is assumed unchanged (`resumed: true`); otherwise
the node MUST re-announce.

Resume of interrupted work is an application concern by design: turn streams resume by
cursor, file transfers by byte offset, PTYs by re-open and redraw. Streaming tools MUST
document their resume story.

## 4. Streams

A stream is born from a request: the request sender allocates the id and declares it in
`req.stream.id`. Ids are `u32`, never reused within a connection; the dialer allocates odd
ids, the acceptor even ids.

Transfer mechanisms, declared per tool in the capability descriptor:

| `transfer`   | Behavior |
|--------------|----------|
| `unary`      | `req` → (`prog`)* → `res`/`err`. Datums inline or via a declared stream. |
| `uni_stream` | Caller declares the stream; the `res` arrives **immediately** carrying the typed stream head (metadata such as file stat or initial status); the **callee** then sends item chunks caller-ward until `s_end` (optional summary in `trailer`) or `s_err`. Failure to start yields a plain `err` and no chunks. |
| `bi_stream`  | Reserved in v1. Frames are defined (one id, direction implicit in sender) but implementations MUST reject tools declaring it. |
| `video`      | No websocket data frames; the request establishes a video session and WebRTC signaling follows (§8). |

Chunk payload semantics are defined per tool. Typed uni-streams encode exactly one item
per chunk in the connection's negotiated serialization (msgpack by default) — this is how
`attacca_api.turn_events` frames and `file_io.read_stream` byte chunks (a `bin`-encoded item)
travel. MJPEG fallback chunks are `[u64 BE timestamp_µs][one complete JPEG]`.

### 4.1 Flow control

Credit-based, mandatory, per stream. The receiver implicitly grants
`initial_stream_credit` bytes at stream birth and tops up with additive
`s_credit { stream, bytes }`. The byte-sender MUST NOT exceed outstanding credit; a
violation is a protocol bug: `s_err credit_violation` + connection close `4409`. CONTROL
frames are never subject to credit.

Rationale: one websocket multiplexes many streams — without credits a slow 2 GiB file sink
head-of-line-blocks PTY keystrokes and heartbeats; and the relayed hop (§9) has no
end-to-end TCP backpressure, so credits are the only bound on relay buffering.

### 4.2 Stream termination

- Clean end: byte-sender emits `s_end { stream, trailer? }` (trailer may carry `sha256`).
- Sender failure: `s_err`; receiver abort: `s_cancel` (sender stops, discards).
- Implicit close: the final `res`/`err` of the owning request closes any stream it
  declared (well-behaved senders still send `s_end` first; the implicit rule covers
  crashes).

```
DECLARED ──first chunk──▶ FLOWING ⇄ BLOCKED(credit=0)
    │                        │ s_err / s_cancel
    │ s_cancel               ▼
    ▼                      ENDING ──s_end──▶ CLOSED
  CLOSED ◀── owning req res/err or connection loss (forced)
```

## 5. Capabilities

A capability is a named, integer-versioned set of tools. Each tool declares a transfer
mechanism and schemars-generated JSON Schemas (request, response, optional stream item).

Announcement is a normal request, available to **either peer** after `hello_ack`:

```jsonc
{ "t": "req", "id": 1, "method": "zyris.announce", "params": {
  "capabilities": [
    { "name": "terminal", "version": 1,
      "tools": [
        { "name": "exec", "description": "Run a command to completion.",
          "transfer": "unary",
          "request_schema": { }, "response_schema": { } },
        { "name": "open", "description": "Open an interactive PTY.",
          "transfer": "uni_stream",
          "request_schema": { }, "response_schema": { }, "item_schema": { } } ] } ] } }

{ "t": "res", "id": 1, "result": {
  "accepted": ["terminal"],
  "rejected": [ { "name": "attacca_api", "reason": "reserved" } ] } }
```

- `zyris.announce` is **full-replacement and idempotent**: re-announcing without a
  previously announced capability revokes it; mid-session capability changes are just a new
  announce. Revocation fails in-flight calls on that capability with
  `capability_unavailable`.
- Calling a tool of a capability the peer has not announced ⇒ `capability_not_announced`.
- A deployment may **reserve** capability names to itself, rejecting them from nodes with
  `capability_rejected`, reason `reserved`. Attacca reserves exactly `attacca_api`: it rejects that
  name from any node and announces it itself immediately after `hello_ack`, filtered to the tools
  the node's scopes permit, with every `attacca_api.*` call scope-checked. That capability is
  declared in `crates/zyris-attacca` — the deployment's surface, not the wire's, and the one
  capability a node only ever calls, which is why it is its own crate and not part of `zyris-caps`.
- A node may announce two versions of the same capability simultaneously (two entries).
  Additive tool changes within a version are permitted; consumers discover tools by
  descriptor.

## 6. Datums and blobs

Three datum kinds cross the wire as values inside `params` / `result` / stream items:

```jsonc
{ "kind": "text",  "text": "…", "format": "markdown" }
{ "kind": "file",  "filename": "report.pdf", "description": "Q3 export",
  "media_type": "application/pdf", "blob": <blob> }
{ "kind": "image", "name": "screen.png", "description": "current display",
  "media_type": "image/png", "blob": <blob> }
```

A blob is either:

- **inline** — allowed only when ≤ 512 KiB: msgpack `bin` (base64 string under negotiated
  JSON — one reason msgpack is the default). The ceiling comes from the far end rather than the
  wire: a deployment that measures a result as JSON pays base64's four-for-three, so 512 KiB
  arrives as 699,052 bytes, and Attacca's `ZYRIS_MAX_RESULT_BYTES` defaults to 1,000,000, or
- **attachment** — metadata in place of bytes:
  `{ "attachment": { "stream": 42, "size": 48293812, "sha256": "…", "offset": 0 } }`,
  bytes following as STREAM_DATA chunks on the referenced stream, `s_end.trailer.sha256`
  for end-to-end integrity. Interrupted transfers resume by re-issuing the call with
  `offset`.

Attachments are **negotiated**: both peers must advertise the `attachments` feature in
`hello.features` / `hello_ack.features`. Against a peer that did not, blobs stay inline
whatever their size, per §3.2.

Where they are used, the conversion is automatic and by size. A responder detaches every
blob over 4 MiB as it writes the response: it allocates a stream from its own parity
space (§4 — the byte sender allocates, so no `req.stream.id` is involved), writes the
reference in place of the bytes, sends the `res`, and then pushes the bytes as
STREAM_DATA chunks of at most `max_chunk`, paced by the receiver's credit. `s_end` carries
the digest, which the reference also names, so a receiver can check length and content
without having trusted either end alone.

The receiver opens the stream when it decodes the payload, not when its caller gets
around to looking — chunks are already in flight by then. An announced attachment nobody
claims within two minutes is cancelled, because an unclaimed stream otherwise holds its
sender at the credit ceiling for the life of the connection.

Implementation status: inline blobs, chunked uni-streams (which cover bulk file transfer
via `file_io.read_stream`/`write_at`), and attachments in responses. Attachments in *request
params* — a large datum travelling caller-ward, as `file_io.write` would want — are the
same mechanism mirrored and are not implemented yet.

## 7. Node identity, enrollment, presence

Enrollment is out-of-band: it happens over HTTP before the websocket exists, so none of it is on the
Zyris wire. It is documented here because `zyris` implements this half of it too, and because a node
author has to choose a credential source before anything else works.

**Three things, one address.** A *system* is a machine (`laptop`). A *credential* is a long-lived
`zc_` bearer issued to one program on one system (`zyris-code` on `laptop`). A *node* is one live
connection made with a credential (`myrepo`). A node's address is `system/program/node` —
`laptop/zyris-code/myrepo` — and every segment is a slug of `[a-z0-9-]`, at most 32 characters,
with the display name kept beside it. A name already taken gets the lowest free `-2`, `-3`…: a
system within its user, and a node among the *live* nodes at that `system/program` path — counted
across every credential that shares the program name, not just this one, since program names
themselves never collide-check (two live credentials can both be `zyris-code`; nothing numbers
programs). Two instances started from the *same* credential with the same requested name are no
exception: they get `myrepo`, `myrepo-2`, `myrepo-3`…, and none displaces another.

**The library holds no credential of its own and picks no path to keep one at.** Every step hands
its result back as a value: `enroll()` returns an `Enrollment` whose `code()` is a `Code` to
display, and `Enrollment::wait` returns a `Credential`. Storing it — a file, a keychain, a
Kubernetes Secret, a database row — is the caller's, and so is whether a code becomes a printed
block, a window in a TUI, or a line in a journal. A `Credential` is serde, never expires and never
rotates, so it is written once.

- **Issuing a credential (device grant)**: the program starts unconfigured and runs RFC 8628. It
  `POST`s `/zyris/v1/device/authorize` with `{program, system_hint, platform, scopes, client_hint}`
  — `program` is its own name, `system_hint` is `zyris::machine_name()` — receives an 8-character
  base-20 code, and polls `/zyris/v1/device/token`. The user types the code into the deployment's
  web UI on whatever device has a browser, picks the system (by default the one named
  `system_hint`, or a new one; nothing on the machine is fingerprinted), reviews the program name
  and scopes, and approves. The next poll receives
  `{credential, system: {id, name, slug}, program: {id, name, slug}, scopes, owner_email}`.
  `verification_uri_complete` is always null; the `verification_uri` points at the code-entry
  screen, so the user has no button to hunt for while a code expires. `zyris::DEFAULT_SERVER_URL`
  is `wss://attacca.cc/api/zyris/v1/ws`, and the HTTP base for the device endpoints is derived from
  it by truncating at `/zyris/`, so a program cannot enroll against one deployment while
  connecting to another.
- **Issuing a credential (web)**: the user picks a system, a program name and scopes in the web UI
  and is shown the `zc_` once. This is the provisioning path with nobody at the machine —
  image-baked nodes, CI, containers — and it produces the same kind of credential.
- **Connecting**: `Node::connect(url, credential.secret())` and `Node::dial(url, bearer)` take any
  bearer string, so a program provisioned with a `zc_` out of a secret manager never touches
  enrollment at all; the `enroll` feature is off by default. `Node::builder().name("myrepo")` is
  sent as `Hello.node_name` (§3.2), and `Link::address()` returns the `NodeAddress` from the latest
  `HelloAck`. A reconnect that lands within the grace gets its old identity back by name (§3.2); one
  that does not is a new node, which gets its old name back when that is free and `-2` when it is
  not.
- **Staying connected**: `Node::connect` returns a `Link` that owns the dial-and-reconnect loop —
  backoff with jitter, reset after a healthy connection — and returns as soon as the first dial has
  settled. A refusal no retry can fix (`Revoked`, `Unauthorized`, `VersionMismatch`) is that call's
  error; anything else is the link's problem and it goes on trying. `Link::wait_closed` resolves
  when the link is finished and `Link::disconnect` ends it, giving the closing frame time to land.
  There is no signal handler and no exit code: when the process stops is the program's decision.
- **Revocation**: a credential is revoked from the web UI, directly or by deleting its system, and
  its nodes are closed. From then on the upgrade is refused with 401 and `connect` fails with
  `ConnectError::Unauthorized`: forget the credential and enroll again. That is the only case that
  reaches the client automatically — a credential the client itself drops (logout, a lost file)
  stays valid until a person revokes it by hand in the web UI, since nothing tells Attacca it was
  dropped. Approving again meanwhile just issues a second credential with the same program name;
  nothing is renumbered, and if both end up connected at once their nodes are told apart by the
  node suffix. There is no automatic replace-on-reapproval, and no separate re-enroll signal.
- **Scopes**: belong to the credential, chosen when it is issued; every node made with it has
  exactly those. The vocabulary is the deployment's; Attacca reuses its API-scope names. Creating
  nodes needs no scope — it is what a credential is for — and a node that only *serves*
  capabilities needs no scopes at all.
- **Presence**: a node exists while its connection does. On READY it is live; on disconnect a
  resume grace (30 s in Attacca) runs, after which the node is gone and its name is free again.
  `node_id` is therefore new on every connect that did not resume, and nothing should hold one
  durably. A periodic sweep removes nodes whose last heartbeat exceeds 2× the heartbeat interval,
  as a backstop against a server replica dying with the socket open.

The server-side registry — what a deployment stores per node, how ownership is fenced, how presence
fans out to a UI — is deployment-internal. See `docs/zyris-protocol.md` in the
[Attacca repo](https://github.com/attacca-cc/attacca) for how the reference deployment does it.

## 8. Video

Media must not traverse Attacca server pods. A `video` tool call establishes a session;
WebRTC signaling rides in-band as symmetric notes; media flows peer-to-peer (or via TURN).

1. `req { method: "screen.live", params: { source, max_height } }` →
   `res { video_session_id, codecs: ["h264/baseline", "vp8"], ice_servers: [ ] }`.
   `codecs` is a capability list; the final choice happens in SDP.
2. Signaling, correlated by `video_session_id`:
   `note webrtc.signal { video_session_id, signal: { type: "offer"|"answer", sdp } }` and
   trickle ICE `signal: { type: "ice", candidate, mid }`.
3. Teardown: `note webrtc.close { video_session_id }` from either side.
4. Fallback (feature `video-mjpeg`): a `uni_stream` tool streaming one timestamped JPEG per
   chunk — a degraded path, labeled as such in the announcement.

Open prerequisite: cross-NAT video requires TURN infrastructure; until Attacca operates
TURN, WebRTC works only where direct/STUN paths exist.

## 9. Relayed hops

A multi-replica server terminates a node's websocket on exactly one replica (the **owning**
replica) while calls may originate on any of them, so the call has to cross a hop the protocol
does not define. How that hop is carried is the deployment's business; what the protocol requires
of it is not:

- **Ownership must be fenced.** A replica that believes it owns a connection it has lost will
  answer for a node it cannot reach. Carry a per-connection epoch on every relayed message and drop
  traffic fenced by a newer one.
- **Ordering is per direction, per call.** Control and data interleave meaningfully — a credit grant
  that overtakes the chunk it was issued for is a bug — so a relay must preserve the order of
  everything it carries in one direction of one call.
- **Loss must surface, not silently truncate.** `chunk_seq` (§1) exists for exactly this hop: an
  at-most-once transport can drop a chunk, and the receiver MUST fail the stream with
  `stream_lagged` rather than deliver bytes past the gap. Credits (§4.1) are the only bound on relay
  buffering, since there is no end-to-end TCP backpressure across the hop.
- **The 256 KiB chunk cap** keeps a relayed frame inside the message-size limit of most brokers.

A deployment with no relay routes node calls on the owning replica only; a call landing elsewhere
fails with `node_offline` (retriable). Attacca's implementation — NATS subjects, epoch fencing
against the node row — is documented in `docs/zyris-protocol.md` in the
[Attacca repo](https://github.com/attacca-cc/attacca).

## 10. Websocket close codes

| Code   | Meaning |
|--------|---------|
| `1000` | graceful close |
| `4400` | unsupported protocol version |
| `4401` | authentication failed |
| `4408` | unknown frame kind / malformed frame |
| `4409` | flow-control violation |
