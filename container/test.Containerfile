# ==========================================
# STAGE 1: Builder
# ==========================================
FROM docker.io/library/debian:bookworm-slim AS builder

# Define the build argument with a default space-separated list
ARG PYTHON_VERSIONS="3.11 3.12 3.13 3.14 3.15"

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl ca-certificates git build-essential \
    && rm -rf /var/lib/apt/lists/*

# Install Rust once at image-build time instead of on every `maturin develop` run
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --profile minimal --default-toolchain stable

COPY --from=ghcr.io/astral-sh/uv:0.11.14 /uv /uvx /bin/

ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1

WORKDIR /app

# Download the requested standalone Python toolchains globally
RUN --mount=type=cache,target=/root/.cache/uv \
    uv python install $PYTHON_VERSIONS

COPY pyproject.toml uv.lock* ./

# 1. Install DEPENDENCIES ONLY.
RUN --mount=type=cache,target=/root/.cache/uv \
    for py in $PYTHON_VERSIONS; do \
    echo "Syncing dependencies for Python $py..." && \
    UV_PROJECT_ENVIRONMENT=/app/.venv-$py uv sync --python $py --all-extras --dev --no-install-project || exit 1; \
    done

# Copy the actual application source code
COPY . .

# 2. Install the PROJECT.
RUN --mount=type=cache,target=/root/.cache/uv \
    for py in $PYTHON_VERSIONS; do \
    echo "Installing project for Python $py..." && \
    UV_PROJECT_ENVIRONMENT=/app/.venv-$py uv sync --python $py --all-extras --dev --reinstall-package pawdf || exit 1; \
    done

# ==========================================
# STAGE 2: Runner
# ==========================================
FROM docker.io/library/debian:bookworm-slim AS runtime

# Re-declare the ARG so this stage can see it, then persist it as an ENV variable
# so the CMD instruction can read it at runtime.
ARG PYTHON_VERSIONS="3.11 3.12 3.13 3.14 3.15"
ENV PYTHON_VERSIONS=$PYTHON_VERSIONS \
    PYTHONUNBUFFERED=1

WORKDIR /app

# 1. Copy the standalone Python toolchains
COPY --from=builder /root/.local/share/uv/python /root/.local/share/uv/python

# 2. Copy the pre-built venvs and your application code
COPY --from=builder /app /app

# Run the test matrix dynamically based on the ENV variable
CMD ["/bin/bash", "container/test.sh"]
