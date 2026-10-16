# Secure Deployment Boundary

This is the normative secure-deployment procedure. The [secure deployment
guide](../guides/secure-deployment.md) is a short operator entry point and the
[security architecture](../security.md) records the product posture; neither
changes the requirements in this runbook.

## Supported exposure model

Panda is not approved for direct Internet exposure. The standard production
Compose stack publishes its web origin on `127.0.0.1:8080` by default.
`PANDA_HOST_BIND_ADDRESS` explicitly overrides the Docker host interface.
The stack runs the app as UID/GID `1001`, with a read-only root filesystem, all Linux
capabilities dropped, `no-new-privileges`, and a bounded `/tmp` tmpfs. Database
initialization and migrations run inside that same unprivileged app container;
the production stack has no root init service. Do not weaken these defaults to
make an origin public.

Place an authenticated tunnel or identity-aware reverse proxy and host firewall
rule in front of that origin. A tunnel that only publishes the port
without an access policy is not sufficient. A non-loopback API listener requires both
`PANDA_BIND_ADDRESS` and `ALLOW_PUBLIC_ORIGIN_BIND=true`. Docker host
publication is controlled separately by `PANDA_HOST_BIND_ADDRESS`; it remains unsupported as a direct
Internet exposure pattern.

The outer gateway must terminate public HTTPS, require an identity policy, and
forward normal HTTP and WebSocket traffic to `http://localhost:8080`. Panda
login remains enabled behind that gateway. Cloudflare Tunnel with Access and
Authentik in front of Caddy, Nginx, or Traefik are supported deployment shapes;
the gateway itself is operated and patched by the self-hoster.

## Required production configuration

- Panda defaults to production mode; use `PANDA_ENV=development` only for local development.
- Configure a 32-byte-or-longer production first-owner proof through exactly
  one of `PANDA_SETUP_TOKEN` or `PANDA_SETUP_TOKEN_FILE`. The setup
  endpoint reports availability but never reveals which source is used. Before
  a user exists, registration fails closed without a valid proof; after the
  first owner claims the instance, remove or rotate the bootstrap proof.
- Supply a complete valid external `JWT_SECRET`, `JWT_PUBLIC_KEY`, and
  `AGE_ENCRYPTION_KEY` bundle, using injected values or their `_FILE` alternatives.
  Production refuses DB-backed keys. Follow the [key-custody runbook](../runbooks/key-custody.md)
  before upgrading an existing database; preserve the original keys and ciphertexts.
  Keep an independently protected key backup; a paid secret manager is not required.
- Set `PANDA_IMAGE` to the reviewed digest of the image built from the tested source.
- Set `ALLOWED_ORIGINS` to the exact public HTTPS origin, with no path.
- Set strong `POSTGRES_PASSWORD` and `REDIS_PASSWORD` values. Standard Compose
  safely constructs its internal URLs; custom `DATABASE_URL` values must be valid URLs.
- Keep `COOKIE_INSECURE` absent. It is local-development-only.
- Do not publish API port 3001, PostgreSQL port 5432, Redis port 6379, or the
  origin port 8080 directly to the Internet.

### Exceptional trusted-LAN HTTP access

Do not use this procedure for public, guest, or untrusted Wi-Fi networks. If
HTTPS cannot be provided on an isolated trusted LAN, set all of these in the
Compose environment file:

```dotenv
PANDA_HOST_BIND_ADDRESS=192.168.1.20
PANDA_BIND_ADDRESS=0.0.0.0
ALLOW_PUBLIC_ORIGIN_BIND=true
ALLOWED_ORIGINS=http://192.168.1.20:8080
ALLOW_INSECURE_LAN_HTTP_AUTH=true
```

The final variable is a strict boolean and defaults to `false` in the standard
Compose file. Startup accepts only private, loopback, or link-local literal IP
HTTP origins; it rejects hostnames, public IPs, paths, credentials, and mixed
HTTP/HTTPS origin lists. This is an intentional reduction in transport
protection: refresh cookies remain HttpOnly, SameSite=Lax, rotating, and
revocable, but they no longer carry the `Secure` attribute. Restrict the port
to the trusted LAN with host firewall rules, disclose the interception risk to
users, and restore HTTPS as soon as possible.

## Gateway requirements

- Enforce authentication before forwarding any request, including `/v1/*` and
  WebSocket upgrades.
- Preserve `Host` and WebSocket upgrade headers. Do not log `Authorization` or
  `Sec-WebSocket-Protocol` headers.
- Preserve live-status control frames and configure the gateway's websocket
  idle/read timeout above 90 seconds. Panda sends a keepalive every 30
  seconds and the browser reconnects when it misses the 90-second liveness
  window.
- Own public TLS, certificate renewal, Internet-facing rate limits, and any
  trusted-client-IP policy. The internal Panda origin intentionally does
  not trust arbitrary forwarded client IP headers.
- Restrict direct host access to port 8080 with host firewall rules.

## Trusted gateway client addresses

By default nginx replaces forwarded-IP headers with the socket peer address.
If the gateway supplies sanitized client addresses, mount an operator-owned
configuration under `/etc/nginx/trusted-proxy/`. Trust only the gateway's exact
CIDRs, never every network:

```nginx
set_real_ip_from 192.168.1.10/32;
real_ip_header X-Forwarded-For;
real_ip_recursive on;
```

Mount that file read-only into the unified app container, for example at
`/etc/nginx/trusted-proxy/gateway.conf`. Verify spoofed forwarded chains cannot
change the address nginx passes to the loopback API. The API accepts one IP
from this trusted local nginx, never an arbitrary comma-separated chain.
Gateway-on-Docker-network deployments should remove the app host `ports`
publication and retain Traefik's service port 8080 on its private network.

## Verification

1. Run `docker compose --env-file .env -f compose/docker-compose.yml config`
   and confirm the unified app publishes the intentionally selected
   `PANDA_HOST_BIND_ADDRESS` and `PANDA_ORIGIN_PORT` values, with no
   database or Redis port.
2. Start the stack and check `curl http://localhost:8080/health` locally.
3. Confirm external access is denied by the gateway before reaching Panda,
   then authenticate through the gateway and sign in to Panda.
4. Confirm `docker compose --env-file .env -f compose/docker-compose.yml ps` shows no host
   mapping for the internal API listener, TimescaleDB, or Redis.
5. Run `pnpm docs:check` and the security test suite before upgrading a shared
   instance.
6. From the public address, leave a signed-in dashboard open beyond the
   gateway idle window, background and refocus the tab, and confirm the status
   transitions through `Reconnecting...` to `Online` without a page reload.
7. Confirm `docker compose ... config` retains `user: "1001:1001"`,
   `read_only: true`, `cap_drop: [ALL]`, and `no-new-privileges:true` for the
   long-lived `panda` service. These are deployment controls, not optional
   tuning.

## Limits of this guidance

This boundary reduces exposure; it is not a security certification or a
substitute for gateway patching, host hardening, backups, monitoring, and an
independent penetration test when the deployment risk warrants one.
