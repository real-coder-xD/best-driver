"""Shared local HTTP fixture server for zero-flake E2E pytest suites."""

from __future__ import annotations

import http.server
import json
import threading
from typing import Iterator

import pytest


class _FixtureHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args) -> None:
        return

    def do_GET(self) -> None:
        if self.path == "/" or self.path.startswith("/?"):
            html = """<!doctype html>
<html>
<head><title>best-driver E2E Fixture</title></head>
<body>
  <h1 data-testid="hero-title">Welcome Portal</h1>
  <form id="login-form">
    <label for="email-in">Email Address</label>
    <input id="email-in" type="email" placeholder="you@example.com" />
    <label>Remember Me <input id="rem-cb" type="checkbox" /></label>
    <select id="plan-sel" aria-label="Plan">
      <option value="free">Free</option>
      <option value="pro">Pro Tier</option>
    </select>
    <button id="submit-btn" type="button" onclick="window.__submitted = document.getElementById('email-in').value">Sign In</button>
    <button id="dup-btn" type="button" class="dup">Action</button>
    <button id="dup-btn-2" type="button" class="dup">Action</button>
  </form>

  <a id="dl-link" href="/download/sample.txt" download="sample.txt">Download Sample</a>
  <a id="popup-link" href="/popup" target="_blank">Open Popup</a>
  <input id="file-in" type="file" multiple />

  <div id="overlay-zone" style="position:relative;width:220px;height:60px;margin-top:16px;">
    <button id="covered-btn" style="width:180px;height:44px;" onclick="window.__coveredClicks = (window.__coveredClicks||0)+1">Covered Action</button>
    <div id="blocker" style="position:absolute;inset:0;background:rgba(0,0,0,0.4);"></div>
  </div>

  <div id="mover-zone" style="margin-top:16px;">
    <div id="spacer" style="height:0px;"></div>
    <button id="shifting-btn" style="width:160px;height:36px;" onclick="window.__shiftClicks = (window.__shiftClicks||0)+1">Shifting Button</button>
  </div>

  <button id="delayed-enable" disabled onclick="window.__enabledClicks = (window.__enabledClicks||0)+1">Enable Soon</button>

  <div id="closed-host"></div>

  <iframe id="reloading-frame" style="margin-top:20px;width:260px;height:120px;" src="/frame/v1"></iframe>

  <script>
    window.__submitted = null;
    window.__coveredClicks = 0;
    window.__shiftClicks = 0;
    window.__enabledClicks = 0;

    // Closed shadow root
    const ch = document.getElementById('closed-host');
    const sr = ch.attachShadow({mode: 'closed'});
    sr.innerHTML = `
      <input id="cs-input" placeholder="Secret Code" value="init" />
      <button id="cs-btn" onclick="window.__csClicked = true">Verify Human</button>
    `;

    // Helpers invoked by tests
    window.__unblockIn = (ms) => setTimeout(() => {
      const b = document.getElementById('blocker');
      if (b) b.style.display = 'none';
    }, ms);

    window.__startShift = () => {
      let step = 0;
      const iv = setInterval(() => {
        step += 1;
        document.getElementById('spacer').style.height = (step * 18) + 'px';
        if (step >= 5) clearInterval(iv);
      }, 30);
    };

    window.__enableIn = (ms) => setTimeout(() => {
      document.getElementById('delayed-enable').disabled = false;
    }, ms);

    window.__reloadFrame = (ver) => {
      document.getElementById('reloading-frame').src = '/frame/' + ver;
    };
  </script>
</body>
</html>""".encode("utf-8")
            self._send(200, "text/html; charset=utf-8", html)
        elif self.path.startswith("/frame/"):
            ver = self.path.split("/")[-1]
            html = f"""<!doctype html><html><body>
              <h2 id="f-ver">Frame {ver}</h2>
              <button id="f-btn" onclick="document.getElementById('f-out').textContent='clicked-{ver}'">Press {ver}</button>
              <div id="f-out">idle</div>
            </body></html>""".encode("utf-8")
            self._send(200, "text/html; charset=utf-8", html)
        elif self.path == "/popup":
            html = b"<!doctype html><html><head><title>Popup Target</title></head><body><h1 id='pop-h1'>Popup Ready</h1></body></html>"
            self._send(200, "text/html; charset=utf-8", html)
        elif self.path == "/download/sample.txt":
            data = b"BEST-DRIVER-V040-DOWNLOAD"
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Disposition", 'attachment; filename="sample.txt"')
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        elif self.path.startswith("/api/data"):
            body = json.dumps({"ok": True, "source": "server", "path": self.path}).encode("utf-8")
            self._send(200, "application/json", body)
        elif self.path == "/blocked.js":
            self._send(200, "application/javascript", b"window.__blocked = true;")
        else:
            self._send(404, "text/plain", b"not found")

    def do_POST(self) -> None:
        ln = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(ln).decode("utf-8", errors="replace")
        body = json.dumps({"hdr": self.headers.get("X-Trace-Test"), "body": raw}).encode("utf-8")
        self._send(200, "application/json", body)

    def _send(self, code: int, ctype: str, body: bytes) -> None:
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


@pytest.fixture(scope="session")
def server_url() -> Iterator[str]:
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _FixtureHandler)
    port = srv.server_address[1]
    thread = threading.Thread(target=srv.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{port}"
    finally:
        srv.shutdown()
        srv.server_close()
