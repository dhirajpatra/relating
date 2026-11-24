#!/usr/bin/env bash
#
# Relating — start the Docker stack and migrate a local graph into Aura.
#
# Usage:
#   ./migrate-aura.sh [options]
#
# Options:
#   --project ID      Migrate this project (default: find/create "aura-fill")
#   --connection CID  Use this Aura connection id (default: auto-pick when
#                     exactly one connection is listed, else fail loudly)
#   --fresh           Delete and rebuild the "aura-fill" project from the
#                     bundled employee CSVs before migrating (asks for
#                     confirmation unless --yes is given)
#   --yes             Non-interactive: approve the migration plan and skip
#                     the --fresh confirmation. Without it, the script prints
#                     the plan summary and asks before approving.
#   --llm-ontology    Propose the seed ontology via the LLM instead of the
#                     deterministic tests/fixtures/aura-fill-ontology.json
#                     fixture (the fixture keeps the demo independent of
#                     model availability).
#   --no-build        Skip `docker compose build` (reuse existing images)
#   --up-only         Start the stack and wait for health, then stop without
#                     migrating (plain start; use `docker compose down` to stop)
#   --down            Stop the stack after the migration instead of leaving
#                     it running
#   -h, --help        Show this help
#
# What it does:
#   1. `docker compose up -d` and waits for the API to answer health checks.
#      The base stack is the server alone (SQLite); add
#      `--profile postgres --profile ollama` to `docker compose up` first
#      when the team Postgres or local Ollama models are needed.
#   2. Resolves the project: --project, else the existing "aura-fill"
#      project, else creates it and ingests the bundled 24-employee graph
#      (import the fixed ontology fixture + approve, ingest -> 42 nodes,
#      72 relationships; --llm-ontology uses an LLM proposal instead).
#      --fresh forces step 2 from scratch.
#   3. Resolves the Aura connection (stored or the AURA_URI one from .env),
#      tests it, then runs the approval-gated migration:
#      analyze -> plan -> dry-run -> approve -> execute -> verify -> report.
#      Aborts before approve when the dry run reports blockers, prints the
#      plan summary, and requires an interactive "yes" (or --yes).
#      The final audit report is fetched and written to
#      migration-<id>-report.json next to this script.
#
# Requirements: docker (compose v2), python3, and a .env with AURA_URI plus
# a resolvable AURA_PASSWORD (see .env.example). LLM calls use Meta when
# LLM_BACKEND=meta_api is set, with Ollama fallback inside the stack.
#
# Platform: bash-only (Linux/macOS). On Windows, run it from WSL2 or
# Git Bash with Docker Desktop's compose v2 on PATH; native PowerShell is
# not supported (see README "Windows" notes).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ID=""
CONNECTION_ID=""
FRESH=0
BUILD=1
DOWN=0
UP_ONLY=0
YES=0
LLM_ONTO=0

usage() {
    cat <<'EOF'
Relating — start the Docker stack and migrate a local graph into Aura.

Usage:
  ./migrate-aura.sh [options]

Options:
  --project ID      Migrate this project (default: find/create "aura-fill")
  --connection CID  Use this Aura connection id (default: auto-pick when
                    exactly one connection is listed, else fail loudly)
  --fresh           Delete and rebuild the "aura-fill" project from the
                    bundled employee CSVs before migrating (asks for
                    confirmation unless --yes is given)
  --yes             Non-interactive: approve the migration plan and skip
                    the --fresh confirmation
  --llm-ontology    Propose the seed ontology via the LLM instead of the
                    deterministic fixture
  --no-build        Skip `docker compose build` (reuse existing images)
  --up-only         Start the stack and wait for health, then stop without
                    migrating (plain start; use `docker compose down` to stop)
  --down            Stop the stack after the migration instead of leaving
                    it running
  -h, --help        Show this help
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --project)
            [[ $# -lt 2 || -z "${2:-}" ]] && { echo "--project needs a value" >&2; exit 1; }
            PROJECT_ID="$2"; shift 2 ;;
        --connection)
            [[ $# -lt 2 || -z "${2:-}" ]] && { echo "--connection needs a value" >&2; exit 1; }
            CONNECTION_ID="$2"; shift 2 ;;
        --fresh) FRESH=1; shift ;;
        --yes) YES=1; shift ;;
        --llm-ontology) LLM_ONTO=1; shift ;;
        --no-build) BUILD=0; shift ;;
        --up-only) UP_ONLY=1; shift ;;
        --down) DOWN=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
    esac
done

need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Missing required tool: $1 ($2)" >&2
        exit 1
    fi
}
need docker "install Docker Desktop or the docker engine with compose v2"
need python3 "install Python 3 (used for API calls and JSON parsing)"
if ! docker compose version >/dev/null 2>&1; then
    echo "Missing required tool: docker compose v2" >&2
    exit 1
fi

# Localhost API calls must bypass any globally configured HTTP proxy.
export no_proxy="${no_proxy:-},127.0.0.1,localhost"
export NO_PROXY="${NO_PROXY:-},127.0.0.1,localhost"

# Host-side port and optional API bearer token come from .env when set.
PORT="8080"
API_TOKEN=""
if [[ -f "$ROOT/.env" ]]; then
    ENV_PORT="$(grep -E '^RELATING_PORT=' "$ROOT/.env" | tail -n 1 | cut -d= -f2- | tr -d '[:space:]"'"'" || true)"
    [[ -n "$ENV_PORT" ]] && PORT="$ENV_PORT"
    API_TOKEN="$(grep -E '^RELATING_API_TOKEN=' "$ROOT/.env" | tail -n 1 | cut -d= -f2- | tr -d '[:space:]"'"'" || true)"
fi
[[ -n "${RELATING_API_TOKEN:-}" ]] && API_TOKEN="$RELATING_API_TOKEN"
API="http://127.0.0.1:${PORT}/api/v1"

echo "==> Starting Relating stack..."
if [[ "$BUILD" -eq 1 ]]; then
    (cd "$ROOT" && docker compose up --build -d)
else
    (cd "$ROOT" && docker compose up -d)
fi

echo "==> Waiting for the API at ${API}/health ..."
API_UP=0
for attempt in $(seq 1 90); do
    if python3 -c "
import sys, urllib.request
h = urllib.request.build_opener(urllib.request.ProxyHandler({}))
try:
    h.open('${API}/health', timeout=5).read()
except Exception:
    sys.exit(1)
"; then
        echo "==> API is up."
        API_UP=1
        break
    fi
    if [[ "$attempt" == "90" ]]; then
        echo "API did not answer after ~3 minutes. Recent server logs:" >&2
        (cd "$ROOT" && docker compose logs --tail=30 server) >&2
        exit 1
    fi
    sleep 2
done
if [[ "$API_UP" -ne 1 ]]; then
    echo "API health wait ended without success; aborting." >&2
    exit 1
fi

if [[ "$UP_ONLY" -eq 1 ]]; then
    echo "==> Stack is up (--up-only, skipping migration)."
    echo "    Workbench: http://127.0.0.1:${PORT}"
    echo "    Stop it later with: docker compose down"
    exit 0
fi

echo "==> Running migration flow..."
export MIGRATE_API="$API" MIGRATE_PROJECT="$PROJECT_ID" MIGRATE_CONNECTION="$CONNECTION_ID" MIGRATE_FRESH="$FRESH" MIGRATE_YES="$YES" MIGRATE_TOKEN="$API_TOKEN" MIGRATE_ROOT="$ROOT" MIGRATE_LLM_ONTO="$LLM_ONTO"
STATUS=0
python3 <<'PYEOF' || STATUS=$?
import json, os, sys, urllib.request, urllib.error

API = os.environ["MIGRATE_API"]
WANT_PROJECT = os.environ["MIGRATE_PROJECT"] or None
WANT_CONN = os.environ["MIGRATE_CONNECTION"] or None
FRESH = os.environ["MIGRATE_FRESH"] == "1"
AUTO_YES = os.environ.get("MIGRATE_YES") == "1"
API_TOKEN = os.environ.get("MIGRATE_TOKEN") or ""
ROOT = os.environ.get("MIGRATE_ROOT") or "."
LLM_ONTO = os.environ.get("MIGRATE_LLM_ONTO") == "1"
SEED_NAME = "aura-fill"
ONTO_FIXTURE = os.path.join(ROOT, "tests", "fixtures", "aura-fill-ontology.json")

# Fictional seed people (no real names): the demo graph shape is unchanged.
EMP = """name,role,company,city,company_id,role_id,city_id
Mara Ellison,Data Engineer,Northwind,Lisbon,Northwind,Data Engineer,Lisbon
Tomas Reyes,ML Engineer,Northwind,Lisbon,Northwind,ML Engineer,Lisbon
Priya Nair,Backend Engineer,Initech,Porto,Initech,Backend Engineer,Porto
Jonas Weber,DevOps Engineer,Initech,Porto,Initech,DevOps Engineer,Porto
Sofia Marchetti,QA Engineer,Globex,Madrid,Globex,QA Engineer,Madrid
Liam Okafor,Frontend Engineer,Globex,Madrid,Globex,Frontend Engineer,Madrid
Ingrid Solberg,Data Engineer,Initech,Porto,Initech,Data Engineer,Porto
Mateo Alvarez,ML Engineer,Northwind,Coimbra,Northwind,ML Engineer,Coimbra
Anya Petrova,Backend Engineer,Globex,Madrid,Globex,Backend Engineer,Madrid
Felix Andersen,DevOps Engineer,Umbrella,Barcelona,Umbrella,DevOps Engineer,Barcelona
Nadia Haddad,QA Engineer,Umbrella,Barcelona,Umbrella,QA Engineer,Barcelona
Oliver Bennett,Frontend Engineer,Northwind,Coimbra,Northwind,Frontend Engineer,Coimbra
Clara Dubois,Data Engineer,Globex,Valencia,Globex,Data Engineer,Valencia
Henrik Larsen,ML Engineer,Umbrella,Barcelona,Umbrella,ML Engineer,Barcelona
Elena Popescu,Backend Engineer,Northwind,Lisbon,Northwind,Backend Engineer,Lisbon
David Kimura,DevOps Engineer,Globex,Valencia,Globex,DevOps Engineer,Valencia
Sara Lindqvist,QA Engineer,Initech,Braga,Initech,QA Engineer,Braga
Rafael Costa,Frontend Engineer,Umbrella,Seville,Umbrella,Frontend Engineer,Seville
Amara Diallo,Data Engineer,Initech,Braga,Initech,Data Engineer,Braga
Kenji Tanaka,ML Engineer,Globex,Valencia,Globex,ML Engineer,Valencia
Isabel Ferreira,Backend Engineer,Umbrella,Seville,Umbrella,Backend Engineer,Seville
Viktor Novak,DevOps Engineer,Northwind,Coimbra,Northwind,DevOps Engineer,Coimbra
Hannah Müller,QA Engineer,Initech,Braga,Initech,QA Engineer,Braga
Diego Santos,Frontend Engineer,Globex,Madrid,Globex,Frontend Engineer,Madrid"""
CO = "name\nNorthwind\nInitech\nGlobex\nUmbrella"
CITY = "name\nLisbon\nPorto\nMadrid\nCoimbra\nBarcelona\nValencia\nBraga\nSeville"
ROLE = "title\nData Engineer\nML Engineer\nBackend Engineer\nFrontend Engineer\nDevOps Engineer\nQA Engineer"
SOURCES = [("employees.csv", EMP), ("company.csv", CO),
           ("city.csv", CITY), ("roles.csv", ROLE)]

opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def api(method, path, body=None, timeout=300):
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Content-Type": "application/json"}
    if API_TOKEN:
        headers["Authorization"] = "Bearer " + API_TOKEN
    req = urllib.request.Request(API + path, data=data, method=method,
                                 headers=headers)
    try:
        with opener.open(req, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode() or "{}")
        except Exception:
            return e.code, {"error": {"code": "HTTP_%s" % e.code, "message": str(e)}}

def die(msg):
    print("ERROR: " + msg)
    sys.exit(1)

def step(msg):
    print("--- " + msg)

def confirm(prompt):
    """Interactive yes/no. Returns True only on an explicit yes; auto-yes
    skips the prompt, and non-interactive stdin refuses (fail closed)."""
    if AUTO_YES:
        print("%s [auto-yes]" % prompt)
        return True
    if not sys.stdin.isatty():
        return False
    try:
        answer = input("%s [y/N] " % prompt).strip().lower()
    except EOFError:
        return False
    return answer in ("y", "yes")

# 1. Resolve the project.
pid = WANT_PROJECT
if pid is None:
    s, projects = api("GET", "/projects")
    if s != 200:
        die("listing projects failed: %s" % projects)
    match = next((p for p in projects if p.get("name") == SEED_NAME), None)
    if match and FRESH:
        if not confirm("Delete and rebuild the existing '%s' project?" % SEED_NAME):
            die("--fresh refused: re-run with --yes or answer 'yes' at the prompt")
        step("deleting existing '%s' (--fresh)" % SEED_NAME)
        s, _ = api("DELETE", "/projects/%s" % match["id"])
        if s != 204:
            die("deleting project failed (status %s)" % s)
        match = None
    if match:
        pid = match["id"]
        print("Using existing project '%s' (%s)" % (SEED_NAME, pid))
    else:
        step("creating project '%s'" % SEED_NAME)
        s, proj = api("POST", "/projects", {"name": SEED_NAME,
            "description": "Employee graph for Aura migration"})
        if s != 201:
            die("creating project failed: %s" % proj)
        pid = proj["id"]
        print("Project: %s" % pid)
        step("registering seed sources")
        for fname, content in SOURCES:
            s, src = api("POST", "/projects/%s/sources" % pid,
                         {"filename": fname, "content": content,
                          "content_type": "text/csv"})
            if s not in (200, 201):
                die("registering %s failed: %s" % (fname, src))
        if LLM_ONTO:
            step("proposing + approving ontology (uses the configured LLM)")
            s, onto = api("POST", "/projects/%s/ontology/propose" % pid,
                          {"domain_hint": "company employees with roles across cities"})
            if s not in (200, 201):
                die("ontology propose failed: %s" % str(onto)[:400])
        else:
            step("importing + approving fixed ontology fixture (deterministic)")
            try:
                with open(ONTO_FIXTURE) as fh:
                    fixture = json.load(fh)
            except (OSError, ValueError) as e:
                die("cannot read ontology fixture %s: %s" % (ONTO_FIXTURE, e))
            fixture.pop("_comment", None)
            s, onto = api("POST", "/projects/%s/ontology/import" % pid,
                          {"schema": fixture})
            if s not in (200, 201):
                die("ontology import failed: %s" % str(onto)[:400])
        oid = onto.get("id") or onto.get("ontology", {}).get("id")
        s, _ = api("POST", "/projects/%s/ontology/approve" % pid,
                   {"ontology_id": oid})
        if s != 200:
            die("ontology approve failed")
        step("ingesting")
        s, ing = api("POST", "/projects/%s/ingest" % pid, {})
        if s != 200:
            die("ingest failed: %s" % ing)
        print("Ingested: %s nodes, %s rels" % (ing.get("nodes_created"),
                                               ing.get("relationships_created")))
else:
    s, detail = api("GET", "/projects/%s" % pid)
    if s != 200:
        die("project %s not found: %s" % (pid, detail))

s, detail = api("GET", "/projects/%s" % pid)
schema = detail.get("schema", {})
print("Project '%s': %s nodes, %s rels" % (
    detail.get("project", {}).get("name", pid),
    schema.get("node_count"), schema.get("relationship_count")))
if not schema.get("node_count"):
    die("project has no graph data to migrate")

# 2. Resolve the Aura connection.
s, conns = api("GET", "/projects/%s/connections/aura" % pid)
if s != 200:
    die("listing Aura connections failed: %s" % conns)
cid = WANT_CONN
if cid is None:
    if len(conns) == 1:
        cid = conns[0]["id"]
    elif not conns:
        die("no Aura connections: set AURA_URI + AURA_PASSWORD in .env "
            "or create one via POST .../connections/aura")
    else:
        die("several Aura connections exist; re-run with --connection CID "
            "(%s)" % ", ".join("%s (%s)" % (c["name"], c["id"]) for c in conns))
conn = next((c for c in conns if c["id"] == cid), None)
print("Connection: %s (%s)" % (conn["name"] if conn else cid,
                               conn["uri"] if conn else "?"))

# 3. Test, analyze, plan, dry-run, approve, execute, verify.
step("testing connection")
s, test = api("POST", "/projects/%s/connections/aura/%s/test" % (pid, cid))
if s != 200 or not test.get("reachable"):
    die("Aura test failed: %s" % test)
print("Reachable: %s" % test.get("message"))

step("analyzing migration")
s, analysis = api("POST", "/projects/%s/migrations/analyze" % pid,
                  {"connection_id": cid})
if s != 200:
    die("analyze failed: %s" % analysis)
print("Target reachable: %s agent: %s" % (analysis.get("target_reachable"),
                                          analysis.get("target_agent")))

step("planning migration")
s, plan = api("POST", "/projects/%s/migrations/plan" % pid,
              {"connection_id": cid})
if s not in (200, 201):
    die("plan failed: %s" % plan)
mid = plan["migration"]["id"]
print("Migration: %s" % mid)

step("dry-run (must report zero remote writes)")
s, dry = api("POST", "/projects/%s/migrations/%s/dry-run" % (pid, mid))
if s != 200:
    die("dry-run failed: %s" % dry)
print("remote_writes=%s blockers=%s" % (dry.get("remote_writes"),
                                        dry.get("blockers")))
if dry.get("blockers"):
    die("dry-run blockers must be resolved before approve: %s"
        % dry["blockers"])

# Approval gate: print what execution will do, then require an explicit yes.
dry_plan = dry.get("plan", {})
print("Plan summary: %s nodes, %s relationships in %s batches "
      "(mode: %s, destructive: %s)" % (
          dry_plan.get("total_nodes"), dry_plan.get("total_relationships"),
          dry_plan.get("total_batches"), dry_plan.get("write_mode"),
          dry_plan.get("destructive")))
print("Semantics: %s" % dry_plan.get("write_semantics"))
for warning in dry_plan.get("warnings", []):
    print("Plan warning: %s" % warning)
if not confirm("Approve this migration (writes to Aura)?"):
    die("not approved: re-run with --yes or answer 'yes' at the prompt")

step("approving")
s, _ = api("POST", "/projects/%s/migrations/%s/approve" % (pid, mid),
           {"approved": True})
if s != 200:
    die("approve failed")

step("executing (writes to Aura)")
s, exe = api("POST", "/projects/%s/migrations/%s/execute" % (pid, mid),
             timeout=600)
if s != 200:
    die("execute failed: %s" % str(exe)[:500])
prog = exe.get("progress", exe)
print("Executed: %s" % json.dumps(prog)[:300])

step("verifying")
s, ver = api("POST", "/projects/%s/migrations/%s/verify" % (pid, mid))
if s != 200:
    die("verify failed: %s" % str(ver)[:500])
print("Verify passed: %s" % ver.get("passed", ver))
if not ver.get("passed", False):
    for detail in ver.get("details", []):
        print("Verify detail: %s" % detail)
    die("verification reported passed=false; migration NOT trusted")

step("fetching audit report")
s, report = api("GET", "/projects/%s/migrations/%s/report" % (pid, mid))
if s != 200:
    die("report fetch failed: %s" % str(report)[:300])
report_path = os.path.join(ROOT, "migration-%s-report.json" % mid)
with open(report_path, "w") as fh:
    json.dump(report, fh, indent=2)
print("Report written: %s" % report_path)

print("DONE: project %s migrated to Aura (migration %s)" % (pid, mid))
PYEOF

if [[ "$DOWN" -eq 1 ]]; then
    echo "==> Stopping stack (--down)..."
    (cd "$ROOT" && docker compose down)
else
    echo "==> Stack left running. Workbench: http://127.0.0.1:${PORT}"
    echo "    Stop it later with: docker compose down"
fi

exit $STATUS
