# Mudlet TLS ingress

Independent Python stdlib TLS service on **127.0.0.1:4001** forwards to the MUD's dedicated trusted PROXYv1 ingress **127.0.0.1:4003**. Ordinary Telnet **4000** remains available and never trusts PROXY headers. TLS session state lives in this service, so MUD copyover can transfer its ordinary backend sockets without breaking encryption sessions.

## Installation (WSL/Linux)

Run `bash tools/mudlet-tls/install.sh`. Requires Python3, OpenSSL and systemd user services; no pip packages. The installer creates private local key/certificate state under `~/.local/state/savsoul-tls`, copies the application to Linux storage and installs `savsoul-tls.service`, without starting it or changing the MUD service.

Configure `GRIM_PROXY_PORT=4003` in the MUD service's environment and deploy the updated binary; the setting survives normal copyover inheritance. The trusted listener is disabled when unset. It always binds loopback, regardless of `GRIM_BIND_HOST`. After the backend is active:

```sh
systemctl --user enable --now savsoul-tls.service
systemctl --user status savsoul-tls.service
journalctl --user -u savsoul-tls.service -n 30
openssl s_client -connect localhost:4001 -servername localhost -CAfile "$HOME/.local/state/savsoul-tls/localhost.crt" -verify_return_error
```

In Mudlet use `localhost`, port4001 and check **Secure**. The generated certificate is self-signed for local development. Compare its SHA256 fingerprint with installer output before trusting it through the client's certificate trust controls; do not disable certificate verification globally. Certificates expire after365days; replace them deliberately and restart only this service. A TLS proxy restart disconnects TLS users; a MUD copyover does not. Keep keys out of the editor/data repo.

## Hosting later

Provide an operator-managed certificate chain and matching key with `SAVSOUL_TLS_CERT=/path/fullchain.pem SAVSOUL_TLS_KEY=/path/privkey.pem SAVSOUL_TLS_HOST=0.0.0.0 bash tools/mudlet-tls/install.sh`. Use a certificate valid for the public hostname; arrange renewal/reload and public firewall routing independently. Public binding is explicit and never the local default. Existing cert/key pairs are never overwritten by installation. The host/certificate paths must be service-readable.

Do not expose4003 through a public port forward: all local processes able to connect to that dedicated listener can assert PROXY addresses. Public4000 cannot assert addresses. Under WSL/Windows port forwarding, the kernel may already present a forwarded connection as loopback; the proxy preserves the true peer seen by its own socket rather than guessing an original address. Client-advertised TLS capability does not prove an encrypted connection.

The Rust backend caps PROXYv1 headers at108bytes and2seconds, accepts only TCP4/TCP6 with valid matching addresses/ports, and admits no more than64pending header reads. SO_REUSEPORT allows the copyover successor to bind4003 while its predecessor briefly retains the listener. TLS relay has128active connections,16KiB transfer chunks, drain deadlines, a10second TLS handshake deadline, and no game idle timeout. No raw traffic or credentials are logged.

MSSP should advertise `TLS=4001` only after the configured endpoint is healthy. Test backend and TLS separately; a TCP accept alone does not validate the certificate. Insecure local certificate acceptance is not public hosting readiness.

Run focused proxy tests with `python3 -m unittest discover -s tools/mudlet-tls -p 'test_*.py'`.

Trusted TLS proxy ingress can be explicitly configured by GRIM_PROXY_PORT or data/tls-proxy-port.txt (port only). The file allows enabling it across copyover without changing the predecessor process environment. It binds only 127.0.0.1; the public Telnet listener never accepts PROXY identity headers. Start the separate TLS proxy after enabling the backend.
