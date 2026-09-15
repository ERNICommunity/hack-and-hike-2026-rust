# Autoflash

Autoflash is a browser tool for ESP boards. It flashes your firmware again each time you build it.

Autoflash does these steps:

1. It connects to all serial devices that you authorized before and that match the [device filter](#device-filter).
2. It shows the serial log of each device in its own tab.
3. It watches the firmware file that you choose.
4. When the file changes, it flashes the file to every connected device at address `0x0`.
5. It opens the serial log of each device again and resets the device.
6. When a device panics, it shows the function, source file and line of each backtrace address.

A small Rust server sends the page to the browser and decodes the backtraces. [SELF_HOSTING.md](SELF_HOSTING.md) describes the server in detail.

## Quick start

1. Build a firmware image. In this repository, run `cargo dist --bin <app>` in the repository root. It writes `firmware.bin`. See [Firmware file](#firmware-file).
2. Start the server in a second terminal:

   ```bash
   cd autoflash
   cargo run --release
   ```

3. Open <http://localhost:8080> in Chrome or Edge on a desktop computer.
4. Connect the board over USB (Universal Serial Bus). Click **Authorize device** and select the board.
5. Click **Choose a .bin file** and select `firmware.bin`.

From now on, each `cargo dist` flashes the board again.

The server listens only on `127.0.0.1`, so only the same computer can connect. Do you use a dev container, Docker or a second computer? Then read [Network access](SELF_HOSTING.md#network-access).

## Browser requirements

Autoflash needs two browser interfaces:

- **Web Serial** reads from and writes to the serial port of a device.
- The **File System Access API** (Application Programming Interface) reads the firmware file again after each change.

Use a current version of Chrome or Edge on a desktop computer.

Both interfaces work only in a secure context: a page from `https://` or from `localhost`. A page from `http://<IP address>` is not a secure context. If an interface is missing or the context is not secure, the Autoflash log shows "Unsupported browser".

## Use the page

### Devices

- The first time, click **Authorize device** and select each device. The browser allows this only after a click. Autoflash cannot skip this step.
- When the page loads, Autoflash connects to all authorized devices that match the device filter.
- When you plug in a device again, Autoflash connects to it again.
- A device tab appears only after the serial port of the device opens. A port that is already open does not get a second tab.
- **Reset selected** resets only the device of the active tab. It uses the RTS (request to send) signal of the serial port.
- **Remove selected** closes the serial port of the device in the active tab and removes its tab.
  - The browser keeps the authorization.
  - Autoflash does not connect to the device again until you click **Authorize device** or reload the page.
- During a flash, you cannot use the device buttons, the file selection, **Flash latest now** or the **Automatic flashing** switch.

Identical devices share one USB ID: the vendor ID (VID) and product ID (PID), written `VID:PID`. Web Serial does not give the serial number of a device. So Autoflash cannot tell identical devices apart:

- After you plug in a device again, the browser can give it a new port object.
- Autoflash then reuses the tab of the device with the same `VID:PID` that disconnected last. It does not create a new tab.
- With several identical devices, a device can appear in a tab that another identical device used before.

While a device is offline, Autoflash tries to connect to it every 250 ms. This does not redraw the tabs or logs of connected devices. So you can still select text in their logs.

### Logs

- Each device tab shows the serial log of that device, at 115,200 baud.
- The **Autoflash** tab on the far right shows the messages of Autoflash itself: status, errors and the output of the flash tool.
- In the **Autoflash** tab, a message about one device starts with the device name and USB ID. An example is `Device 1 (303a:1001)`.
- **Clear on reconnect** is on by default. Autoflash then clears a device log when the serial log of the device opens again. This happens after each flash and after you plug in the device again. Turn it off to keep the log.
- The buttons above the log copy the active log, download it as a file or clear it.
- Each log keeps only its last 350,000 characters.

### Flashing

- Autoflash checks the size and modification time of the firmware file every 400 ms.
- After a change, it waits until the file stays the same for 700 ms. Then it flashes the file.
- It flashes all connected devices at the same time, at 460,800 baud.
- The **Automatic flashing** switch turns the file watching on and off.
- **Flash latest now** flashes the current file at once.
- The file can change during a flash. Autoflash then flashes the newer file after the current flash, also when the current flash fails.
- An automatic flash can fail while the browser tab is in the background. Autoflash then tries again when you return to the tab.
- After a flash, Autoflash opens the serial log again and resets the device.

The browser stores a handle to the firmware file in IndexedDB, the database of the browser. A handle is a reference to the file, not its content. Autoflash never uploads the firmware.

After you reload the page, Autoflash uses the same file again. Sometimes the browser asks for permission again. Then click the file button (**Resume**).

## Firmware file

Autoflash writes the whole file at address `0x0`. So the file must be a merged image for address `0x0`. A merged image contains the bootloader, the partition table and the application.

- In this repository, `cargo dist` writes a merged image (`espflash save-image --merge`).
- Do not use an application-only image. It belongs at another address. At `0x0`, the board does not start.

## Device filter

The device filter selects the serial devices that Autoflash uses. Set it with `ESP_AUTOFLASH_SERIAL_PORT_SEARCH` when you start the server:

```bash
ESP_AUTOFLASH_SERIAL_PORT_SEARCH=10c4:ea60 cargo run --release
```

| Value | Matches |
| --- | --- |
| `vvvv:pppp` | One USB vendor ID and product ID, in hexadecimal |
| `vvvv:*` | Every product of one vendor |
| `*` | Every serial port |

The default is `303a:*`. The server stops with an error when the value has another format.

The filter controls two things:

- The devices that the **Authorize device** dialog lists. With `*`, the dialog lists all serial ports.
- The authorized devices that Autoflash connects to automatically.

Common values:

| Adapter | Value |
| --- | --- |
| Espressif native USB | `303a:*` |
| Silicon Labs CP210x | `10c4:ea60` |
| WCH CH340 | `1a86:7523` |

You cannot filter by port name, such as `USB JTAG/serial debug unit`. Web Serial does not give the name that the operating system shows.

The server sends the filter to the page in `/runtime-config.js`. The page uses its own default `303a:*` only when the server sends no filter. The Vite dev server is an example (see [Change the page](#change-the-page)). To use another filter there, change the default of `SERIAL_PORT_SEARCH` in `src/config.ts`.

## Decoded panic backtraces

When the firmware panics, esp-backtrace prints a `Backtrace:` line. One code address per line follows it. The firmware on the board has no debug information, so the addresses have no names. The Rust server finds these names.

Autoflash shows the result in the device log:

```text
Decoded backtrace (/tmp/cargo-target/xtensa-esp32s3-none-elf/release/panic_backtrace):
0x4205f584  panic_backtrace::band_name
            at /workspaces/erni-rust-hack-and-hike-2026/src/bin/panic_backtrace.rs:89
            inlined into panic_backtrace::on_tap
            at /workspaces/erni-rust-hack-and-hike-2026/src/bin/panic_backtrace.rs:78
```

`inlined into` names the caller: the compiler copied the function above into this caller.

### How decoding works

1. Autoflash removes the `Backtrace:` line, the blank lines around it and the addresses from the device log.
2. The backtrace ends at the first line that is not an address. It also ends when the device sends nothing for 300 ms. A backtrace has at most 64 addresses.
3. Autoflash reads the ELF hash from the firmware image.
   - The ELF (Executable and Linkable Format) file is the build output with the debug information.
   - The image contains an application descriptor. `espflash save-image` writes the SHA-256 hash of the ELF file into it.
4. The browser sends the hash and the addresses to `/api/backtrace` on the server.
5. The server searches its ELF directories for the ELF file with this hash. It runs `addr2line` from the ESP toolchain on that file.
6. Autoflash adds the decoded backtrace to the device log.

Which hash does Autoflash use for a device?

- For a device that it flashed, the hash of the firmware that it flashed.
- For a device that it did not flash, the hash of the firmware file that you chose.
- For a device whose flash failed, the hash of the firmware that the device had before. So Autoflash does not decode its backtraces with the new build.

### When decoding fails

The log then shows `Backtrace not decoded:`, the reason and the raw addresses. Common reasons:

- **No matching ELF file.** The ELF file of the running firmware no longer exists, or it is not in an ELF directory. For example, you rebuilt without flashing, and the build replaced the ELF file.
- **The server does not run where you build.** The server needs the ELF files. So it must run on the computer where you build the firmware.
- **No hash.** Autoflash did not flash the device, and you did not choose a firmware file. Or the image has no application descriptor with an ELF hash.
- **"this server does not decode backtraces".** Only the Rust server decodes backtraces. The Vite dev server and the Cloudflare Worker answer `/api/backtrace` with the page.

A `Backtrace:` line without an address after it stays in the log as normal text.

[Configuration](SELF_HOSTING.md#configuration) describes where the server searches for ELF files and which `addr2line` it uses.

## Change the page

The browser page is TypeScript and CSS in `src/`. You need Node and npm only to change it.

The `site/` directory contains the built page. `build.rs` embeds these files into the server executable. After a change to `src/`, build `site/` again:

```bash
npm ci
npm run build:selfhost
```

The build deletes the old files in `site/`. Commit all changes in `site/`. The next Cargo build embeds the new files.

Other settings are constants in `src/config.ts`:

- the baud rate of the serial log and of flashing
- the flash address
- the file check interval and the wait time for a stable file
- the quiet time that ends a backtrace
- the maximum log size

After a change, build `site/` again.

Other ways to serve the page:

- `npm run dev` starts the Vite dev server, which reloads the page after each change. Open the address that Vite prints. It uses the default device filter and cannot decode backtraces.
- `npm run build` writes the page to `dist/client` and a Cloudflare Worker to `dist/server`. The Worker cannot decode backtraces.

## Tests

```bash
cargo test
npm test
```

- `cargo test` tests the Rust server:
  - URL decoding and the rejection of `..` in paths
  - device filter validation and content types
  - the embedded page files and SHA-256
  - backtrace requests, `addr2line` output and the JSON answer
- `npm test` tests the page with Vitest. Run `npm ci` first. It tests:
  - device filter parsing
  - the ELF hash in an image
  - backtrace detection in the log
  - the decoded text and the request to the server

## Licenses

The page in `site/` contains code from esptool-js, pako, atob-lite and tslib. Their licenses are in `THIRD_PARTY_LICENSES/`.
