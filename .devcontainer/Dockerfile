FROM mcr.microsoft.com/devcontainers/rust:latest

# The Rust version is the `rust-version` of the root Cargo.toml; a Dockerfile
# cannot read it, so it is repeated here. The Xtensa toolchain below adds a
# fourth version component.
# The base image sets ENV RUST_VERSION to whatever stable it shipped with, and an
# ENV overrides an ARG of the same name, so the ARG needs a different name.
ARG TOOLCHAIN_VERSION=1.97.0

# The name of the user for the devcontainer session
ARG USER_NAME=vscode

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

# Host toolchain for the host-side tests, rustfmt and rust-analyzer.
RUN rustup toolchain install "$TOOLCHAIN_VERSION" --component rustfmt --component clippy --component rust-analyzer --component rust-src && \
    rustup default "$TOOLCHAIN_VERSION"

# Install cargo-binstall and ESP utilities
RUN cargo install cargo-binstall
RUN cargo binstall -y --secure \
        espup \
        cargo-espflash

# Install the matching ESP Xtensa toolchain.
RUN espup install --toolchain-version "$TOOLCHAIN_VERSION.0" -t esp32s3

# Add espup environment variables to shell profile
RUN echo 'source $HOME/export-esp.sh' >> /home/$USER_NAME/.bashrc && \
    echo 'source $HOME/export-esp.sh' >> /home/$USER_NAME/.profile

CMD [ "sleep", "infinity" ]
