Client protocols

Use protocols to see what your client negotiated. Preferences:
  protocols links on
  protocols links off
  protocols sound on
  protocols sound off
  protocols sound test

Sound is optional and starts disabled. The sample needs a reachable media_base_url in data/mudlet.json. Local development serves it from http://localhost:4080/media/. Sound playback depends on client settings.

GMCP sends your Savage Soul vitals, attributes, inventory and visible room exits. MSDP supports LIST, SEND, REPORT, UNREPORT and RESET. Injury is an injury description, not an invented hit-point pool. Room.Info uses stable compact numeric room IDs shared with the downloadable Mudlet map. Native identity remains in grim_id.

NAWS reports window size for room wrapping. CHARSET negotiates UTF-8. MTTS/TTYPE and the safe subset of NEW-ENVIRON/MNES describe client capabilities. MSSP provides public server information. Password ECHO negotiation, EOR, optional MXP links, MSP and GMCP Client.Media are supported. Legacy ATCP provides Room.Brief when GMCP is unavailable.

Connect securely to savsoul.com port 4001 using the trusted public certificate. Port 4000 is ordinary Telnet. Audio and links are convenience features; text commands remain available.

MMCP is Mudlet's client-to-client Chat protocol: configure it in Mudlet's Chat settings. It does not connect to a server-side MUD listener. Aardwolf's channel 102 is game-specific and is not emulated here.

Mudlet saved sign-in
--------------------
Set your character name and password in Mudlet's connection profile, enable
GMCP, and reconnect. Char.Login version 1 sends saved credentials without
typing them into the game window. Account email opens character selection;
email:character selects an owned character. Ordinary interactive login remains
available. MOTD and accessibility prompts still apply. OAuth/browser sign-in
and passwordless reconnect tokens are not offered.

Mudlet maps
-----------
Open Mudlet's mapper after connecting. The MUD offers a public map and a small
SavsoulMapper package that follows your position and changes floor with you.
Enable server GUI/package downloads in Mudlet if you previously disabled them.
The map omits hidden exits and secret-only rooms. Gate/puzzle movement checks
still apply. Positions are inferred from exits, so unusual connections can have
imperfect geometry. To refresh an installed map, use Mudlet's Mapper settings
and re-download it.
