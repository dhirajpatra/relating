"""Synchronous Relating client over the JSON API (stdlib only).

.. code-block:: python

    from relating import RelatingClient

    client = RelatingClient()  # token from RELATING_API_TOKEN
    result = client.cypher_read(project_id, "MATCH (n:Person) RETURN n LIMIT 5")
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

from .errors import RelatingError, error_from_response
from .targets import TARGET_ENV_VAR, default_target, parse_target

DEFAULT_BASE_URL = "http://127.0.0.1:8080/api/v1"
TOKEN_ENV_VAR = "RELATING_API_TOKEN"
_LOOPBACK_HOSTS = {"localhost", "127.0.0.1", "::1"}


def _is_loopback_url(url: str) -> bool:
    try:
        return (urllib.parse.urlsplit(url).hostname or "").lower() in _LOOPBACK_HOSTS
    except ValueError:
        return False


class RelatingClient:
    """Thin wrapper over ``/api/v1``. Responses are plain decoded JSON."""

    def __init__(self, base_url: str = DEFAULT_BASE_URL, token: str | None = None,
                 timeout_secs: float = 60.0):
        self.base_url = base_url.rstrip("/")
        self.token = token if token is not None else os.environ.get(TOKEN_ENV_VAR)
        self.timeout_secs = timeout_secs
        # Loopback always goes direct: a sandbox/corporate ALL_PROXY must
        # never route local traffic through the gateway. Remote hosts honor
        # the process proxy environment.
        if _is_loopback_url(self.base_url):
            self._opener = urllib.request.build_opener(
                urllib.request.HTTPHandler, urllib.request.HTTPSHandler
            )
        else:
            self._opener = None

    def _request(self, method: str, path: str, body: Any = None,
                 query: dict[str, Any] | None = None) -> Any:
        url = f"{self.base_url}{path}"
        if query:
            cleaned = {k: v for k, v in query.items() if v is not None}
            if cleaned:
                url += "?" + urllib.parse.urlencode(cleaned, doseq=True)
        data = None
        headers = {}
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            if self._opener is not None:
                response = self._opener.open(request, timeout=self.timeout_secs)
            else:
                response = urllib.request.urlopen(request, timeout=self.timeout_secs)
        except urllib.error.HTTPError as exc:
            try:
                payload = json.loads(exc.read().decode("utf-8") or "null")
            except (ValueError, UnicodeDecodeError):
                payload = None
            raise error_from_response(exc.code, payload) from exc
        except urllib.error.URLError as exc:
            raise RelatingError(f"Cannot reach Relating at {self.base_url}: {exc.reason}") from exc
        with response:
            raw = response.read().decode("utf-8")
        if not raw.strip():
            return None
        try:
            return json.loads(raw)
        except ValueError as exc:
            raise RelatingError(f"Invalid JSON response from {url}") from exc

    # -- health -----------------------------------------------------------
    def health(self) -> Any:
        return self._request("GET", "/health")

    def mcp_health(self) -> Any:
        base = self.base_url.rsplit("/api/v1", 1)[0]
        url = f"{base}/mcp/health"
        try:
            if self._opener is not None:
                response = self._opener.open(url, timeout=self.timeout_secs)
            else:
                response = urllib.request.urlopen(url, timeout=self.timeout_secs)
        except urllib.error.URLError as exc:
            raise RelatingError(f"MCP health check failed: {exc.reason}") from exc
        with response:
            return json.loads(response.read().decode("utf-8"))

    # -- projects ---------------------------------------------------------
    def list_projects(self) -> Any:
        return self._request("GET", "/projects")

    def get_project(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}")

    def create_project(self, name: str, description: str | None = None,
                       ai_config: dict[str, Any] | None = None) -> Any:
        body: dict[str, Any] = {"name": name}
        if description is not None:
            body["description"] = description
        if ai_config is not None:
            body["ai_config"] = ai_config
        return self._request("POST", "/projects", body)

    def delete_project(self, project_id: str) -> None:
        self._request("DELETE", f"/projects/{project_id}")

    def test_ai(self, project_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/ai/test")

    # -- cypher + backend flip -------------------------------------------
    def cypher_read(self, project_id: str, query: str, params: dict[str, Any] | None = None,
                    limit: int | None = None, target: str | None = None) -> Any:
        """Run a read on ``target`` (default ``RELATING_TARGET``).

        Only the target changes at cutover: ``relate://local`` runs on the
        project store, ``aura://<connection-id>`` proxies the identical
        statement to Aura.
        """
        connection_id = parse_target(target if target is not None else default_target())
        return self._request("POST", f"/projects/{project_id}/cypher/read", {
            "query": query,
            "params": params or {},
            "limit": limit,
            "connection_id": connection_id,
        })

    def cypher_write(self, project_id: str, query: str, params: dict[str, Any] | None = None,
                     target: str | None = None, confirm_remote_write: bool = False) -> Any:
        connection_id = parse_target(target if target is not None else default_target())
        body: dict[str, Any] = {"query": query, "params": params or {},
                                "connection_id": connection_id}
        # The server takes a bare bool here (not nullable): only send it
        # when affirming a remote write.
        if confirm_remote_write:
            body["confirm_remote_write"] = True
        return self._request("POST", f"/projects/{project_id}/cypher/write", body)

    def query_targets(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/query-targets")

    # -- sources ----------------------------------------------------------
    def list_sources(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/sources")

    def register_source(self, project_id: str, filename: str, content: str = "",
                        content_type: str | None = None,
                        content_base64: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/sources", {
            "filename": filename,
            "content": content,
            "content_base64": content_base64,
            "content_type": content_type,
        })

    def delete_source(self, project_id: str, source_id: str) -> None:
        self._request("DELETE", f"/projects/{project_id}/sources/{source_id}")

    # -- ontology + ingest -----------------------------------------------
    def get_ontology(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/ontology")

    def propose_ontology(self, project_id: str, domain_hint: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/ontology/propose",
                             {"domain_hint": domain_hint})

    def import_ontology(self, project_id: str, schema: dict[str, Any]) -> Any:
        return self._request("POST", f"/projects/{project_id}/ontology/import",
                             {"schema": schema})

    def approve_ontology(self, project_id: str, ontology_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/ontology/approve",
                             {"ontology_id": ontology_id})

    def ingest(self, project_id: str, full: bool = False) -> Any:
        return self._request("POST", f"/projects/{project_id}/ingest",
                             query={"full": "true" if full else None} if full else None)

    def graph_elements(self, project_id: str, limit: int | None = None) -> Any:
        return self._request("GET", f"/projects/{project_id}/graph/elements",
                             query={"limit": limit})

    # -- GraphRAG ---------------------------------------------------------
    def embeddings_status(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/embeddings")

    def generate_embeddings(self, project_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/embeddings", {})

    def retrieve(self, project_id: str, query: str, strategy: str | None = None,
                 top_k: int | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/retrieval",
                             {"query": query, "strategy": strategy, "top_k": top_k})

    def ask(self, project_id: str, question: str, strategy: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/questions",
                             {"question": question, "strategy": strategy})

    def list_evaluations(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/evaluations")

    def create_evaluation(self, project_id: str, questions: list[str],
                          strategy: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/evaluations",
                             {"questions": questions, "strategy": strategy})

    def get_evaluation(self, project_id: str, evaluation_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/evaluations/{evaluation_id}")

    # -- Aura connections + migration ------------------------------------
    def list_aura_connections(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/connections/aura")

    def create_aura_connection(self, project_id: str, name: str, uri: str,
                               username: str | None = None, database: str | None = None,
                               auth_env_var: str | None = None) -> Any:
        body: dict[str, Any] = {"name": name, "uri": uri}
        if username is not None:
            body["username"] = username
        if database is not None:
            body["database"] = database
        if auth_env_var is not None:
            body["auth_env_var"] = auth_env_var
        return self._request("POST", f"/projects/{project_id}/connections/aura", body)

    def delete_aura_connection(self, project_id: str, connection_id: str) -> None:
        self._request("DELETE", f"/projects/{project_id}/connections/aura/{connection_id}")

    def test_aura_connection(self, project_id: str, connection_id: str) -> Any:
        return self._request(
            "POST", f"/projects/{project_id}/connections/aura/{connection_id}/test")

    def migration_analyze(self, project_id: str, connection_id: str | None = None,
                          aura_tier: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/migrations/analyze",
                             {"connection_id": connection_id, "aura_tier": aura_tier})

    def migration_plan(self, project_id: str, connection_id: str,
                       batch_size: int | None = None, aura_tier: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/migrations/plan",
                             {"connection_id": connection_id, "batch_size": batch_size,
                              "aura_tier": aura_tier})

    def list_migrations(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/migrations")

    def get_migration(self, project_id: str, migration_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/migrations/{migration_id}")

    def migration_dry_run(self, project_id: str, migration_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/migrations/{migration_id}/dry-run")

    def migration_approve(self, project_id: str, migration_id: str, approved: bool) -> Any:
        return self._request("POST", f"/projects/{project_id}/migrations/{migration_id}/approve",
                             {"approved": approved})

    def migration_execute(self, project_id: str, migration_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/migrations/{migration_id}/execute")

    def migration_verify(self, project_id: str, migration_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/migrations/{migration_id}/verify")

    def migration_rollback(self, project_id: str, migration_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/migrations/{migration_id}/rollback")

    def migration_report(self, project_id: str, migration_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/migrations/{migration_id}/report")

    def migration_sync(self, project_id: str, migration_id: str,
                       propagate_deletes: bool = False) -> Any:
        query = {"propagate_deletes": "true"} if propagate_deletes else None
        return self._request(
            "POST", f"/projects/{project_id}/migrations/{migration_id}/sync", query=query)

    def migration_drift(self, project_id: str, migration_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/migrations/{migration_id}/drift")

    def changes(self, project_id: str, since: str | None = None,
                limit: int | None = None) -> Any:
        return self._request("GET", f"/projects/{project_id}/changes",
                             query={"since": since, "limit": limit})

    # -- schema + retrieval config ---------------------------------------
    def schema_export(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/schema/export")

    def list_schema_revisions(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/schema/revisions")

    def create_schema_revision(self, project_id: str, document: Any = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/schema/revisions",
                             {"document": document})

    def get_schema_revision(self, project_id: str, version: int) -> Any:
        return self._request("GET", f"/projects/{project_id}/schema/revisions/{version}")

    def schema_diff(self, project_id: str, from_version: int | None = None,
                    to_version: int | None = None, to_document: Any = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/schema/diff",
                             {"from_version": from_version, "to_version": to_version,
                              "to_document": to_document})

    def schema_apply(self, project_id: str, document: Any) -> Any:
        return self._request("POST", f"/projects/{project_id}/schema/apply",
                             {"document": document})

    def get_retrieval_config(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/retrieval-config")

    def update_retrieval_config(self, project_id: str, default_strategy: str | None = None,
                                default_top_k: int | None = None) -> Any:
        return self._request("PUT", f"/projects/{project_id}/retrieval-config",
                             {"default_strategy": default_strategy,
                              "default_top_k": default_top_k})

    # -- portability ------------------------------------------------------
    def portability_scan(self, project_id: str, repo_path: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/portability/scan",
                             {"repo_path": repo_path})

    def portability_run_tests(self, project_id: str, connection_id: str,
                              tests: list[dict[str, Any]]) -> Any:
        return self._request("POST", f"/projects/{project_id}/portability/run-tests",
                             {"connection_id": connection_id, "tests": tests})

    # -- shadow + debug ---------------------------------------------------
    def shadow_run(self, project_id: str, connection_id: str,
                   queries: list[dict[str, Any]], redact_params: bool | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/shadow/runs",
                             {"connection_id": connection_id, "queries": queries,
                              "redact_params": redact_params})

    def debug_trace(self, project_id: str, question: str, strategy: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/debug/trace",
                             {"question": question, "strategy": strategy})

    # -- synth + advisor --------------------------------------------------
    def synth_profile(self, project_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/synth/profile")

    def synth_generate(self, project_id: str, scale: int | None = None,
                       seed: int | None = None, edge_cases: list[dict[str, Any]] | None = None,
                       materialize: bool | None = None,
                       target_name: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/synth/generate",
                             {"scale": scale, "seed": seed, "edge_cases": edge_cases,
                              "materialize": materialize, "target_name": target_name})

    def advisor_analyze(self, project_id: str, queries: list[dict[str, Any]],
                        connection_id: str | None = None, measure: bool | None = None,
                        vectors: list[dict[str, Any]] | None = None,
                        aura_tier: str | None = None) -> Any:
        return self._request("POST", f"/projects/{project_id}/advisor/analyze",
                             {"queries": queries, "connection_id": connection_id,
                              "measure": measure,
                              "vectors": vectors or None, "aura_tier": aura_tier})

    # -- data connectors --------------------------------------------------
    def list_connectors(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/connectors")

    def create_connector(self, project_id: str, name: str, kind: str,
                         config: dict[str, Any], mapping: dict[str, Any] | None = None,
                         poll_interval_secs: int | None = None) -> Any:
        body: dict[str, Any] = {"name": name, "kind": kind, "config": config}
        if mapping is not None:
            body["mapping"] = mapping
        if poll_interval_secs is not None:
            body["poll_interval_secs"] = poll_interval_secs
        return self._request("POST", f"/projects/{project_id}/connectors", body)

    def get_connector(self, project_id: str, connector_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/connectors/{connector_id}")

    def delete_connector(self, project_id: str, connector_id: str) -> None:
        self._request("DELETE", f"/projects/{project_id}/connectors/{connector_id}")

    def test_connector(self, project_id: str, connector_id: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/connectors/{connector_id}/test")

    def sync_connector(self, project_id: str, connector_id: str, full: bool = False) -> Any:
        query = {"full": "true"} if full else None
        return self._request(
            "POST", f"/projects/{project_id}/connectors/{connector_id}/sync", query=query)

    def due_connectors(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/connectors/due")

    # -- auth + audit -----------------------------------------------------
    def auth_login(self, redirect_uri: str) -> Any:
        return self._request("POST", "/auth/login", {"redirect_uri": redirect_uri})

    def auth_callback(self, state: str, id_token: str) -> Any:
        return self._request("POST", "/auth/callback", {"state": state, "id_token": id_token})

    def auth_me(self) -> Any:
        return self._request("GET", "/auth/me")

    def list_grants(self, project_id: str) -> Any:
        return self._request("GET", f"/projects/{project_id}/grants")

    def create_grant(self, project_id: str, email: str, role: str) -> Any:
        return self._request("POST", f"/projects/{project_id}/grants",
                             {"email": email, "role": role})

    def delete_grant(self, project_id: str, grant_id: str) -> None:
        self._request("DELETE", f"/projects/{project_id}/grants/{grant_id}")

    def list_global_grants(self) -> Any:
        return self._request("GET", "/grants")

    def create_global_grant(self, email: str, role: str) -> Any:
        return self._request("POST", "/grants", {"email": email, "role": role})

    def audit_export(self, project_id: str, format: str = "json",
                     after_seq: int | None = None, actor: str | None = None,
                     limit: int | None = None) -> Any:
        return self._request("GET", f"/projects/{project_id}/audit/export",
                             query={"format": format, "after_seq": after_seq,
                                    "actor": actor, "limit": limit})


__all__ = ["RelatingClient", "DEFAULT_BASE_URL", "TOKEN_ENV_VAR", "TARGET_ENV_VAR"]
