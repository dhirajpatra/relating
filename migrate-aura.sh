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
#                     bundled employee CSVs before migrating
#   --no-build        Skip `docker compose build` (reuse existing images)
#   --down            Stop the stack after the migration instead of leaving
#                     it running
#   -h, --help        Show this help
#
# What it does:
#   1. `docker compose up -d` (server + postgres + ollama) and waits for
#      the API to answer health checks.
#   2. Resolves the project: --project, else the existing "aura-fill"
#      project, else creates it and ingests the bundled 24-employee graph
#      (propose + approve ontology, ingest -> 42 nodes, 72 relationships).
#      --fresh forces step 2 from scratch.
#   3. Resolves the Aura connection (stored or the AURA_URI one from .env),
#      tests it, then runs the approval-gated migration:
#      analyze -> plan -> dry-run -> approve -> execute -> verify.
#      Aborts before approve when the dry run reports blockers.
#
# Requirements: docker (compose v2), python3, and a .env with AURA_URI plus
# a resolvable AURA_PASSWORD (see .env.example). LLM calls use Meta when
# LLM_BACKEND=meta_api is set, with Ollama fallback inside the stack.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ID=""
CONNECTION_ID=""
FRESH=0
BUILD=1
DOWN=0

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
                    bundled employee CSVs before migrating
  --no-build        Skip `docker compose build` (reuse existing images)
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
        --no-build) BUILD=0; shift ;;
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

# Host-side port comes from .env when set (compose default: 8080).
PORT="8080"
if [[ -f "$ROOT/.env" ]]; then
    ENV_PORT="$(grep -E '^RELATING_PORT=' "$ROOT/.env" | tail -n 1 | cut -d= -f2- | tr -d '[:space:]"'"'" || true)"
    [[ -n "$ENV_PORT" ]] && PORT="$ENV_PORT"
fi
API="http://127.0.0.1:${PORT}/api/v1"

echo "==> Starting Relating stack (server + postgres + ollama)..."
if [[ "$BUILD" -eq 1 ]]; then
    (cd "$ROOT" && docker compose up --build -d)
else
    (cd "$ROOT" && docker compose up -d)
fi

echo "==> Waiting for the API at ${API}/health ..."
for _ in $(seq 1 90); do
    if python3 -c "
import sys, urllib.request
h = urllib.request.build_opener(urllib.request.ProxyHandler({}))
try:
    h.open('${API}/health', timeout=5).read()
except Exception:
    sys.exit(1)
"; then
        echo "==> API is up."
        break
    fi
    sleep 2
    if [[ "${_}" == "90" ]]; then
        echo "API did not answer after ~3 minutes. Recent server logs:" >&2
        (cd "$ROOT" && docker compose logs --tail=30 server) >&2
        exit 1
    fi
done

echo "==> Running migration flow..."
export MIGRATE_API="$API" MIGRATE_PROJECT="$PROJECT_ID" MIGRATE_CONNECTION="$CONNECTION_ID" MIGRATE_FRESH="$FRESH"
STATUS=0
python3 <<'PYEOF' || STATUS=$?
import json, os, sys, urllib.request, urllib.error

API = os.environ["MIGRATE_API"]
WANT_PROJECT = os.environ["MIGRATE_PROJECT"] or None
WANT_CONN = os.environ["MIGRATE_CONNECTION"] or None
FRESH = os.environ["MIGRATE_FRESH"] == "1"
SEED_NAME = "aura-fill"

EMP = """name,role,company,city,company_id,role_id,city_id
Ada Lovelace,Data Engineer,Northwind,Lisbon,Northwind,Data Engineer,Lisbon
Alan Turing,ML Engineer,Northwind,Lisbon,Northwind,ML Engineer,Lisbon
Grace Hopper,Backend Engineer,Initech,Porto,Initech,Backend Engineer,Porto
Linus Torvalds,DevOps Engineer,Initech,Porto,Initech,DevOps Engineer,Porto
Margaret Hamilton,QA Engineer,Globex,Madrid,Globex,QA Engineer,Madrid
Tim Berners-Lee,Frontend Engineer,Globex,Madrid,Globex,Frontend Engineer,Madrid
Barbara Liskov,Data Engineer,Initech,Porto,Initech,Data Engineer,Porto
Donald Knuth,ML Engineer,Northwind,Coimbra,Northwind,ML Engineer,Coimbra
Edsger Dijkstra,Backend Engineer,Globex,Madrid,Globex,Backend Engineer,Madrid
Radia Perlman,DevOps Engineer,Umbrella,Barcelona,Umbrella,DevOps Engineer,Barcelona
Katherine Johnson,QA Engineer,Umbrella,Barcelona,Umbrella,QA Engineer,Barcelona
John von Neumann,Frontend Engineer,Northwind,Coimbra,Northwind,Frontend Engineer,Coimbra
Claude Shannon,Data Engineer,Globex,Valencia,Globex,Data Engineer,Valencia
Richard Hamming,ML Engineer,Umbrella,Barcelona,Umbrella,ML Engineer,Barcelona
Frances Allen,Backend Engineer,Northwind,Lisbon,Northwind,Backend Engineer,Lisbon
Ken Thompson,DevOps Engineer,Globex,Valencia,Globex,DevOps Engineer,Valencia
Dennis Ritchie,QA Engineer,Initech,Braga,Initech,QA Engineer,Braga
Bjarne Stroustrup,Frontend Engineer,Umbrella,Seville,Umbrella,Frontend Engineer,Seville
Guido van Rossum,Data Engineer,Initech,Braga,Initech,Data Engineer,Braga
Yukihiro Matsumoto,ML Engineer,Globex,Valencia,Globex,ML Engineer,Valencia
James Gosling,Backend Engineer,Umbrella,Seville,Umbrella,Backend Engineer,Seville
Anders Hejlsberg,DevOps Engineer,Northwind,Coimbra,Northwind,DevOps Engineer,Coimbra
Brendan Eich,QA Engineer,Initech,Braga,Initech,QA Engineer,Braga
Sheryl Sandberg,Frontend Engineer,Globex,Madrid,Globex,Frontend Engineer,Madrid"""
CO = "name\nNorthwind\nInitech\nGlobex\nUmbrella"
CITY = "name\nLisbon\nPorto\nMadrid\nCoimbra\nBarcelona\nValencia\nBraga\nSeville"
ROLE = "title\nData Engineer\nML Engineer\nBackend Engineer\nFrontend Engineer\nDevOps Engineer\nQA Engineer"
SOURCES = [("employees.csv", EMP), ("company.csv", CO),
           ("city.csv", CITY), ("roles.csv", ROLE)]

opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def api(method, path, body=None, timeout=300):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(API + path, data=data, method=method,
                                 headers={"Content-Type": "application/json"})
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

# 1. Resolve the project.
pid = WANT_PROJECT
if pid is None:
    s, projects = api("GET", "/projects")
    if s != 200:
        die("listing projects failed: %s" % projects)
    match = next((p for p in projects if p.get("name") == SEED_NAME), None)
    if match and FRESH:
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
        step("proposing + approving ontology (uses the configured LLM)")
        s, onto = api("POST", "/projects/%s/ontology/propose" % pid,
                      {"domain_hint": "company employees with roles across cities"})
        if s not in (200, 201):
            die("ontology propose failed: %s" % str(onto)[:400])
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
