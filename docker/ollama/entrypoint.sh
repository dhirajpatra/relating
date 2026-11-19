#!/bin/sh
# Ollama entrypoint: serve the API, pre-pull models, stay in the foreground.
#
# OLLAMA_PRELOAD_MODELS controls which models are pulled (space-separated,
# e.g. "llama3.1 nomic-embed-text"). Leave it empty to skip pulling entirely
# (models can be pulled later with `docker compose exec ollama ollama pull X`).
# A failed pull is a warning, not fatal: the server stays up so the rest of
# the stack keeps working with the built-in mock provider.
set -eu

# Start the server in the background, then wait for its API.
ollama serve &
SERVER_PID="$!"

echo "Waiting for Ollama API..."
i=1
while [ "$i" -le 60 ]; do
    if ollama list >/dev/null 2>&1; then
        break
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "ollama serve exited unexpectedly" >&2
        exit 1
    fi
    i=$((i + 1))
    sleep 2
done
if ! ollama list >/dev/null 2>&1; then
    echo "Ollama API never became ready" >&2
    exit 1
fi
echo "Ollama API is up."

MODELS="${OLLAMA_PRELOAD_MODELS:-}"
if [ -n "$MODELS" ]; then
    # shellcheck disable=SC2086
    for model in $MODELS; do
        if ollama list | awk 'NR>1 {print $1}' | grep -qx "$model"; then
            echo "Model '$model' already present, skipping pull."
        else
            echo "Pulling model '$model' (first run downloads several GB)..."
            if ollama pull "$model"; then
                echo "Model '$model' ready."
            else
                echo "WARNING: failed to pull '$model'; continuing without it." >&2
            fi
        fi
    done
else
    echo "OLLAMA_PRELOAD_MODELS is empty, skipping model pulls."
fi

echo "Ollama ready. Models:"
ollama list || true

wait "$SERVER_PID"
