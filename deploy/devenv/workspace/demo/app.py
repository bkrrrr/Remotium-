"""Tiny HTTP server for trying terminals, port forwarding and the debugger."""
from http.server import BaseHTTPRequestHandler, HTTPServer


def greeting(path: str) -> str:
    name = path.strip("/") or "world"
    return f"hello, {name}\n"  # set a breakpoint here


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = greeting(self.path).encode()
        self.send_response(200)
        self.send_header("content-type", "text/plain")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    print("listening on http://127.0.0.1:8000")
    HTTPServer(("127.0.0.1", 8000), Handler).serve_forever()
