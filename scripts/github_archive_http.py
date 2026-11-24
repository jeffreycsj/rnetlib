"""Small bounded GitHub.com REST client used only by the trusted archive workflow.

Credentials are sent only to fixed GitHub API/upload hosts. Signed artifact
redirects are data-only GETs with no Authorization header. No write is retried.
"""
import http.client
import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request

from archive_io import MAX_MEMBER, regular_file

MAX_JSON = 2 * 1024**2


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def download_host(url):
    p = urllib.parse.urlsplit(url)
    if (p.scheme != "https" or p.username is not None or p.password is not None or p.fragment
            or p.port not in (None, 443) or not p.hostname):
        raise ValueError("unsafe artifact redirect")
    host = p.hostname
    if host != "api.github.com" and not (
            host.endswith(".githubusercontent.com") or host.endswith(".blob.core.windows.net")):
        raise ValueError("artifact redirected outside approved GitHub storage hosts")
    return host


class GitHub:
    def __init__(self, token=None, *, allow_write=False):
        self.token = token or os.environ.get("GH_TOKEN", "")
        if not self.token or any(c in self.token for c in "\r\n"):
            raise ValueError("GH_TOKEN is required")
        self.allow_write = allow_write
        self.opener = urllib.request.build_opener(NoRedirect())

    def _headers(self, *, authenticated=True, binary=False):
        h = {"Accept": "application/octet-stream" if binary else "application/vnd.github+json",
             "X-GitHub-Api-Version": "2026-03-10", "User-Agent": "rnet-release-archive"}
        if authenticated:
            h["Authorization"] = "Bearer " + self.token
        return h

    def _json(self, route, method="GET", body=None):
        if not route.startswith("/repos/") or any(c in route for c in "\r\n#"):
            raise ValueError("invalid GitHub API route")
        if method != "GET" and (not self.allow_write or method not in ("POST", "PATCH")
                or not re.fullmatch(r"/repos/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/releases(?:/[1-9][0-9]*)?", route)):
            raise ValueError("GitHub write is not enabled or is outside Release scope")
        headers = self._headers()
        data = None if body is None else json.dumps(body).encode()
        if data is not None:
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request("https://api.github.com" + route, data=data, headers=headers, method=method)
        try:
            with self.opener.open(request, timeout=60) as response:
                raw = response.read(MAX_JSON + 1)
                if len(raw) > MAX_JSON:
                    raise ValueError("GitHub JSON size limit exceeded")
                return json.loads(raw)
        except (urllib.error.URLError, http.client.HTTPException, TimeoutError) as error:
            # Do not echo signed URLs, response bodies, tokens or proxy credentials.
            raise ValueError("GitHub API request failed; no automatic write retry") from None

    def get(self, route):
        return self._json(route)

    def write(self, method, route, body):
        return self._json(route, method, body)

    def pages(self, route, key=None):
        result = []
        for page in range(1, 11):
            separator = "&" if "?" in route else "?"
            data = self.get(f"{route}{separator}per_page=100&page={page}")
            items = data.get(key) if key else data
            if not isinstance(items, list) or len(items) > 100:
                raise ValueError("invalid paginated GitHub response")
            result.extend(items)
            if len(items) < 100:
                return result
        raise ValueError("GitHub pagination limit exceeded")

    def download(self, route, output, maximum):
        if not re.fullmatch(r"/repos/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/actions/artifacts/[1-9][0-9]*/zip", route):
            raise ValueError("invalid artifact download route")
        url = "https://api.github.com" + route
        for _ in range(5):
            host = download_host(url)
            request = urllib.request.Request(url, headers=self._headers(authenticated=host == "api.github.com", binary=True))
            try:
                response = self.opener.open(request, timeout=60)
            except urllib.error.HTTPError as error:
                if error.code not in (301, 302, 303, 307, 308):
                    raise ValueError("GitHub artifact download failed") from None
                location = error.headers.get("Location", "")
                error.close()
                url = urllib.parse.urljoin(url, location)
                continue
            except (urllib.error.URLError, TimeoutError, http.client.HTTPException):
                raise ValueError("GitHub artifact download failed") from None
            with response, output.open("xb") as target:
                count = 0
                while True:
                    block = response.read(65536)
                    if not block:
                        return
                    count += len(block)
                    if count > maximum:
                        raise ValueError("artifact download size limit exceeded")
                    target.write(block)
        raise ValueError("artifact redirect limit exceeded")

    def upload(self, repository, release_id, path):
        path = regular_file(path, MAX_MEMBER)
        if (not self.allow_write or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository)
                or type(release_id) is not int or release_id <= 0):
            raise ValueError("Release upload not authorized")
        headers = self._headers()
        headers.update({"Content-Type": "application/octet-stream", "Content-Length": str(path.stat().st_size)})
        route = f"/repos/{repository}/releases/{release_id}/assets?" + urllib.parse.urlencode({"name": path.name})
        connection = http.client.HTTPSConnection("uploads.github.com", timeout=60)
        try:
            connection.putrequest("POST", route)
            for key, value in headers.items():
                connection.putheader(key, value)
            connection.endheaders()
            with path.open("rb") as stream:
                for block in iter(lambda: stream.read(65536), b""):
                    connection.send(block)
            response = connection.getresponse()
            raw = response.read(MAX_JSON + 1)
            if response.status != 201 or len(raw) > MAX_JSON:
                raise ValueError("asset upload failed; draft preserved, no automatic retry")
        except (OSError, http.client.HTTPException):
            raise ValueError("asset upload interrupted; inspect draft before retry") from None
        finally:
            connection.close()
