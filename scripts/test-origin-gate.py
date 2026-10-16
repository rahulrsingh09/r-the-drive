#!/usr/bin/env python3
"""Exercise the production nginx access boundary with a synthetic backend."""

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

repo = Path(__file__).resolve().parent.parent
token = "AbC_" + secrets.token_urlsafe(36)


class Backend(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({
            "authorization": self.headers.get("Authorization"),
            "originHeaderPresent": self.headers.get("X-Panda-Edge") is not None,
        }).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_POST = do_GET

    def log_message(self, *_):
        pass


backend = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Backend)
threading.Thread(target=backend.serve_forever, daemon=True).start()
with socket.socket() as port_socket:
    port_socket.bind(("127.0.0.1", 0))
    port = port_socket.getsockname()[1]

container = None
process = None
with tempfile.TemporaryDirectory(prefix="panda-origin-test-") as directory:
    root = Path(directory)
    gate = root / "origin.conf"
    env = {**os.environ, "PANDA_GATEWAY_TOKEN": token, "PANDA_REQUIRE_GATEWAY": "true"}
    subprocess.run(["sh", str(repo / "compose/render-origin-gate.sh"), str(gate)], env=env, check=True)
    site = root / "site"
    (site / "assets").mkdir(parents=True)
    (site / "index.html").write_text("synthetic dashboard")
    (site / "assets/probe.js").write_text("/* synthetic asset */")
    config = (repo / "compose/nginx/nginx.conf").read_text()
    config = config.replace("include       mime.types;", "include       /etc/nginx/mime.types;")
    config = config.replace("/tmp/panda-origin-gate.conf", str(gate))
    config = config.replace("/etc/nginx/security-headers.conf", str(repo / "compose/nginx/security-headers.conf"))
    config = config.replace("/tmp/nginx.pid", str(root / "nginx.pid"))
    for name in ["client_temp", "proxy_temp", "fastcgi_temp", "uwsgi_temp", "scgi_temp"]:
        config = config.replace("/tmp/" + name, str(root / name))
    config = config.replace("listen 8080;", f"listen 127.0.0.1:{port};")
    config = config.replace("127.0.0.1:3001", f"127.0.0.1:{backend.server_port}")
    config = config.replace("127.0.0.1:3002", f"127.0.0.1:{backend.server_port}")
    config = config.replace("/app/dist", str(site))
    config_file = root / "nginx.conf"
    config_file.write_text(config)
    command = ["nginx", "-c", str(config_file), "-g", "daemon off;"]
    image = os.environ.get("PANDA_NGINX_TEST_IMAGE")
    if image:
        container = "panda-origin-test-" + secrets.token_hex(6)
        command = [
            "docker", "run", "--rm", "--name", container, "--network", "host",
            "--user", f"{os.getuid()}:{os.getgid()}", "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges:true",
            "-v", f"{repo}:{repo}:ro", "-v", f"{root}:{root}",
            "--entrypoint", "nginx", image, *command[1:],
        ]

    def request(path, headers=None, method="GET"):
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        try:
            connection.request(method, path, headers=headers or {})
            response = connection.getresponse()
            return response.status, response.read()
        finally:
            connection.close()

    try:
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        for _ in range(100):
            if process.poll() is not None:
                raise RuntimeError("Test nginx exited before becoming ready")
            try:
                if request("/health")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.1)
        else:
            raise RuntimeError("Test nginx did not become ready")

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
    finally:
        if container:
            subprocess.run(["docker", "stop", "--time", "2", container], capture_output=True)
        elif process:
            process.terminate()
        if process:
            stdout, stderr = process.communicate(timeout=10)
            assert token.encode() not in stdout + stderr
            assert b"synthetic-token" not in stdout + stderr
        backend.shutdown()
