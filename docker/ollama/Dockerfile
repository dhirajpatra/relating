# Relating Ollama image: local AI backend (chat + embeddings).
#
# Build context: this folder.
#   docker build -f docker/ollama/Dockerfile -t relating-ollama docker/ollama
#
# On start the entrypoint serves the API, then pre-pulls the models named in
# OLLAMA_PRELOAD_MODELS (space-separated). Point a Relating project at it from
# the workbench AI tab:
#   provider=ollama  base_url=http://ollama:11434/v1
#   chat=llama3.1    embeddings=nomic-embed-text
FROM ollama/ollama:latest

COPY entrypoint.sh /usr/local/bin/ollama-entrypoint.sh
RUN chmod +x /usr/local/bin/ollama-entrypoint.sh

ENV OLLAMA_HOST=0.0.0.0
ENV OLLAMA_PRELOAD_MODELS="llama3.1 nomic-embed-text"

EXPOSE 11434

ENTRYPOINT ["ollama-entrypoint.sh"]
