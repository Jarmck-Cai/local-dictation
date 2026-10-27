import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError

import pytest

from validation.providers import _post_json


@pytest.mark.parametrize("credential_header", ["x-api-key", "Authorization"])
def test_http_redirect_never_forwards_credentials_to_another_endpoint(credential_header):
    target_requests = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            self.rfile.read(int(self.headers.get("Content-Length", 0)))
            self.send_response(302)
            self.send_header("Location", "/unexpected-target")
            self.end_headers()

        def do_GET(self):
            target_requests.append(dict(self.headers))
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"unexpected": "redirect followed"}).encode())

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with pytest.raises(HTTPError) as caught:
            _post_json(f"http://127.0.0.1:{server.server_port}/original", {credential_header: "dummy-not-a-secret"},
                       {"messages": [{"role": "user", "content": "test text"}]}, 2)
        assert caught.value.code == 302
        assert target_requests == []
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
