#!/usr/bin/env python3
import json
import re
import sys
from urllib.request import Request, ProxyHandler, HTTPRedirectHandler, build_opener

class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None

try:
    client = build_opener(ProxyHandler({}), NoRedirect())
    refs = {}
    for branch in ("main", "dev"):
        request = Request("https://upstream.panda.invalid/git/ref/heads/" + branch,
            headers={"User-Agent": "panda-upstream-watch", "Accept": "application/vnd.github+json"})
        with client.open(request, timeout=20) as response:
            value = json.load(response)
        sha = value["object"]["sha"]
        if value["ref"] != "refs/heads/" + branch or value["object"]["type"] != "commit" or not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise ValueError("invalid ref")
        refs[branch] = sha
    print("UPSTREAM main=" + refs["main"] + " dev=" + refs["dev"])
except Exception:
    print("Upstream metadata unavailable; no review dispatched.", file=sys.stderr)
    raise SystemExit(1)
