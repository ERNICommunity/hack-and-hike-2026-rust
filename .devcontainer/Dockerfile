# Pinned on purpose. With `:latest` the container changes under the
# repository: another Debian, another default Rust, in principle another
# `vscode` user id. Bump this deliberately, and rebuild the container when
# you do. Tags: https://mcr.microsoft.com/v2/devcontainers/rust/tags/list
FROM mcr.microsoft.com/devcontainers/rust:2.1.1-trixie

# The Rust version is the `rust-version` of the root Cargo.toml; a Dockerfile
# cannot read it, so it is repeated here. The Xtensa toolchain below adds a
# fourth version component.
# The base image sets ENV RUST_VERSION to whatever stable it shipped with, and an
# ENV overrides an ARG of the same name, so the ARG needs a different name.
ARG TOOLCHAIN_VERSION=1.97.0

# The name of the user for the devcontainer session
ARG USER_NAME=vscode

# The host-side tools, pinned for the same reason as the base image: a new
# release of espup or espflash must not silently change what a rebuild
# installs. `cargo dist` in .cargo/config.toml is an espflash command line,
# so espflash is part of the build interface of this repository.
ARG BINSTALL_VERSION=1.24.0
ARG ESPUP_VERSION=0.17.1
ARG ESPFLASH_VERSION=4.6.0

USER root

# Allow non-root sudo and configure locale
RUN echo "$USER_NAME ALL=(ALL:ALL) NOPASSWD: ALL" | tee /etc/sudoers.d/$USER_NAME && \
    echo "en_US.UTF-8 UTF-8" > /etc/locale.gen && locale-gen && update-locale

# Install system dependencies
RUN apt-get update && \
    apt-get install -y gcc build-essential curl pkg-config npm && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

USER $USER_NAME

ENV RUST_VERSION=$TOOLCHAIN_VERSION

# Keep build artifacts off the workspace bind mount.
ENV CARGO_TARGET_DIR=/tmp/cargo-target

# Host toolchain for the host-side tests, rustfmt and rust-analyzer.
RUN rustup toolchain install "$TOOLCHAIN_VERSION" --component rustfmt --component clippy --component rust-analyzer --component rust-src && \
    rustup default "$TOOLCHAIN_VERSION"

# Install cargo-binstall and ESP utilities
RUN cargo install cargo-binstall --version "$BINSTALL_VERSION"
RUN cargo binstall -y --secure \
        "espup@$ESPUP_VERSION" \
        "cargo-espflash@$ESPFLASH_VERSION"

# Install the matching ESP Xtensa toolchain.
RUN espup install --toolchain-version "$TOOLCHAIN_VERSION.0" -t esp32s3

# Add espup environment variables to shell profile
RUN echo 'source $HOME/export-esp.sh' >> /home/$USER_NAME/.bashrc && \
    echo 'source $HOME/export-esp.sh' >> /home/$USER_NAME/.profile

CMD [ "sleep", "infinity" ]
