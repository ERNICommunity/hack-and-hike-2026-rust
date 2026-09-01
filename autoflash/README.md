# ESP AutoFlash

A focused browser tool for the repetitive ESP firmware loop:

1. Reconnect to a previously authorized serial device.
2. Display its serial log immediately.
3. Watch a selected firmware binary on disk.
4. When the file changes and settles, load the newest bytes and flash them at `0x0`.
5. Reopen the serial monitor and reset the device.

## Configure the device

Edit `SERIAL_PORT_SEARCH` in `src/config.ts` before building:

```ts
export const SERIAL_PORT_SEARCH = "303a:*";
```

The format is `vvvv:pppp`, using hexadecimal USB vendor and product IDs. A product wildcard (`303a:*`) matches every authorized serial device from that vendor. `*` uses the first previously authorized serial port.

Common IDs include:

| Adapter | Search value |
| --- | --- |
| Espressif native USB | `303a:*` |
| Silicon Labs CP210x | `10c4:ea60` |
| WCH CH340 | `1a86:7523` |

Web Serial does not expose the operating system's human-readable port name, so a name such as `USB JTAG/serial debug unit` cannot be used as a reliable browser-side filter.

Other constants in `src/config.ts` control the serial baud rate, flash baud rate, firmware address, polling interval, and stable-file delay.

## Run with Rust only

```bash
cargo run --release
```

That is the entire self-hosting setup. The checked-in `site/` directory contains the prebuilt browser application, and `build.rs` embeds those files into the Rust executable at compile time. The server has no crate dependencies and requires no Node installation, npm packages, runtime asset directory, ChatGPT account, or internet connection.

The resulting executable is `target/release/esp-autoflash-server`. It is a single self-contained binary that can be copied to another compatible machine and run directly:

```bash
./target/release/esp-autoflash-server
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

The first serial authorization and file selection must be initiated by a click due to browser security rules. The site then reuses those grants whenever the browser still permits it. The selected file handle is stored in IndexedDB; firmware bytes are never uploaded.

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
