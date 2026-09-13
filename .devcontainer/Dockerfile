FROM mcr.microsoft.com/devcontainers/rust:latest

ENV USER_NAME=vscode

USER root

# Allow non-root sudo and configure locale
RUN echo "$USER_NAME ALL=(ALL:ALL) NOPASSWD: ALL" | tee /etc/sudoers.d/$USER_NAME && \
    echo "en_US.UTF-8 UTF-8" > /etc/locale.gen && locale-gen && update-locale

# Install system dependencies
RUN apt-get update && \
    apt-get install -y gcc build-essential curl pkg-config && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

USER $USER_NAME

# Host toolchain for the host-side tests, rustfmt and rust-analyzer. Keep the
# version equal to the Xtensa toolchain version below.
RUN rustup toolchain install 1.97.0 --component rustfmt --component clippy --component rust-analyzer --component rust-src && \
    rustup default 1.97.0

# Install cargo-binstall and ESP utilities
RUN cargo install cargo-binstall
RUN cargo binstall -y --secure \
        espup \
        cargo-espflash

# Install the matching ESP Xtensa toolchain. Keep in sync with
# .github/workflows/firmware-build.yml.
RUN espup install -v 1.97.0.0 -t esp32s3

# Add espup environment variables to shell profile
RUN echo 'source $HOME/export-esp.sh' >> /home/$USER_NAME/.bashrc && \
    echo 'source $HOME/export-esp.sh' >> /home/$USER_NAME/.profile

CMD [ "sleep", "infinity" ]
