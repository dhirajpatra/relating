import io
import json
import os
import unittest
from unittest.mock import patch

from relating import (
    AuthenticationError,
    NotFoundError,
    RelatingClient,
    RelatingError,
    UpstreamError,
    ValidationError,
    default_target,
    parse_target,
)


class FakeResponse:
    def __init__(self, payload, status=200):
        body = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        self._body = io.BytesIO(body)
        self.status = status

    def read(self):
        return self._body.read()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


def http_error(url, code, payload):
    import urllib.error

    body = io.BytesIO(json.dumps(payload).encode())
    return urllib.error.HTTPError(url, code, f"HTTP {code}", {}, body)


class TargetTests(unittest.TestCase):
    def test_parse_target(self):
        self.assertIsNone(parse_target("relate://local"))
        self.assertEqual(parse_target("aura://cid-1"), "cid-1")
        for bad in ["", "bolt://x", "aura://", "RELATE://LOCAL"]:
            with self.assertRaises(ValueError, msg=bad):
                parse_target(bad)

    def test_default_target_reads_env(self):
        with patch.dict(os.environ, {"RELATING_TARGET": "aura://c9"}, clear=False):
            self.assertEqual(default_target(), "aura://c9")
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(default_target(), "relate://local")


class ClientTests(unittest.TestCase):
    def _client(self, **kwargs):
        kwargs.setdefault("token", "test-token")
        return RelatingClient(base_url="http://127.0.0.1:9/api/v1", **kwargs)

    def test_bearer_header_and_json_body(self):
        client = self._client()
        with patch("urllib.request.build_opener") as opener_mock:
            client2 = RelatingClient(base_url="http://127.0.0.1:9/api/v1", token="t")
            opener_mock.assert_called_once()
            self.assertIs(client2._opener, opener_mock.return_value)
        with patch.object(client, "_opener") as opener:
            opener.open.return_value = FakeResponse({"ok": True})
            out = client.create_project("demo", description="d")
            self.assertEqual(out, {"ok": True})
            request = opener.open.call_args[0][0]
            self.assertEqual(request.get_header("Authorization"), "Bearer test-token")
            self.assertEqual(request.get_header("Content-type"), "application/json")
            self.assertEqual(request.get_method(), "POST")
            body = json.loads(request.data.decode())
            self.assertEqual(body, {"name": "demo", "description": "d"})

    def test_target_flip_sends_connection_id(self):
        client = self._client()
        with patch.object(client, "_opener") as opener:
            opener.open.return_value = FakeResponse({"rows": []})
            client.cypher_read("pid", "MATCH (n) RETURN n", {"a": 1}, target="aura://c1")
            body = json.loads(opener.open.call_args[0][0].data.decode())
            self.assertEqual(body["connection_id"], "c1")
            self.assertEqual(body["params"], {"a": 1})

            opener.open.return_value = FakeResponse({"rows": []})
            client.cypher_read("pid", "MATCH (n) RETURN n", target="relate://local")
            body = json.loads(opener.open.call_args[0][0].data.decode())
            self.assertIsNone(body["connection_id"])

    def test_target_defaults_to_env(self):
        client = self._client()
        with patch.dict(os.environ, {"RELATING_TARGET": "aura://env-c"}):
            with patch.object(client, "_opener") as opener:
                opener.open.return_value = FakeResponse({"rows": []})
                client.cypher_read("pid", "MATCH (n) RETURN n")
                body = json.loads(opener.open.call_args[0][0].data.decode())
                self.assertEqual(body["connection_id"], "env-c")

    def test_token_defaults_to_env(self):
        with patch.dict(os.environ, {"RELATING_API_TOKEN": "env-token"}):
            client = RelatingClient(base_url="http://127.0.0.1:9/api/v1")
            self.assertEqual(client.token, "env-token")

    def test_delete_returns_none_on_empty_body(self):
        client = self._client()
        with patch.object(client, "_opener") as opener:
            opener.open.return_value = FakeResponse(b"")
            self.assertIsNone(client.delete_project("pid"))
            self.assertEqual(opener.open.call_args[0][0].get_method(), "DELETE")

    def test_error_mapping(self):
        client = self._client()
        cases = [
            (400, ValidationError), (401, AuthenticationError), (403, AuthenticationError),
            (404, NotFoundError), (429, RelatingError), (502, UpstreamError),
            (500, RelatingError),
        ]
        for status, exc_type in cases:
            with patch.object(client, "_opener") as opener:
                err = http_error("http://x", status,
                                 {"error": {"code": "SOME_CODE", "message": "boom"}})
                opener.open.side_effect = err
                with self.assertRaises(exc_type) as ctx:
                    client.list_projects()
                self.assertEqual(ctx.exception.status, status)
                self.assertEqual(ctx.exception.code, "SOME_CODE")
                self.assertIn("boom", str(ctx.exception))

    def test_query_params_and_put(self):
        client = self._client()
        with patch.object(client, "_opener") as opener:
            opener.open.return_value = FakeResponse({"ok": True})
            client.changes("pid", since="2026-01-01T00:00:00Z", limit=10)
            url = opener.open.call_args[0][0].full_url
            self.assertIn("since=2026-01-01T00%3A00%3A00Z", url)
            self.assertIn("limit=10", url)

            opener.open.return_value = FakeResponse({"ok": True})
            client.update_retrieval_config("pid", default_top_k=7)
            request = opener.open.call_args[0][0]
            self.assertEqual(request.get_method(), "PUT")

    def test_migration_flow_paths(self):
        client = self._client()
        with patch.object(client, "_opener") as opener:
            opener.open.return_value = FakeResponse({"ok": True})
            client.migration_sync("p", "m", propagate_deletes=True)
            url = opener.open.call_args[0][0].full_url
            self.assertTrue(url.endswith("/projects/p/migrations/m/sync?propagate_deletes=true"))

            opener.open.return_value = FakeResponse({"ok": True})
            client.migration_rollback("p", "m")
            url = opener.open.call_args[0][0].full_url
            self.assertTrue(url.endswith("/projects/p/migrations/m/rollback"))

    def test_remote_hosts_use_env_proxies(self):
        client = RelatingClient(base_url="https://api.example.com/api/v1", token="t")
        self.assertIsNone(client._opener)


if __name__ == "__main__":
    unittest.main()
