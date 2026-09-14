# ESP AutoFlash — standalone Rust host

## Requirements

- Rust 1.65 or newer, including Cargo
- Desktop Chrome or Edge on the host computer

Node, npm, a ChatGPT account, and an internet connection are not required.

The checked-in `site/` directory contains the prebuilt browser application. Cargo embeds it into the executable at compile time.

## Start the server

From this directory:

```bash
cargo run --release
```

The server binds to `0.0.0.0:8080`. The browser application is embedded into the executable during the Cargo build.

If the server runs inside a Dev Container, forward port `8080` and open `http://localhost:8080` on the host. Do not use a raw HTTP container-IP URL because Web Serial requires a secure or trustworthy origin; browsers treat `http://localhost` as trustworthy.

For VS Code Dev Containers, add this to your existing `.devcontainer/devcontainer.json`:

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

## Configuration

| Environment variable | Default | Purpose |
| --- | --- | --- |
| `ESP_AUTOFLASH_BIND` | `0.0.0.0` | Server bind address |
| `ESP_AUTOFLASH_PORT` | `8080` | Server port |
| `ESP_AUTOFLASH_SERIAL_PORT_SEARCH` | `303a:*` | USB `VID:PID`, `VID:*`, or `*` |

Example for a CP210x adapter:

```bash
ESP_AUTOFLASH_SERIAL_PORT_SEARCH=10c4:ea60 cargo run --release
```

Common USB search values:

| Adapter | Value |
| --- | --- |
| Espressif native USB | `303a:*` |
| Silicon Labs CP210x | `10c4:ea60` |
| WCH CH340 | `1a86:7523` |
| All previously authorized ports | `*` |

On first use, the browser still requires a user gesture to grant access to each serial port. Browsers do not allow a page to bypass that permission prompt. After ports have been authorized, the application reconnects automatically to all matching ports when the page loads. Each connected device has its own serial-log tab, while Autoflash application and flash-tool messages are isolated in a dedicated rightmost **Autoflash** tab. Flash operations run concurrently across all currently connected matching devices when possible. Reset targets only the device selected by the active tab, and Remove selected closes and removes only that device for the current page session while keeping browser authorization available for later reuse. Duplicate discovery or authorization of an already-open port is ignored, and a failed duplicate open never leaves an extra tab behind. If Chromium replaces a disconnected device with a new `SerialPort` wrapper, the application rebinds the most recently disconnected matching VID:PID tab to the new wrapper instead of growing another stale tab; identical devices therefore use stable logical slots even though Web Serial does not expose hardware serial numbers. Device logs clear by default after a successful serial-monitor reconnect; the **Clear on reconnect** checkbox disables that behavior, and the active log can be copied or downloaded.

## Build a portable executable

```bash
cargo build --release
```

Copy `target/release/esp-autoflash-server` to another compatible machine. All browser assets are embedded, so no `site/` directory is required next to the executable.

## Tests

```bash
cargo test
```

The tests cover URL decoding, path traversal rejection, USB search validation, content types, and the presence of the embedded application files.

## Firmware warning

This application writes the selected binary at address `0x0`. Use a merged/full-flash image intended for that address. An application-only ESP-IDF binary commonly belongs at another offset.
