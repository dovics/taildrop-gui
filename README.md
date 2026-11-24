# Taildrop

Files over your tailnet: automatic device discovery, and a receiver that
actually accepts. Transport is wire-compatible with LocalSend v2.2, so an
unmodified LocalSend client — including the phone apps — can send to Taildrop and
receive from it.

## Status

Prototype, verified end-to-end on a real tailnet (2026-11-24).

| Verified | How |
|---|---|
| Reads the peer list from `tailscaled` | unix socket, no root needed |
| Peer reachability + identity probe | `POST /api/localsend/v2/register` |
| Taildrop → Taildrop transfer | 3 MiB, SHA-256 matched |
| Taildrop → official `localsend-cli` | 2 MiB, accepted and saved by the unmodified client |
| Third-party client → Taildrop | 2 MiB, protocol-compliant receive |
| Fingerprint pinning | wrong fingerprint refused before any bytes move |
| PIN gate | missing/wrong → 401, correct → accepted |
| Path traversal | `../../etc/passwd` → saved as `passwd` inside the inbox |
| Absolute path | `/tmp/pwned` → saved as `pwned` inside the inbox, `/tmp/pwned` never created |
| Checksum mismatch | 422, partial file deleted |
| Forged upload token | 403 |
| Name collision | second copy saved as `name (2).ext`, never overwritten |

Not yet done: GUI, drag-and-drop, tray, send-to-menu, mobile, resume-on-failure,
pairing/quick-save, history database.

## Layout

```
core/           transport and discovery, no UI
  src/tailscale/    LocalAPI status → peer list (unix socket; CLI on Windows)
  src/localsend/    v2.2 protocol
    types.rs          wire types, verbatim from the spec
    identity.rs       self-signed cert; fingerprint is the device identity
    sender.rs         probe, prepare, stream upload
    receiver.rs       real server: policy, sanitising, atomic commit
cli/            `taildrop-cli` binary
```

## Use

```sh
cargo build --release

# who is out there, and who can receive?
./target/release/ferry peers

# receive into ~/Downloads/Taildrop
./target/release/ferry serve

# receive into a specific directory, requiring a PIN
./target/release/ferry serve --inbox ~/inbox --pin 1234

# send
./target/release/ferry send a.zip b.pdf --to desktop
./target/release/ferry send a.zip --to desktop --expect-fingerprint <64 hex chars>
```

`--to` accepts a peer hostname, its MagicDNS name, or a literal tailnet IP.

## Decisions worth knowing

**Ports.** Taildrop listens on 53318 and probes both 53318 and LocalSend's 53317,
so a machine running only LocalSend is still found. Two servers on one port is a
conflict nobody needs.

**Trust model.** Identity is the SHA-256 of the device's self-signed
certificate, uppercase hex — the same form LocalSend uses. TLS is accepted
without CA verification *by design* (peer certs are self-signed), and trust
comes from comparing fingerprints instead. Anything that skips
`verify_peer_fingerprint` is opting out of the only real check there is.

**Where received files go.** Into one inbox, with a sanitised name. The filename
arrives from the network and is joined to a path on disk, so it is treated as
hostile: last component only, no control characters, Windows reserved names
escaped, length capped. Files are written to `*.part`, hash-verified, then
renamed — a corrupt or truncated file never appears under its final name.

**One session at a time.** A second sender during an active upload gets 409,
matching LocalSend. This keeps the inbox unambiguous; if it becomes annoying,
the fix is per-sender subdirectories, not concurrent writes to one directory.

**No global request timeout.** `prepare-upload` blocks until a human on the
receiving side accepts, which can take minutes. A whole-request timeout reports
"send failed" while the receiver is still showing its prompt — this was an
actual bug here, found by testing, not by reading.

**Resume.** Not implemented. LocalSend's protocol has no `Range` support, so a
dropped transfer restarts from zero. Worth adding as a Taildrop-specific chunking
layer above the protocol rather than by changing the wire format.

## Licensing note

Wire protocol and identity scheme follow LocalSend (Apache-2.0). This code is
original; if you vendor LocalSend sources instead of reimplementing, keep their
LICENSE/NOTICE and avoid their name and marks.
