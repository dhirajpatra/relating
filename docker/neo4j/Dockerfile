# Relating Neo4j image: OPTIONAL local Bolt target for migration testing.
#
# Build context: this folder.
#   docker build -f docker/neo4j/Dockerfile -t relating-neo4j docker/neo4j
#
# Relating's documented migration destination is Neo4j Aura (cloud). This
# service exists so the analyze -> plan -> dry-run -> execute -> verify flow
# can be exercised locally without an Aura account: register an Aura
# connection in the workbench pointing at bolt://neo4j:7687 with the neo4j
# user and the NEO4J_PASSWORD from your .env. It is off by default; enable it
# with `docker compose --profile neo4j up`.
#
# What this image adds over stock neo4j:5-community:
#   - APOC core procedures, commonly expected by migration/verification queries
FROM neo4j:5-community

ENV NEO4J_PLUGINS='["apoc"]'
ENV NEO4J_dbms_security_procedures_unrestricted=apoc.*
ENV NEO4J_dbms_security_procedures_allowlist=apoc.*

EXPOSE 7474 7687
