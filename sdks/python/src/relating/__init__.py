"""Relating Python SDK: ergonomic client over the JSON API."""

from .client import DEFAULT_BASE_URL, TOKEN_ENV_VAR, RelatingClient
from .errors import (
    AuthenticationError,
    NotFoundError,
    RateLimitedError,
    RelatingError,
    UpstreamError,
    ValidationError,
    error_from_response,
)
from .targets import LOCAL_TARGET, TARGET_ENV_VAR, default_target, parse_target

__version__ = "0.1.0"

__all__ = [
    "RelatingClient",
    "RelatingError",
    "AuthenticationError",
    "NotFoundError",
    "ValidationError",
    "UpstreamError",
    "RateLimitedError",
    "error_from_response",
    "parse_target",
    "default_target",
    "LOCAL_TARGET",
    "TARGET_ENV_VAR",
    "DEFAULT_BASE_URL",
    "TOKEN_ENV_VAR",
    "__version__",
]
