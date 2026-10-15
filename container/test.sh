#!/bin/sh
set -e

for py in $PYTHON_VERSIONS; do
    echo "========================================"
    echo "🚀 Testing against Python $py..."
    echo "========================================"
    /app/.venv-$py/bin/pytest
done