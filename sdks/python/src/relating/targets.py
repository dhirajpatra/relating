"""Query-target handling for the backend flip (docs/api/backend_flip.md).

Application code keeps one Cypher statement and injects the target from
configuration. Only ``RELATING_TARGET`` changes at cutover::

    TARGET = os.environ.get("RELATING_TARGET", "relate://local")
    result = client.cypher_read(project_id, QUERY, {"name": "Ada"}, target=TARGET)
"""

from __future__ import annotations

import os

LOCAL_TARGET = "relate://local"
TARGET_ENV_VAR = "RELATING_TARGET"


def parse_target(target: str) -> str | None:
    """Map a target string to the ``connection_id`` query argument.

    ``relate://local`` runs on the project's own store (``None``);
    ``aura://<connection-id>`` proxies to that Aura connection.
    """
    text = (target or "").strip()
    if text == LOCAL_TARGET:
        return None
    if text.startswith("aura://"):
        connection_id = text[len("aura://"):]
        if connection_id:
            return connection_id
    raise ValueError(
        f"Invalid target {target!r}: use {LOCAL_TARGET!r} or 'aura://<connection-id>'"
    )


def default_target() -> str:
    """Effective target: ``RELATING_TARGET`` or the local store."""
    return os.environ.get(TARGET_ENV_VAR, LOCAL_TARGET)
