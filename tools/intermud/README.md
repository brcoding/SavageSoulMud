# Savage Soul Intermud-3

The local development MUD uses **The Savage Soul Dev** on the legacy I3 network, initially for wizards. `data/intermud.json` controls the game adapter; `gateway.yaml` controls the independent gateway. Reload the game after editing adapter settings; restart `savsoul-intermud` after editing gateway settings.

The gateway is [LuminariMUD/Intermud3](https://github.com/LuminariMUD/Intermud3), pinned at `3317042aebb9ab463489896dbe50ce13cd123981`, version 0.4.7-beta. Its repository LICENSE is the Unlicense. The source is installed at `/home/barry/.local/share/savsoul-intermud/gateway`; Python dependencies are pinned in `requirements.lock`. `patch_gateway.py` changes fresh startup discovery IDs from 0 to -1, matching the recovered Dead Souls client. Without this change the live router supplied no channels; with it all 169 channel records synchronized during setup. No LPC parser is duplicated inside Grim.

## Operation

- Game: localhost:4000, user service `savsoul-grim`.
- Gateway: user service `savsoul-intermud`.
- Authenticated game API: Linux loopback TCP 4082; HTTP health on loopback 4081.
- Ready check: `curl --fail http://127.0.0.1:4081/health/ready`.
- Logs: `journalctl --user -u savsoul-intermud`.
- Installation/recovery: from WSL, `bash /mnt/p/savsoulmud/grim/tools/intermud/install-wsl.sh`.

The API key, gateway secret and assigned router password live under `/home/barry/.local/state/savsoul-intermud` with owner-only permissions. Keep that directory when updating or moving hosts; never copy its contents into authored data or commit them. A replacement host should receive private credentials through an operator-controlled transfer.

The confirmed network name is distinct from the original MUD. Public metadata uses the recovered administrator contact; advertised player port is 0 because this is a private localhost development server. Local telnet still uses 4000. OOB file/mail services are disabled. The gateway persists router credentials and stays connected while Grim copies over; the game reconnects its authenticated API session and republishes eligible online names. Account email, password, IP address and room locations are not sent.

`help intermud` documents status, discovery, listening, channel chat, tells and remote who. Text commands derive the sender from the logged-in character. A gateway acknowledgement is acceptance, not proof of remote delivery. No queued player message is replayed after disconnect. Network protocol packets are bounded, requests have timeouts, and network I/O runs off the game loop.

## Validation

Focused Rust tests exercise authenticated fake-gateway framing, request/response, cancellation, command parsing and remote-text escaping. Live checks completed startup/credential persistence and authenticated status/MUD/channel discovery. They did not send chat, tells or contact another player. Channel and MUD counts change with the network. Upstream specification: https://wotf.org/i3/.

## Recent channel activity

`i3 activity` summarizes recently observed public channel traffic by channel. `i3 recent` shows the latest 20 retained messages; `i3 recent intergossip` filters to one channel. Each entry identifies its age, channel, speaker and originating MUD. This is an observation of channels this MUD listens to, not a network-wide channel-usage census.

The adapter retains at most 200 public channel messages in the local Linux file named by `activity_file` in `data/intermud.json`. Collection starts with the activity update. Private tells, private emotes, who answers and errors are excluded. The file survives game copyovers; it contains public message text and must not be confused with the private router credentials. Removing the file while the MUD is stopped clears the retained history.

## Wizard channel subscriptions

All `i3`, `intermud` and `imud` commands are wizard-only, as are incoming traffic and retained history. This is enforced at session dispatch and again using the character's current role in the adapter. The legacy `wizard_only` config field cannot enable access for mortal characters.

`i3 listen all` requests all discovered public channels. Restricted channels still require network permission and are not included in the bulk public subscription. Joins are paced so the gateway is not flooded; the command reports the queued work. `i3 unlisten all` leaves the desired channels. Desired subscriptions persist in the local Linux JSON file named by `subscriptions_file`, allowing reconnection and copyover to restore them. Adding the capability does not subscribe the MUD to every channel automatically.
