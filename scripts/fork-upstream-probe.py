#!/usr/bin/env python3
import json
import re
import sys
from urllib.request import Request, ProxyHandler, HTTPRedirectHandler, build_opener


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def probe(client):
    branches = ("main", "dev")
    commits = []
    for branch in branches:
        request = Request("https://upstream.panda.invalid/git/ref/heads/" + branch,
                          headers={"User-Agent": "panda-upstream-watch",
                                   "Accept": "application/vnd.github+json"})
        with client.open(request, timeout=20) as response:
            document = json.load(response)
        expected = {"ref": "refs/heads/" + branch}
        identity = document["object"]
        if any(document.get(key) != value for key, value in expected.items()):
            raise ValueError("invalid ref")
        if identity.get("type") != "commit" or not re.fullmatch(r"[0-9a-f]{40}", identity["sha"]):
            raise ValueError("invalid ref")
        commits.append(branch + "=" + identity["sha"])
    return "UPSTREAM " + " ".join(commits)


def main():
    try:
        print(probe(build_opener(ProxyHandler({}), NoRedirect())))
    except Exception:
        print("Upstream metadata unavailable; no review dispatched.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
