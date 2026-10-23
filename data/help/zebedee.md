# Zebedee (inetd / Intermud 2)

All `zebedee` and `i2` commands are wizard-only. This is a separate network from I3.

- `zebedee status`: UDP worker state and peers that replied this session.
- `zebedee muds`: configured peers and last replies (not a global online list).
- `zebedee ping MorgenGrauen`
- `zebedee who MorgenGrauen`
- `zebedee finger Name@MorgenGrauen`
- `zebedee mudlist MorgenGrauen`: ask for hosts-json; results are information only.
- `zebedee query MorgenGrauen: commands`
- `zebedee channels`
- `zebedee listen all` or `zebedee listen intermud`
- `zebedee unlisten all`
- `zebedee chat intermud Hello`
- `zebedee tell Name@MorgenGrauen: Hello`

Listening is a local preference, cleared on logout/restart. Channel chat is sent to configured peers; reaching a peer does not imply that every other MUD is reached. There is no centralized router or automatic peer discovery. Add verified discovered peers to data/zebedee.json and copyover. Respect each network channel's language and purpose.

UDP binding is not evidence of network reachability. `status` distinguishes configured peers from received replies. A development PC behind NAT/WSL may need UDP forwarding of port4002 to receive replies or unsolicited messages. TCP localhost forwarding for the MUD does not forward UDP. See tools/zebedee/README.md for configuration and limitations.
