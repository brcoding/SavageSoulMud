# Zebedee / inetd adapter

Implementation: crates/example-mud/src/zebedee.rs. `register(app)` installs a Startup worker and Update pump; `command(world, actor, arg)` handles game-local verbs. Root command registration uses `zebedee` and `i2`.

Configuration is authored data/zebedee.json. `enabled`, literal bind endpoint, mud_name, advertised_port, peers [{name,host,port}], channels are explicit. Restart/copyover reloads it. The initial official MorgenGrauen peer is pinned to 89.58.11.82 UDP 4246. Never accept arbitrary incoming host-list advertisements as trusted peers. Add verified peers deliberately. Peer names match case-insensitively. Source IP and declared NAME/UDP must match one configured peer; many inetd implementations send from a different UDP source port, so the source port itself is not pinned.

A dedicated std::net UDP thread resolves up to 8 addresses per peer at startup, sends queued traffic, decodes bounded packets, and forwards validated packets to Bevy. No socket/DNS I/O runs on the frame loop. Outbound queue 64, inbound 128, frame drain 32, pending 64, fragment groups 16, expiry 15s, aggregate packet 24 KiB. Requests time out after 5s; read-only requests retry twice. Tell/channel never retry to avoid duplicate communication. Remote text is flattened and colour markup escaped. All command paths and deliveries check current online wizard roles. Incoming who/finger/query expose only online wizard names and generic software info; no account/email/IP/room/character sheet disclosure. No intermud mail, file transfer, executable requests, remote commands, or automatic content installation.

Protocol is unauthenticated UDP. The source allowlist reduces unwanted traffic but is not cryptographic authentication. Use a firewall allowlist too on a public host. No auto-send public chat or tells on startup, reconnect, copyover, or tests.

## Hosting and current verification

Local bind is 0.0.0.0:4002. `advertised_port` must equal the public UDP port, forwarded to the WSL/server bound port. WSL Windows localhost TCP forwarding does not provide UDP forwarding. Remote replies may target the advertised port rather than the transient NAT source port. Configure router/Windows/WSL firewall forwarding on the intended hosting machine. Do not report a working network link merely because the worker is bound.

One bounded read-only ping from WSL to MorgenGrauen89.58.11.82:4246 timed out on2026-10-18. DNS for official mg.mud.de resolved to that address. No public chat or tells were sent. Subsequent peers/probes are reported separately by deployment verification. No claim of successful external message routing is made here.

## Protocol scope

Classic `KEY:value|...|DATA:...` framing; DATA consumes the tail and may include delimiters. `$` quoting preserved for numeric-looking strings. ID/UDP are encoded as integers. Incoming and outgoing PKT:name:id:n/total reassembly/splitting supported. Default ASCII interoperability, incoming Latin-1 decoded if UTF-8 invalid. This release sends classic format; optional SMTP-like newline framing is deferred because current published peer implementations default to classic. Requests: ping,query,who,finger,tell,channel. channels supports regular messages and emotes; history/list service explicitly declined. hosts-json results are printed as data and never trusted as configuration. No claim to implement all historic inetd extensions.

## References / dependency review

Primary protocol: https://mg.mud.de/cgi-bin/mudhelp.pl?doc=intermud.basic

Service reference and documented peer observations: https://mudstandards.org/intermud/zebedee/

Crates.io/web search found no maintained Rust implementation of Zebedee's MUD inetd protocol. The unrelated Zebedee secure-tunnel packages do not implement it. Reuse std::net and existing serde instead of adding a protocol framework or sidecar. I3 remains independent and unchanged.

## Optional Windows UDP relay for WSL

Current local Windows receives MorgenGrauen UDP replies, while WSL direct4002 does not. The portable Rust direct mode remains for a public Linux server. For this WSL development machine, use the small standard-library Windows relay:

    python windows-relay.py --bind WINDOWS_WSL_GATEWAY_IP --port4084 --udp-port4002 --key-file PRIVATE_WINDOWS_KEY_PATH --config P:/savsoulmud/grim/data/zebedee.json

Bind the TCP control listener only to the Windows WSL virtual gateway IP, never 0.0.0.0. Generate 32 random bytes and store as 64 lowercase hexadecimal characters in a private key file on Windows and a matching chmod 600 WSL file. Do not put the key in authored config or command-line arguments. Configure the MUD with relay_host (literal gateway IPv4), relay_port 4084, relay_key_file (private WSL path). Empty relay_host uses direct UDP. Windows relay runs hidden; root deployment instructions own process startup. On restart/copyover, the Rust worker reconnects automatically. While disconnected no chat/tell traffic is replayed. Peer allowlist and source/NAME/UDP checks remain in place on both sides.

The authenticated TCP stream uses newline JSON, maximum 64 KiB line; individual datagrams are byte arrays bounded 12000 bytes. No arbitrary UDP destinations allowed. It permits one authenticated MUD connection, rejects unauthenticated clients after 3 seconds, bounds output 256 KiB and traffic 100 packets/sec. UDP binds 4002 separately so inetd replies to the advertised port reach the gateway. The frame queue and fragment assembly stay bounded in Rust.

Verification 2026-10-18: WSL ping to MorgenGrauen, UNItopia and Wunderland all timed out; the same native Windows 4002 ping to official mg.mud.de 89.58.11.82:4246 immediately returned `MorgenGrauen is alive.` Offline `test_relay.py` passed authentication, pinned endpoint roundtrip/source port, source metadata, and forbidden destination rejection. No public chat/tells sent. Live relayed Rust/MUD verification is a separate deployment step.



For this development machine, run tools/zebedee/start-relay.ps1 before starting the MUD after a Windows/WSL restart. It creates a private per-user relay key, copies it into WSL mode0600, detects the WSL gateway, updates the local relay configuration and starts the relay hidden. It is idempotent; copyover reconnects to the running relay. It does not install a Windows startup task. Direct Linux hosting should set relay_host to an empty string and use UDP directly.

For a new checkout, copy `data/zebedee.example.json` to the ignored
`data/zebedee.json` before configuring the adapter. Never store keys in either file.
