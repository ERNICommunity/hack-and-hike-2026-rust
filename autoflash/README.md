# ESP AutoFlash

A focused browser tool for the repetitive ESP firmware loop:

1. Reconnect to all previously authorized matching serial devices.
2. Display each device's serial log in its own tab, with Autoflash's own log in a separate rightmost tab.
3. Watch a selected firmware binary on disk.
4. When the file changes and settles, load the newest bytes and flash them at `0x0` to every connected device.
5. Reopen each serial monitor and reset each device.

## Configure the device

Edit `SERIAL_PORT_SEARCH` in `src/config.ts` before building:

```ts
export const SERIAL_PORT_SEARCH = "303a:*";
```

The format is `vvvv:pppp`, using hexadecimal USB vendor and product IDs. A product wildcard (`303a:*`) matches every authorized serial device from that vendor. `*` matches every previously authorized serial port.

Common IDs include:

| Adapter | Search value |
| --- | --- |
| Espressif native USB | `303a:*` |
| Silicon Labs CP210x | `10c4:ea60` |
| WCH CH340 | `1a86:7523` |

Web Serial does not expose the operating system's human-readable port name, so a name such as `USB JTAG/serial debug unit` cannot be used as a reliable browser-side filter.

Authorize additional matching devices with the same button. Automatic and manual flashing apply to every currently connected matching device in parallel when more than one device is attached. Reset applies only to the device selected by the active log tab. Remove selected closes that device's serial connection and removes its tab for the current page session without revoking browser authorization; it can be authorized again later. Devices are processed independently and their serial logs stay separated in device tabs. Autoflash status, lifecycle, error, flash-tool, and diagnostic messages are written only to a dedicated **Autoflash** tab pinned at the far right; device-related application messages are tagged with the device name and USB ID there. By default, a device log is cleared after that device serial monitor successfully reconnects (for example after flashing or a physical reconnect); disable **Clear on reconnect** beside Auto-scroll to preserve it. The active log can also be copied to the clipboard or downloaded. A device tab is created only after its serial port opens successfully; selecting or rediscovering a port that is already connected does not create another tab. When Chromium supplies a new `SerialPort` wrapper after a physical reconnect, AutoFlash reuses the most recently disconnected tab with the same VID:PID instead of creating another stale tab. With multiple physically identical VID:PID devices, the browser exposes no serial number, so reconnects reuse logical slots rather than attempting to preserve hardware identity.

## Decoded panic backtraces

When a device prints a panic backtrace (esp-backtrace's `Backtrace:` followed by one address per line), the addresses are decoded and the function, source file and line of each one, including inlined callers, appear in that device's log right below the backtrace:

```text
Decoded backtrace (/tmp/cargo-target/xtensa-esp32s3-none-elf/release/panic_backtrace):
0x4205f584  panic_backtrace::band_name
            at /workspaces/erni-rust-hack-and-hike-2026/src/bin/panic_backtrace.rs:89
            inlined into panic_backtrace::on_tap
            at /workspaces/erni-rust-hack-and-hike-2026/src/bin/panic_backtrace.rs:78
```

The flashed image carries no debug information, but its application descriptor holds the SHA-256 of the ELF file it was built from (`espflash save-image` fills it in). The browser reads that hash from the firmware it flashed, or from the selected firmware file for a device it has not flashed, and sends it with the addresses to the server's `/api/backtrace` endpoint. The server looks for the ELF file with that hash in the Cargo target directories and runs `addr2line` from the ESP toolchain on it, so the server must run where the firmware is built. A backtrace is only decoded while the ELF file of the running firmware still exists: rebuilding without flashing replaces it, and the log then says that no matching ELF file was found.

Other constants in `src/config.ts` control the serial baud rate, flash baud rate, firmware address, polling interval, and stable-file delay.

## Run with Rust only

```bash
cargo run --release
```

That is the entire self-hosting setup. The checked-in `site/` directory contains the prebuilt browser application, and `build.rs` embeds those files into the Rust executable at compile time. The server has no crate dependencies and requires no Node installation, npm packages, runtime asset directory, ChatGPT account, or internet connection.

The resulting executable is `target/<host triple>/release/esp-autoflash-server`, for example `target/x86_64-unknown-linux-gnu/release/esp-autoflash-server`: `.cargo/config.toml` builds for the machine running Cargo, because the firmware repository around it builds for the ESP32. It is a single self-contained binary that can be copied to another compatible machine and run directly:

```bash
./target/x86_64-unknown-linux-gnu/release/esp-autoflash-server
```

The server listens on `0.0.0.0:8080`, sends `Permissions-Policy: serial=(self)`, applies a self-only Content Security Policy, and supports client-side route fallback.

Select a different USB adapter without rebuilding anything:

```bash
ESP_AUTOFLASH_SERIAL_PORT_SEARCH=10c4:ea60 cargo run --release
```

Accepted values are `vvvv:pppp`, `vvvv:*`, and `*`.

### Connect from a Dev Container host

Forward container port `8080` to the host. For VS Code Dev Containers, merge these values into `.devcontainer/devcontainer.json`:

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

Then open the forwarded URL from the host as `http://localhost:8080`. If VS Code assigns another host port, use the localhost URL shown in the **Ports** panel.

Do not open the site through a raw `http://<container-ip>:8080` URL. Web Serial and the File System Access API require a secure context; browsers treat loopback origins such as `http://localhost` as trustworthy, but not arbitrary HTTP container addresses. Use current desktop Chrome or Edge.

The Rust server can be configured without changing source:

| Environment variable | Default | Purpose |
| --- | --- | --- |
| `ESP_AUTOFLASH_BIND` | `0.0.0.0` | Address inside the container |
| `ESP_AUTOFLASH_PORT` | `8080` | Container listening port |
| `ESP_AUTOFLASH_SERIAL_PORT_SEARCH` | `303a:*` | USB `VID:PID`, vendor wildcard, or `*` |
| `ESP_AUTOFLASH_ELF_DIRS` | `$CARGO_TARGET_DIR`, `target`, `../target` | Cargo target directories searched for the ELF file of a panicking firmware, separated like `PATH` |
| `ESP_AUTOFLASH_ADDR2LINE` | found on `PATH` or in the rustup `esp` toolchain | The `addr2line` used to decode backtraces |

The first authorization for each serial device and the file selection must be initiated by a click due to browser security rules. The site then reuses those grants whenever the browser still permits it. The selected file handle is stored in IndexedDB; firmware bytes are never uploaded.

## Modify the browser UI

Node and npm are only required when changing the TypeScript/CSS sources. Rebuild the checked-in embedded assets with:

```bash
npm ci
npm run build:selfhost
```

Commit the resulting `site/` files. The next Cargo build automatically notices and embeds them.

## Build and test

```bash
cargo test
cargo build --release
```

For frontend development, also run `npm test` and `npm run build:selfhost`.

## Licenses

The server and application code are MIT licensed. Licenses for the browser dependencies embedded in `site/` are included under `THIRD_PARTY_LICENSES/`.

## Firmware layout

This project intentionally writes one binary at address `0x0`. Use a merged/full-flash image made for that address. An app-only ESP-IDF binary commonly belongs at another offset and must not be flashed with this configuration.
