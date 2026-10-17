"""TLS provider fixture; run only on an unpublished, internal Docker network."""
import base64
import http.server
import json
import os
import ssl
import subprocess
import time
import urllib.parse

ORIGIN = "https://identity-provider:8443"
KEY = "/evidence/tls/key.pem"
CERT = "/evidence/tls/cert.pem"


def encode(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode()


modulus = subprocess.check_output(["openssl", "rsa", "-in", KEY, "-noout", "-modulus"], stderr=subprocess.DEVNULL).decode().strip().split("=", 1)[1]
JWK = {"kty": "RSA", "kid": "synthetic", "use": "sig", "alg": "RS256", "n": encode(bytes.fromhex(modulus)), "e": "AQAB"}


class Provider(http.server.BaseHTTPRequestHandler):
    def reply(self, status, value):
        payload = value if isinstance(value, bytes) else json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        variant = self.path.split("/")[1]
        if self.path.endswith("/jwks"):
            return self.reply(200, {"keys": [JWK]})
        if self.path.endswith("/userinfo"):
            return self.reply(200, {"sub": "synthetic-subject", "email": "Artist@Example.invalid", "email_verified": True, "groups": ["artists"], "tenant": "fallback"})
        if variant == "bad-json":
            return self.reply(200, b"{broken")
        if variant == "oversize":
            return self.reply(200, b" " * 3000000)
        if variant == "unavailable":
            return self.reply(503, {"error": "fixture unavailable"})
        issuer = ORIGIN + "/" + variant
        self.reply(200, {"issuer": issuer if variant != "mismatch" else ORIGIN + "/other",
                        "authorization_endpoint": issuer + "/authorize",
                        "token_endpoint": (issuer + "/token") if variant != "http-token" else "http://identity-provider/token",
                        "jwks_uri": issuer + "/jwks", "userinfo_endpoint": issuer + "/userinfo",
                        "response_types_supported": ["code"], "subject_types_supported": ["public"],
                        "id_token_signing_alg_values_supported": ["RS256"],
                        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
                        "code_challenge_methods_supported": ["plain"] if variant == "no-pkce" else ["S256"]})

    def do_POST(self):
        params = urllib.parse.parse_qs(self.rfile.read(int(self.headers.get("Content-Length", 0))).decode())
        code = params.get("code", [""])[0]
        if code == "reject-code":
            return self.reply(400, {"error": "invalid_grant"})
        claims = {"iss": ORIGIN + "/good", "sub": "synthetic-subject", "aud": "fixture-client", "iat": int(time.time()), "exp": int(time.time()) + 120, "nonce": "nonce", "tenant": "primary"}
        if code == "wrong-nonce":
            claims["nonce"] = "other"
        if code == "wrong-audience":
            claims["aud"] = "other"
        if code == "wrong-issuer":
            claims["iss"] = ORIGIN + "/other"
        if code == "expired":
            claims["exp"] = int(time.time()) - 120
        content = encode(json.dumps({"alg": "RS256", "kid": "synthetic"}).encode()) + "." + encode(json.dumps(claims).encode())
        signed = subprocess.run(["openssl", "dgst", "-sha256", "-sign", KEY], input=content.encode(), stdout=subprocess.PIPE, check=True).stdout
        if code == "bad-signature":
            signed = bytes([signed[0] ^ 1]) + signed[1:]
        self.reply(200, {"access_token": "fixture-access", "token_type": "Bearer", "expires_in": 120, "id_token": content + "." + encode(signed)})

    def log_message(self, fmt, *args):
        print(fmt % args, flush=True)


server = http.server.ThreadingHTTPServer((os.environ["FIXTURE_BIND_IP"], 8443), Provider)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(CERT, KEY)
server.socket = context.wrap_socket(server.socket, server_side=True)
server.serve_forever()
