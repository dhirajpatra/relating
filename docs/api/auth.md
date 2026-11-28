# Enterprise Identity — SSO, RBAC, and Audit

Base path: `/api/v1`. Locked mode engages when the server carries a static
`RELATING_API_TOKEN` and/or OIDC configuration; with neither, the API stays
open (loopback only) and every request runs as `anonymous`.

## Authentication

Bearers resolve in order: the static API token (break-glass administrator,
constant-time compared), an opaque login session token (12-hour TTL, hashed
at rest), or a verified OIDC JWT for an identity linked through a prior
callback. Unknown identities must complete the handshake first — a valid
JWT alone does not auto-provision API access.

- `POST /auth/login` — `{redirect_uri}` (absolute http(s) URL). Returns
  `{auth_url, state, expires_in_secs}`; states are single-use and expire
  after ten minutes. The frontend completes the provider flow and posts
  back to the callback.
- `POST /auth/callback` — `{state, id_token}`. Verifies the HS256 JWT
  (issuer, audience, expiry with 30s leeway, signature), links
  `(issuer, subject)` to a user (existing identity, same-email shell, or
  fresh row), and returns `{token, token_type, expires_at, user}`.
- `GET /auth/me` — the caller's `{actor, kind, email, global_role,
  project_role, global_admin}`. Works for grantless users.

Configuration: `RELATING_OIDC_ISSUER` + `RELATING_OIDC_AUDIENCE` enable SSO,
`RELATING_OIDC_CLIENT_SECRET` verifies HS256 id tokens, and
`RELATING_OIDC_AUTH_URL` overrides `{issuer}/authorize` for login.

## RBAC

Roles are `viewer` (read), `editor` (read + write), `admin` (grant
management, global only). Effective role is the stronger of the global and
project grants; project creators are auto-granted admin on their project.

- `GET /grants`, `POST /grants {email, role}` — global grants (admin only).
- `GET /projects/{id}/grants`, `POST .../grants {email, role}` — project
  grants (admin only). Admins may pre-grant addresses that never signed in;
  the identity links on first login.
- `DELETE /projects/{id}/grants/{grant_id}` — revoke (admin only, 204).

Writes need an editor grant; drift checks (`POST .../drift`) are reads and
need only a viewer grant. Migration approvals add a second-actor rule: an
SSO user cannot approve their own plan. Break-glass and anonymous modes
are exempt (no second identity exists to compare).

Migration progress stamps `created_by`, `approved_by`, and `executed_by`
(`user:<email>`, `api-token`, or `anonymous`) for the compliance trail.

## Hardening

- Token comparisons are constant-time on both the API and MCP gates.
- CORS allows `GET/POST/PUT/DELETE` with `Authorization`/`Content-Type`
  headers; origins stay open (loopback + same-origin workbench).
- Five consecutive 401s lock the client key out for a minute (`429
  RATE_LIMITED` with `Retry-After`); `/health` never locks.
- Unknown `RELATING_SECRET_BACKEND` modes fail closed (no backend
  consulted) instead of silently falling back to `auto`.
- Provider API keys are write-only: accepted on project create, used
  internally, never returned by the API.
- Connection `auth_env_var` names are allowlisted (`AURA_*` /
  `RELATING_*` prefixes or `*_PASSWORD` / `*_TOKEN` / `*_SECRET` / `*_KEY`
  suffixes, `UPPER_SNAKE_CASE`) so connections cannot point at unrelated
  process environment.
- MCP HTTP snapshots `RELATING_MCP_TOKEN` at boot (no per-request env
  reads) and accepts session tokens scoped to the caller's grants for the
  `x-relating-project` header; the static token stays global.

## Audit

Every gated request appends `{seq, actor, action, resource, outcome,
detail, prev_hash, hash, created_at}` to a hash-chained, append-only log
(no update/delete API); explicit domain events cover logins, grants,
approvals, and executions. A failing audit store is traced but never fails
the request it records.

- `GET /projects/{id}/audit/export[?format=json|csv&after_seq=N&actor=A&limit=N]`
  — project-scoped rows (resource-path filter) plus full-chain
  verification (`{valid, checked}` or the first break). CSV returns the
  event rows with a header line.

## Limits (stated, not silent)

- JWT verification is HS256 only and dependency-free; RS256/ES256 tokens
  are rejected naming the `jsonwebtoken` follow-up.
- The callback accepts id tokens directly (frontend-completed flow); a
  server-side authorization-code exchange needs an HTTP client and is not
  implemented.
- Audit verification pages the first 5000 rows; retention purges are
  explicit operator SQL (documented, not automated).
- HashiCorp Vault / cloud KMS secret backends are deferred by request;
  secrets resolve via env, `<NAME>_FILE`, and the OS keychain as before.
