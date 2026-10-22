# Grapevine

This adapter is separate from I3 and Zebedee. Its Rust source is game-local, registered by SavageSoulPlugin, and exposes wizard-only `grapevine` / `gv` commands. The endpoint is certificate-verified `wss://grapevine.haus/socket`. No additional listener or server process is required.

## Setup

Register the development game at https://grapevine.haus/config, using the display name The Savage Soul Dev. The game configuration page supplies its Client ID and Client Secret. Grapevine's short game name is assigned there; it is distinct from the display name used by the MUD.

From the live MUD folder on Windows:

    ./configure-grapevine.ps1

Or from a WSL/Linux terminal in the MUD folder:

    python3 tools/grapevine/configure.py

The interactive helper hides the secret, validates input, and atomically stores the two credentials under ~/.local/state/savsoul-grapevine/credentials.json, mode0600 in a mode0700 directory. It honors XDG_STATE_HOME. Keep data/grapevine.json credentials_file aligned if you customize that location. Credentials never belong in authored data, a source checkout, logs, or a command-line argument.

Then use `gv reload` in the MUD. Missing credentials leave a clear disconnected status without affecting login, combat, I3, or Zebedee. Copyover reloads private state and reconnects.

## Commands and behavior

See `help grapevine` for status, games, who, channels, listen/unlisten, channel chat, tells, recent channel activity and reload. The aliases and help entry are hidden from mortals, with a fresh role check in the adapter as well as the session command gate. Inbound deliveries check the current connected wizard role, and only wizard display names are published in presence. No accounts, emails, IPs, room locations or sheets are sent.

Public channel history is bounded and stored at activity_file if configured. Private tells are not retained. Authenticated connection state and subscribed channel acknowledgements are separate from queueing requests; no queued chat/tell is replayed after disconnect. Network errors use bounded reconnect backoff and restart hints. Unknown/unimplemented protocol support flags are never advertised.

Grapevine dynamically creates valid channels when subscribed. There is no unrestricted `listen all` discovery operation: `gv channels` lists local subscriptions. Defaults are gossip and testing. No public chat/tells are automatically sent by startup, setup, reconnect or verification.

Primary protocol documentation: https://grapevine.haus/docs

## Hosting

Requires outbound TCP443 and working certificate validation, plus the private credentials file. No GPU or model is involved. The adapter's worker runs outside the game frame loop with bounded queues and protocol messages. Move the private credentials separately when moving to Linux hosting; disable the development connection before reusing that same game registration remotely.
