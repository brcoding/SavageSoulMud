/* Same-origin MUD terminal. No credentials, commands or transcripts are stored. */
(() => {
  'use strict';
  const mount = document.querySelector('#mud-terminal');
  if (!mount) return;
  const connect = document.querySelector('#terminal-connect');
  const disconnect = document.querySelector('#terminal-disconnect');
  const status = document.querySelector('#terminal-status');
  const form = document.querySelector('#terminal-command');
  const input = document.querySelector('#mud-command');
  const label = document.querySelector('#command-label');
  const send = form.querySelector('button');
  const reader = document.querySelector('#terminal-reader');
  const terminal = new Terminal({
    cursorBlink: false, disableStdin: true, convertEol: false, scrollback: 2000,
    fontSize: 15, fontFamily: 'Consolas, "Liberation Mono", monospace',
    minimumContrastRatio: 4.5, screenReaderMode: false,
    theme: {background: '#080b08', foreground: '#e8eee1', cursor: '#c2d6a1'},
    linkHandler: {activate(event, text) {
      try { const url = new URL(text); if (['https:', 'http:'].includes(url.protocol)) window.open(url.href, '_blank', 'noopener,noreferrer'); } catch {}
    }}
  });
  const fit = new FitAddon.FitAddon();
  terminal.loadAddon(fit); terminal.open(mount); fit.fit();
  let socket = null, secret = true, ready = false, resizeTimer;
  function mode(value) {
    if (secret !== value) input.value = '';
    secret = value; input.type = value ? 'password' : 'text';
    label.textContent = value ? 'Password (hidden)' : 'Command or login response';
    input.placeholder = value ? 'Password is hidden' : 'Type here; press Enter to send';
  }
  function enabled(value) { ready = value; input.disabled = !value; send.disabled = !value; }
  function tell(text) { status.textContent = text; }
  function sendPacket(packet) {
    if (socket && socket.readyState === WebSocket.OPEN && socket.bufferedAmount < 65536) socket.send(JSON.stringify(packet));
  }
  function resize() {
    fit.fit();
    if (ready) sendPacket({type: 'resize', cols: Math.max(20, Math.min(240, terminal.cols)), rows: Math.max(8, Math.min(100, terminal.rows))});
  }
  new ResizeObserver(() => { clearTimeout(resizeTimer); resizeTimer = setTimeout(resize, 160); }).observe(mount);
  reader.addEventListener('change', () => { terminal.options.screenReaderMode = reader.checked; });
  document.querySelector('#terminal-clear').addEventListener('click', () => terminal.clear());
  connect.addEventListener('click', () => {
    if (socket && socket.readyState < WebSocket.CLOSING) return;
    if (location.protocol !== 'https:') { tell('Open this page over HTTPS to connect securely.'); return; }
    input.value = ''; mode(true); enabled(false); terminal.reset();
    tell('Connecting securely…'); connect.disabled = true; disconnect.disabled = false;
    const active = new WebSocket(`wss://${location.host}/mud-websocket`); socket = active;
    active.addEventListener('open', () => tell('Connected securely. Follow the game’s login prompts below.'));
    active.addEventListener('message', event => {
      if (socket !== active) return;
      let packet; try { packet = JSON.parse(event.data); } catch { active.close(); return; }
      if (packet.type === 'mode' || packet.type === 'output') {
        mode(packet.secret === true);
        if (!ready) { enabled(true); resize(); input.focus(); }
      }
      if (packet.type === 'output' && typeof packet.text === 'string' && Number.isSafeInteger(packet.seq)) {
        terminal.write(packet.text, () => { if (socket === active) sendPacket({type: 'ack', seq: packet.seq}); });
      } else if (packet.type === 'notice') tell(packet.text);
    });
    active.addEventListener('error', () => tell('Connection unavailable. Try again in a moment, or use a MUD client on port 4000.'));
    active.addEventListener('close', () => {
      if (socket !== active) return;
      input.value = ''; enabled(false); mode(true); connect.disabled = false; disconnect.disabled = true;
      tell('Disconnected. Select Connect to log in again.');
    });
  });
  disconnect.addEventListener('click', () => { input.value = ''; if (socket) socket.close(1000); });
  form.addEventListener('submit', event => {
    event.preventDefault();
    if (!ready || !socket || socket.readyState !== WebSocket.OPEN) return;
    const text = input.value;
    if (text.length > 2048 || /[^\x20-\x7e]/.test(text)) { tell('Use printable English keyboard characters, up to 2048 per command.'); return; }
    if (socket.bufferedAmount >= 65536) { tell('Connection is busy. Please wait and try again.'); return; }
    // Secret submissions never enter xterm, history, storage or the console.
    if (!secret) terminal.write(text + '\r\n');
    sendPacket({type: 'line', text}); input.value = ''; input.focus();
  });
  enabled(false); mode(true);
})();
