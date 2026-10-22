# Intermud

Intermud connects this development MUD to the Intermud-3 network as **The Savage Soul Dev**. Every Intermud command and received message is available only to wizards. This restriction cannot be disabled by the legacy `wizard_only` configuration field.

- `intermud status` — gateway and router connection status.
- `intermud muds` — online MUDs.
- `intermud recent [channel]` — last 20 observed public messages, oldest first, with channel, speaker, MUD and age.
- `intermud activity` — per-channel retained message counts and latest activity, newest activity first.
- `intermud channels` — available network channels.
- `intermud listen all` — discover and request all currently public channels; restricted/private channels are skipped.
- `intermud unlisten all` — cancel requested subscriptions and queue leaving channels.
- `intermud listening` — show requested subscriptions, gateway acknowledgements, queue progress and failures.
- `intermud listen intergossip` — subscribe this MUD to a channel.
- `intermud unlisten intergossip` — stop receiving that channel.
- `intermud chat intergossip Your message` — speak on a network channel.
- `intermud tell name@MudName Your message` — send a private tell.
- `intermud tell name@Mud Name: Your message` — tell someone at a MUD whose name contains spaces.
- `intermud who Mud Name` — request that MUD's online-player list.

`i3` is an alias for `intermud`; `imud <channel> <message>` sends channel chat. Channel listening is shared across the MUD. Incoming private tells and who answers go only to the named online character. Online wizard names and titles are published so remote users can find and contact them. Account information is never published.

A successful send means the gateway accepted it; the network has no delivery receipt. Later failures may arrive as a separate error. Messages are not replayed when a connection drops. The gateway reconnects automatically and survives game copyovers.

Operators configure `data/intermud.json`. Restart/copyover the game to reload it. Keep the API key in the private Linux file named by `api_key_file`; do not put credentials into help or authored content. The gateway and its API run locally in WSL. Connecting to Intermud does not make the local game publicly reachable.

Recent history starts as messages arrive on channels this MUD listens to; it is not a network-wide activity report. Up to 200 public channel messages/emotes are retained across copyovers when `activity_file` is configured. Private tells and private emotes are never stored here. `recent` and `activity` work while the gateway is disconnected. Counts describe the retained window only. Operators set `activity_file` to a file in the private local Linux `/home` state directory; an empty value keeps history only until copyover.

Subscriptions are paced at about one per second to respect gateway limits. `listen all` queues requests rather than claiming immediate access; router restrictions still apply. It covers public channels known when issued, so issue it again to discover newly added channels. Desired subscriptions persist across copyovers and reconnects when `subscriptions_file` points to a private local Linux state file. `unlisten all` saves an empty desired list so the previous subscriptions do not return on the next copyover. Joining a channel enables listening; it does not send a chat message.

`unlisten all` silences those incoming channels immediately while the paced leave requests drain. Recent history already collected remains available to wizards.

Incoming discovery audit notices are local and wizard-only, for example:
SYSTEM <intermud> [Crawler@MUIndex requests the who list]
Who and finger requests are reported, bounded to one notice per second. These are not channel messages and are not sent back over I3. Presence replies remain limited to the existing public wizard presence data.
