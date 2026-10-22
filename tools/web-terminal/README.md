# Browser MUD terminal

`https://savsoul.com/playnow.html` renders a self-hosted xterm.js 6 terminal with
a separate command/password field. The normal MUD account/character login is
used. The Connect button opens same-origin secure WebSockets; no account
credentials, command history or transcripts are saved by the web application.
There is no automatic reconnect or queued-input replay. Screen reader support
is available in the client; use `screenreader on` in game for plain output.

The standalone `bridge.py` uses aiohttp 3.14.5 and telnetlib3 5.0.1, bound only to
`127.0.0.1:4086`. It can reach only the game's trusted ingress 127.0.0.1:4003.
Apache terminates TLS and proxies only `/mud-websocket` in the savsoul.com:443
virtual host using stock `mod_proxy`/`mod_proxy_wstunnel`. Forward proxying is off.
Origin/Host are restricted to this website. Apache appends its actual client
address as the final X-Forwarded-For hop; the bridge trusts only that final hop
from its loopback proxy, ignoring client-supplied earlier IP claims. The bridge
writes a validated PROXY header before Telnet negotiation,
preserving player IP bans/rate limits. Original browser source ports are
unavailable and represented as 1 in the PROXY header. IPv4 and IPv6 are supported.

Telnet parsing, charset, NAWS and password ECHO are handled by telnetlib3, rather
than another custom parser. A small callback wrapper observes ECHO-only changes
after telnetlib3's asynchronous option processing; controls precede prompt data.
Window changes update NAWS. xterm renders ANSI without injecting HTML. Only the
fit addon is enabled; no clipboard or logging addons are loaded. UI libraries
and licenses are pinned under `website/vendor/VERSIONS.json` with npm integrity
digests, so browser sessions do not depend on a third-party CDN.

Limits: 64 total sessions, 4 per IP, 10 connection attempts/minute/IP, 2048 ASCII
characters/line, 30 commands/10 seconds, 4096-byte inbound WebSocket frames, 128 KiB
unacknowledged output, 30-second render-ACK timeout, 10-second send timeout, bounded
Telnet read buffer. ACK sequence numbers must not exceed the last sent number.
EOF waits briefly for final output consumption. Both directions close/cancel
together. No payloads are written to service or Apache access logs.

Apache 2.4.7 tunnels even rejected WebSocket upgrades. Every non-upgraded backend
HTTP response therefore force-closes its connection, including errors/404s, and
HTTP keepalive is disabled. Without this, a pipelined or reused HTTP connection
could bypass Apache header processing. Only a prepared 101 WebSocket is exempt.

Remote service: `/etc/init/savsoul-web-terminal.conf`; `status savsoul-web-terminal`.
Run as the locked savsoul account with the private Python virtual environment.
Health: `http://127.0.0.1:4086/health`. Logs: `/var/log/upstart/savsoul-web-terminal.log`.
Restarting this bridge disconnects browser sessions; restarting/gracefully reloading
the website does not require restarting the game. Game copyover retains the
bridge's upstream sockets for in-game characters as with other Telnet clients.

Offline checks: `python -m unittest discover -s tools/web-terminal -p test_bridge.py`.
These verify fragmented negotiation, ECHO-only controls, hidden password input,
final EOF output, real-IP PROXY framing and untrusted-origin/target rejection.
Public smoke checks stop at Rawr's password prompt and send no password or chat.

References: https://xtermjs.org/docs/ ; https://telnetlib3.readthedocs.io/ ;
https://httpd.apache.org/docs/2.4/mod/mod_proxy_wstunnel.html .
