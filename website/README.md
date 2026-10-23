# Public Savage Soul website

Static source for https://savsoul.com, served by Apache from `/var/www/lpmud`.
Edit `build.py` for shared navigation and page copy, `site.css` for styling, and
`content/` for preserved original lore/news. Run `python build.py`; deploy the
generated files in `dist/` into the existing webroot. No build dependencies
are required. The browser terminal uses pinned, self-hosted
xterm.js assets; other pages need no JavaScript. Existing images, maps, downloads, historical
documentation, validation files and unrelated applications stay in place.

Connections: `savsoul.com:4000` (standard), `savsoul.com:4001` (SSL/TLS enabled).
The Connect page also offers an encrypted browser terminal through the private
WebSocket bridge in `tools/web-terminal`. The old Flash/Mudportal embed and LPC
who.php lookup are not used by the current pages.

Apache uses the PositiveSSL leaf certificate, matching private key and CA bundle
at `/etc/apache2/certs/savsoul_com.crt`, `/etc/apache2/certs/savsoul.key`, and
`/etc/apache2/certs/savsoul_com.ca-bundle`. The certificate also covers
`www.savsoul.com`. It expires May5,2027. The MUD TLS proxy holds a separate installed
copy under `/home/savsoul/state/tls`; update both when reissuing the certificate.

Back up every overwritten page/config outside the public webroot before deploy.
Validate Apache config before a graceful reload. Keep HTTP certificate-validation
paths accessible when redirecting other requests to HTTPS.
