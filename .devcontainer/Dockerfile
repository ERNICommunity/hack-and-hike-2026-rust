FROM mcr.microsoft.com/devcontainers/rust:latest

ENV USER_NAME vscode

USER root

# Allow non-root sudo and configure locale
RUN echo "$USER_NAME ALL=(ALL:ALL) NOPASSWD: ALL" | tee /etc/sudoers.d/$USER_NAME && \
    echo "en_US.UTF-8 UTF-8" > /etc/locale.gen && locale-gen && update-locale

# Install system dependencies
RUN apt-get update && apt-get upgrade -y && \
    apt-get install -y gcc build-essential curl pkg-config && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

USER $USER_NAME

# Use Rust 1.93 consistently for host-side tools such as rustfmt and rust-analyzer.
RUN rustup toolchain install 1.93.0 --component rustfmt --component rust-analyzer --component rust-src && \
    rustup default 1.93.0

# Install cargo-binstall and ESP utilities
RUN cargo install cargo-binstall
RUN cargo binstall -y --secure \
        defmt-print \
        espup \
        cargo-espflash

# Install the matching ESP Xtensa toolchain.
# This is also the version set in the rust-analyzer path in .vscode/settings.json.
RUN espup install -v 1.93.0.0 -t esp32s3

# Add espup environment variables to shell profile
RUN echo 'source $HOME/export-esp.sh' >> /home/$USER_NAME/.bashrc && \
    echo 'source $HOME/export-esp.sh' >> /home/$USER_NAME/.profile

CMD [ "sleep", "infinity" ]