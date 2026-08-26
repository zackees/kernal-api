FROM rust:1.95.0-trixie

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        libssl-dev \
        pkg-config \
        python3 \
        python3-venv \
        zstd \
    && rm -rf /var/lib/apt/lists/*

RUN python3 -m venv /opt/soldr \
    && /opt/soldr/bin/pip install --disable-pip-version-check soldr==0.9.10

ENV PATH="/opt/soldr/bin:${PATH}"
WORKDIR /repo
