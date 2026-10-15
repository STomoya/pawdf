#!/bin/bash
set -e

# ==========================================
# 0. Detect Containerization CLI
# ==========================================
if command -v docker >/dev/null 2>&1; then
    CONTAINER_CMD="docker"
elif command -v podman >/dev/null 2>&1; then
    CONTAINER_CMD="podman"
else
    echo "❌ Error: Neither 'docker' nor 'podman' was found in your PATH."
    echo "   Please install a container runtime to continue."
    exit 1
fi

# Use the first script argument as the versions, or default to 3.11 -> 3.15
PYTHON_VERSIONS=${1:-"3.11 3.12 3.13 3.14 3.15"}
IMAGE_NAME="pawdf-uv-test-matrix"

# ==========================================
# 1. Parse pyproject.toml
# ==========================================
if [ ! -f "pyproject.toml" ]; then
    echo "❌ Error: pyproject.toml not found in the current directory."
    exit 1
fi

# Extract the minimum required Python version
MIN_PYTHON_VERSION=$(grep -m 1 -E '^requires-python' pyproject.toml | grep -oE '[0-9]+\.[0-9]+' | head -1)

# Fallback in case the field is missing or unparsable
if [ -z "$MIN_PYTHON_VERSION" ]; then
    echo "⚠️  Warning: Could not parse 'requires-python' from pyproject.toml. Assuming minimum is 3.11."
    MIN_PYTHON_VERSION="3.11"
fi

# Extract the minimum minor version integer (e.g., '11' from '3.11')
MIN_MINOR=$(echo "$MIN_PYTHON_VERSION" | cut -d. -f2)

# ==========================================
# 2. Validate Versions
# ==========================================
echo "🔍 Validating Python versions (Minimum required: >=$MIN_PYTHON_VERSION)..."

for py in $PYTHON_VERSIONS; do
    target_minor=$(echo "$py" | cut -d. -f2)

    if [ -z "$target_minor" ] || [ "$target_minor" -lt "$MIN_MINOR" ]; then
        echo "❌ Error: Unsupported Python version '$py' detected."
        echo "   Your pyproject.toml requires Python >=$MIN_PYTHON_VERSION."
        echo "   Please run the script with valid versions (e.g., ./build_and_test.sh \"$MIN_PYTHON_VERSION 3.12\")"
        exit 1
    fi
done

echo "✅ Versions validated: $PYTHON_VERSIONS"
echo "----------------------------------------"

# ==========================================
# 3. Build Image
# ==========================================
echo "🔨 Building $CONTAINER_CMD image '$IMAGE_NAME'..."

$CONTAINER_CMD build \
    --file container/test.Containerfile \
    --build-arg PYTHON_VERSIONS="$PYTHON_VERSIONS" \
    -t "$IMAGE_NAME" .

# ==========================================
# 4. Run Tests
# ==========================================
echo ""
echo "✅ Build complete. Running tests in $CONTAINER_CMD container..."
echo "----------------------------------------"

$CONTAINER_CMD run --rm "$IMAGE_NAME"
