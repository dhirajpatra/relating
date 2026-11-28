"""Typed errors for the Relating client.

Every server failure arrives as ``{"error": {"code", "message"}}`` with an
HTTP status; this module preserves all three so callers can branch on what
they can handle (retry a 502, re-authenticate a 401) instead of parsing
message strings.
"""

from __future__ import annotations


class RelatingError(Exception):
    """Base error for every failed Relating API call."""

    def __init__(self, message: str, *, status: int | None = None, code: str | None = None):
        super().__init__(message)
        self.message = message
        self.status = status
        self.code = code

    def __str__(self) -> str:  # pragma: no cover - trivial
        if self.code and self.status:
            return f"[{self.code} HTTP {self.status}] {self.message}"
        return self.message


class AuthenticationError(RelatingError):
    """401/403: missing, invalid, or unauthorized credentials."""


class NotFoundError(RelatingError):
    """404: the referenced project, source, or migration does not exist."""


class ValidationError(RelatingError):
    """400: the request itself is malformed."""


class UpstreamError(RelatingError):
    """502: a downstream dependency (Aura, provider, connector) failed."""


class RateLimitedError(RelatingError):
    """429: authentication lockout or API rate limit."""


def error_from_response(status: int, payload: object) -> RelatingError:
    """Build the narrowest error for an HTTP failure."""
    code: str | None = None
    message = f"HTTP {status}"
    if isinstance(payload, dict):
        error = payload.get("error")
        if isinstance(error, dict):
            raw_code = error.get("code")
            raw_message = error.get("message")
            code = str(raw_code) if raw_code is not None else None
            if isinstance(raw_message, str) and raw_message:
                message = raw_message
    if status == 401 or status == 403:
        return AuthenticationError(message, status=status, code=code)
    if status == 404:
        return NotFoundError(message, status=status, code=code)
    if status == 400:
        return ValidationError(message, status=status, code=code)
    if status == 429:
        return RateLimitedError(message, status=status, code=code)
    if status == 502:
        return UpstreamError(message, status=status, code=code)
    return RelatingError(message, status=status, code=code)
