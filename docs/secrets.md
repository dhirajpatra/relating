# Secrets — Solo Env Vars to Team Secret Management

Relating never stores secrets: Aura passwords and provider keys live outside
the database and are resolved at runtime. Every error names the variable,
never the value.

## Resolution order

`RELATING_SECRET_BACKEND` selects the backends (default `auto`):

| Mode | Order |
|---|---|
| `auto` (default, or unset) | `$NAME`, then `$NAME_FILE` |
| `env` | `$NAME` only (strict historical behavior) |
| `file` | `$NAME_FILE`, then `$NAME` |
| `keyring` | OS keychain (`relating` / `$NAME`), then `$NAME`, then `$NAME_FILE` |

`auto` preserves historical behavior exactly when no `_FILE` sidecar exists —
upgrading changes nothing until a team opts into files or the keychain.

## Solo developer (unchanged)

```bash
export AURA_PASSWORD='<secret>'
./start.sh
```

## Team staging (mounted secret files)

Docker/Kubernetes style: mount the secret as a file and point at it. No
rebuild needed.

```bash
export AURA_PASSWORD_FILE=/run/secrets/aura_password
./start.sh
```

## Team workstations (OS keychain)

Requires building with `--features os-keychain` (Linux Secret Service,
macOS Keychain, Windows Credential Manager via the `keyring` crate). On Linux
the build needs the system dbus development files (`apt install libdbus-1-dev
pkg-config` or equivalent):

```bash
cargo build -p relating-server --features relating-core/os-keychain
relating secrets set --name AURA_PASSWORD --from-file /path/to/secret
export RELATING_SECRET_BACKEND=keyring
```

Helpers (never print values):

- `relating secrets check --name AURA_PASSWORD` → `SET` / `NOT SET` + hints.
- `relating secrets delete --name AURA_PASSWORD`.

On headless servers with no Secret Service reachable, keyring operations fail
with a clear error instead of pretending to work — use the file backend there
(see above). The build pins real per-OS keyring backends (keyring 3.x has no
default features; a compile-time guard rejects the in-memory mock fallback).

## Where it applies

Aura connection passwords (`auth_env_var`, default `AURA_PASSWORD`) resolve
through this chain in the API server, the MCP server, and the CLI (`shadow
run`, `advisor analyze --measure`). Connection records still store only the
variable *name*.
