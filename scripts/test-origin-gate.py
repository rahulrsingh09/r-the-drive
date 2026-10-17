#!/usr/bin/env python3
"""Exercise the production nginx access boundary with a synthetic backend."""
import contextlib
import http.client
import http.server
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import threading
import time


class Backend(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        fields = dict(authorization=self.headers.get("Authorization"),
                      originHeaderPresent="X-Panda-Edge" in self.headers)
        payload = json.dumps(fields).encode("utf-8")
        self.send_response(200)
        for name, value in [("Content-Type", "application/json"),
                            ("Content-Length", str(len(payload)))]:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(payload)

    do_POST = do_GET

    def log_message(self, *_):
        return None


def http_probe(port, path, headers=None, method="GET"):
    with contextlib.closing(http.client.HTTPConnection("127.0.0.1", port, timeout=5)) as connection:
        connection.request(method, path, headers=headers or {})
        response = connection.getresponse()
        return response.status, response.read()


def configure(root, repo, port, backend_port, token):
    site = root / "site"
    (site / "assets").mkdir(parents=True)
    for file, content in [("index.html", "synthetic dashboard"), ("assets/probe.js", "/* synthetic asset */")]:
        (site / file).write_text(content)
    gate = root / "origin.conf"
    env = dict(os.environ, PANDA_GATEWAY_TOKEN=token, PANDA_REQUIRE_GATEWAY="true")
    subprocess.run(["sh", str(repo / "compose/render-origin-gate.sh"), str(gate)], env=env, check=True)
    replacements = {
        "include       mime.types;": "include       /etc/nginx/mime.types;",
        "/tmp/panda-origin-gate.conf": str(gate),
        "/etc/nginx/security-headers.conf": str(repo / "compose/nginx/security-headers.conf"),
        "/tmp/nginx.pid": str(root / "nginx.pid"),
        "listen 8080;": "listen 127.0.0.1:{};".format(port),
        "127.0.0.1:3001": "127.0.0.1:{}".format(backend_port),
        "127.0.0.1:3002": "127.0.0.1:{}".format(backend_port),
        "/app/dist": str(site),
    }
    for name in ["client_temp", "proxy_temp", "fastcgi_temp", "uwsgi_temp", "scgi_temp"]:
        replacements["/tmp/" + name] = str(root / name)
    source = (repo / "compose/nginx/nginx.conf").read_text()
    for before, after in replacements.items():
        source = source.replace(before, after)
    file = root / "nginx.conf"
    file.write_text(source)
    return ["nginx", "-c", str(file), "-g", "daemon off;"]


@contextlib.contextmanager
def running_gateway():
    repo = Path(__file__).resolve().parent.parent
    token = "AbC_" + secrets.token_urlsafe(36)
    backend = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Backend)
    worker = threading.Thread(target=backend.serve_forever, daemon=True)
    worker.start()
    container = None
    process = None
    try:
        with tempfile.TemporaryDirectory(prefix="panda-origin-test-") as directory:
            root = Path(directory)
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                port = reservation.getsockname()[1]
            command = configure(root, repo, port, backend.server_port, token)
            image = os.environ.get("PANDA_NGINX_TEST_IMAGE")
            if image:
                subprocess.run(["docker", "image", "inspect", image], check=True, stdout=subprocess.DEVNULL)
                container = "panda-origin-test-" + secrets.token_hex(6)
                command = ["docker", "run", "--pull=never", "--rm", "--name", container,
                           "--label", "panda.validation=origin-gate", "--network", "host",
                           "--user", "{}:{}".format(os.getuid(), os.getgid()), "--cap-drop", "ALL",
                           "--security-opt", "no-new-privileges:true", "-v", "{}:{}:ro".format(repo, repo),
                           "-v", "{}:{}".format(root, root), "--entrypoint", "nginx", image] + command[1:]
            try:
                process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                def request(path, headers=None, method="GET"):
                    return http_probe(port, path, headers, method)
                deadline = time.monotonic() + 10
                ready = False
                while not ready:
                    if process.poll() is not None:
                        raise RuntimeError("Test nginx exited before becoming ready")
                    try:
                        ready = request("/health")[0] == 200
                    except OSError:
                        pass
                    if ready:
                        break
                    if time.monotonic() >= deadline:
                        raise RuntimeError("Test nginx did not become ready")
                    time.sleep(0.1)
                yield request, token
            finally:
                if container:
                    subprocess.run(["docker", "stop", "--time", "2", container], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                elif process and process.poll() is None:
                    process.terminate()
                if process:
                    stdout, stderr = process.communicate(timeout=10)
                    assert token.encode() not in stdout + stderr
                    assert b"synthetic-token" not in stdout + stderr
    finally:
        backend.shutdown()
        backend.server_close()
        worker.join(timeout=5)


def main():
    with running_gateway() as (request, token):
        paths = ["/", "/index.html", "/assets/probe.js", "/v1/vehicles",
                 "/v2/themes", "/v1/admin/backups/imports",
                 "/v1/restore-runtime/jobs/probe", "/v1/vehicles/live",
                 "/health/", "/health/../v1/vehicles",
                 "/health%2f..%2fv1/vehicles", "/HEALTH"]
        for path in paths:
            assert request(path)[0] == 403, path
            assert request(path, {"X-Panda-Edge": token.swapcase()})[0] == 403, path
        for method in ["POST", "HEAD", "OPTIONS"]:
            assert request("/v1/vehicles", method=method)[0] == 403
        assert request("/", {"Authorization": "Bearer synthetic-token"})[0] == 403
        assert request("/", {"X-Panda-Edge": token})[0] == 200
        assert request("/assets/probe.js", {"X-Panda-Edge": token})[0] == 200
        for path in ["/v1/vehicles", "/v2/themes", "/v1/restore-runtime/jobs/probe"]:
            status, body = request(path, {"X-Panda-Edge": token, "Authorization": "Bearer synthetic-token"})
            assert status == 200, path
            assert json.loads(body)["authorization"] == "Bearer synthetic-token"
            assert json.loads(body)["originHeaderPresent"] is False
        assert request("/health?target=/v1/vehicles")[0] == 200
        print("Origin gate passed: protected routes, methods, path normalization, case-sensitive token and preserved application authorization.")


if __name__ == "__main__":
    main()
