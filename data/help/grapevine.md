Grapevine (wizard only)
======================

grapevine status
gv channels
gv listen gossip
gv unlisten gossip
gv games
gv who
gv who GameName
gv chat gossip Hello everyone
gv tell Name@GameName: Hello there
gv recent
gv recent gossip
gv reload

After connecting, use gv listen gossip to opt into live messages. Unlisten only changes your listening; other wizards and shared channel history are unaffected.

Grapevine is separate from I3 and Zebedee. Messages show the channel and Name@Game. Only connected wizards can use these commands or receive network messages. Only online wizard display names are shared; account details, room locations and character sheets are not published.

Register the development game on https://grapevine.haus, then run configure-grapevine.ps1 from the MUD folder. It asks for the existing Client ID and Client Secret and stores them privately in WSL. Do not paste credentials into chat, authored content or this help file. Use gv reload after configuration.

The connection uses certificate-verified WSS. Disconnections trigger reconnects; chat and tells are not replayed. Status distinguishes missing credentials, connecting and authenticated. Channel names contain 3-15 letters, underscore or hyphen. There is no universal channels list: subscribing can create a public channel on Grapevine. Only join names you intend to use.

Recent history contains public channel messages only; private tells are excluded. A send acknowledgement is reported separately from local queuing. Remote availability and delivery depend on Grapevine and the receiving game.
