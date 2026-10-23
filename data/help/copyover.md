Restart with connected sessions
===============================

copyover [seconds]

Warn players, then hand connected sessions to a successor server process. The default countdown is 30 seconds. Supported on the current Linux/WSL service; native Windows cannot use its Unix descriptor handoff. A failed copyover can still disconnect clients.
