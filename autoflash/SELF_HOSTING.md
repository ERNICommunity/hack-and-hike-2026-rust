# Autoflash server

The Rust server in `server/` sends the Autoflash page to the browser and decodes panic backtraces. [README.md](README.md) describes how to use the page.

## Requirements

- Rust 1.84 or newer, with Cargo. `.cargo/config.toml` uses the `host-tuple` build target, which needs Cargo 1.84.
- Chrome or Edge on a desktop computer. See [Browser requirements](README.md#browser-requirements).
- To decode backtraces: the ESP toolchain and the ELF files of your firmware, on the computer where the server runs. ELF (Executable and Linkable Format) files are the build output with debug information.

You do not need Node, npm or an internet connection:

- The server uses no other crates.
- The `site/` directory contains the built page.
- `build.rs` embeds the files in `site/` into the executable when Cargo builds it.

## Start the server

In the `autoflash/` directory, run:

```bash
cargo run --release
```

The server prints its address, the device filter and the directories where it searches for ELF files. Open <http://localhost:8080>.

The repository around `autoflash/` builds firmware for the ESP32-S3. `.cargo/config.toml` in `autoflash/` builds the server for your computer instead. In this repository, the first build takes longer: Cargo also compiles `core` and `alloc` from source. The repository's Cargo configuration asks for this, and `autoflash/` cannot turn it off.

## Network access

By default, the server listens on `127.0.0.1:8080`. Only programs on the same computer can connect to it.

Always open the page from a `localhost` URL. Web Serial works only in a secure context, and `http://<IP address>` is not one. See [Browser requirements](README.md#browser-requirements).

### VS Code Dev Container

VS Code forwards the port from the container to your computer. This works with the default address `127.0.0.1`. Open <http://localhost:8080> on your computer. If VS Code uses another port, use the `localhost` URL in the **Ports** panel.

The dev container of this repository also uses `--net=host`. On Linux, the container then uses the network of your computer directly.

To give the port a name in VS Code, add these values to `.devcontainer/devcontainer.json`:

```json
{
  "forwardPorts": [8080],
  "portsAttributes": {
    "8080": {
      "label": "ESP AutoFlash",
      "onAutoForward": "notify"
    }
  }
}
```

### Docker with `docker run -p`

Docker sends connections from `-p` to the network interface of the container, not to `127.0.0.1` in the container. So set `ESP_AUTOFLASH_BIND=0.0.0.0` in the container. Then open <http://localhost:8080> on your computer.

### Another computer

The board and the browser can be on one computer and the server on another one. There are two ways to connect:

- **SSH (Secure Shell) port forwarding.** This works with the default address. On the computer with the browser, run `ssh -L 8080:localhost:8080 <server computer>`. Then open <http://localhost:8080>.
- **Direct connection.** Set `ESP_AUTOFLASH_BIND=0.0.0.0` on the server. But the browser then opens `http://<IP address>:8080`, which is not a secure context. So Web Serial does not work without HTTPS.

With `0.0.0.0`, every computer in your network can open the page and use `/api/backtrace`. The answers contain the paths of your ELF and source files.

## Configuration

Set these environment variables when you start the server:

| Environment variable | Default | Purpose |
| --- | --- | --- |
| `ESP_AUTOFLASH_BIND` | `127.0.0.1` | The address that the server listens on. `0.0.0.0` accepts connections from other computers and from `docker run -p`. See [Network access](#network-access). |
| `ESP_AUTOFLASH_PORT` | `8080` | The TCP port that the server listens on. |
| `ESP_AUTOFLASH_SERIAL_PORT_SEARCH` | `303a:*` | The device filter: `vvvv:pppp`, `vvvv:*` or `*`. See [Device filter](README.md#device-filter). |
| `ESP_AUTOFLASH_ELF_DIRS` | `$CARGO_TARGET_DIR`, `target`, `../target` | The Cargo target directories where the server searches for ELF files. Separate several directories like in `PATH` (with `:` on Linux and macOS). |
| `ESP_AUTOFLASH_ADDR2LINE` | Searched, see below | The `addr2line` program that decodes backtraces. |

Example:

```bash
ESP_AUTOFLASH_PORT=9000 ESP_AUTOFLASH_SERIAL_PORT_SEARCH=10c4:ea60 cargo run --release
```

### ELF directories

- If you set `ESP_AUTOFLASH_ELF_DIRS`, the server uses only these directories.
- A relative directory is relative to the directory where you start the server. With `cargo run` in `autoflash/`, `../target` is the target directory of the repository.
- `$CARGO_TARGET_DIR` is used only when it is set. The dev container of this repository sets it to `/tmp/cargo-target`.
- In each directory, the server searches `release/`, `debug/`, `*/release/` and `*/debug/`, for example `target/xtensa-esp32s3-none-elf/release/`. It does not search deeper.
- It uses only Xtensa and RISC-V ELF files. It checks the newest files first.
- It stores the hash of each file and calculates it again only when the file changes.

### addr2line

The server selects the program from the ELF file:

- `xtensa-esp-elf-addr2line` for an Xtensa ELF file, for example for the ESP32-S3
- `riscv32-esp-elf-addr2line` for a RISC-V ELF file, for example for the ESP32-C3

It searches for the program in this order:

1. The directories in `PATH`.
2. The ESP toolchain that `espup` installs, in `$RUSTUP_HOME/toolchains/esp/` and `~/.rustup/toolchains/esp/`. The newest version comes first.

`ESP_AUTOFLASH_ADDR2LINE` sets one program for all ELF files. The server then does not search.

## Build a portable executable

```bash
cargo build --release
```

The executable is `target/<host triple>/release/esp-autoflash-server`, for example `target/x86_64-unknown-linux-gnu/release/esp-autoflash-server`. If `CARGO_TARGET_DIR` is set, it is in that directory instead of `target/`.

The page is embedded, so the executable needs no `site/` directory. Copy it to another computer with the same operating system and processor type, and run it:

```bash
./target/x86_64-unknown-linux-gnu/release/esp-autoflash-server
```

To decode backtraces there, that computer also needs the ELF files and `addr2line`.

## HTTP behavior

Paths:

- `/runtime-config.js` sends the device filter to the page.
- `/api/backtrace?elf=<SHA-256 in hex>&addresses=<hex>,<hex>,...` decodes up to 64 addresses. The answer is JSON (JavaScript Object Notation).
  - `200`: the decoded frames.
  - `400`: the request has a wrong format.
  - `404`: no ELF file has this hash.
  - `500`: another error, for example no `addr2line`.
- Every other path returns an embedded file from `site/`.
  - A path without a file extension that matches no file returns `index.html`. So routes inside the page work.
  - A missing file with an extension returns `404`.

Rules:

- The server accepts only `GET` and `HEAD` requests. Other methods get `405`.
- It sends `Permissions-Policy: serial=(self)`. Only pages from this server can use Web Serial.
- It sends a Content Security Policy (CSP) for the page files. Scripts load only from the server itself.
- The browser checks `index.html` again on each load. It caches the other files for one year: their names contain a hash of their content.
- The server serves at most 32 connections at the same time. It closes more connections at once.
- A client has 5 seconds to send its request header. The header can be at most 32 KiB.
- It closes each connection after one response.

## Tests

See [Tests](README.md#tests).
