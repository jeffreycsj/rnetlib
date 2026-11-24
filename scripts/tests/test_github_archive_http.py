"""Exercise actual HTTP request construction with only network I/O replaced."""
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock
import urllib.error

from archive_fixture import fixture
from github_archive_http import GitHub, download_host


class Response(io.BytesIO):
    pass


class HttpTests(unittest.TestCase):
    def test_redirect_does_not_forward_token_to_storage(self):
        api = GitHub("test-token")
        api.opener = Mock()
        api.opener.open.side_effect = [urllib.error.HTTPError(
            "https://api.github.com/test", 302, "redirect", {"Location": "https://example.blob.core.windows.net/file?signature=private"}, None),
            Response(b"bytes")]
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "zip"
            api.download("/repos/owner/repo/actions/artifacts/1/zip", path, 5)
            self.assertEqual(path.read_bytes(), b"bytes")
        first, second = [c.args[0] for c in api.opener.open.call_args_list]
        self.assertEqual(first.get_header("Authorization"), "Bearer test-token")
        self.assertIsNone(second.get_header("Authorization"))

    def test_unsafe_redirects_fail_before_network_contact(self):
        for url in ("http://example.blob.core.windows.net/a", "https://githubusercontent.com.evil/a",
                    "https://user:pass@api.github.com/a", "https://@api.github.com/a", "https://api.github.com:444/a",
                    "https://api.github.com/a#fragment", "https://evil/a", "file:///tmp/a"):
            with self.subTest(url=url), self.assertRaises(ValueError): download_host(url)

    def test_read_only_client_cannot_write_or_delete(self):
        api = GitHub("test-token"); api.opener = Mock()
        for method in ("POST", "PATCH", "DELETE"):
            with self.assertRaises(ValueError): api.write(method, "/repos/owner/repo/releases/1", {})
        api.allow_write = True
        with self.assertRaises(ValueError): api.write("POST", "/repos/owner/repo/git/refs", {})
        api.opener.open.assert_not_called()

    def test_download_and_pagination_are_bounded(self):
        api = GitHub("test-token"); api.opener = Mock()
        api.opener.open.return_value = Response(b"too many bytes")
        with tempfile.TemporaryDirectory() as d, self.assertRaises(ValueError):
            api.download("/repos/owner/repo/actions/artifacts/1/zip", Path(d) / "zip", 3)
        api.get = Mock(return_value=[{}] * 100)
        with self.assertRaises(ValueError): api.pages("/repos/owner/repo/releases")
        self.assertEqual(api.get.call_count, 10)

    def test_failures_do_not_print_credentials_or_signed_urls(self):
        api = GitHub("test-token"); api.opener = Mock()
        api.opener.open.side_effect = urllib.error.URLError("SECRET-SIGNED-URL")
        with self.assertRaises(ValueError) as failure: api.get("/repos/owner/repo")
        self.assertNotIn("SECRET", str(failure.exception))
        self.assertNotIn("test-token", str(failure.exception))
