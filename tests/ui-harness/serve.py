"""Serves the real ui/ pages in a normal browser with a fake backend (mock-tauri.js).

Usage: python3 tests/ui-harness/serve.py   then open http://localhost:8765/tests/ui-harness/
Synthetic data only. Nothing here talks to Dexcom, Nightscout or the keychain.
"""

import http.server
import os
import socketserver

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
MOCK = '<script src="/tests/ui-harness/mock-tauri.js"></script>'


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=ROOT, **kwargs)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_GET(self):
        path = self.path.split("?")[0]
        if path.startswith("/ui/") and path.endswith(".html"):
            with open(os.path.join(ROOT, path.lstrip("/")), encoding="utf-8") as f:
                html = f.read().replace("<link", MOCK + "\n  <link", 1)
            body = html.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()


socketserver.TCPServer.allow_reuse_address = True
with socketserver.TCPServer(("127.0.0.1", 8765), Handler) as httpd:
    print("UI harness on http://localhost:8765/tests/ui-harness/", flush=True)
    httpd.serve_forever()
