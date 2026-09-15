/**
 * The device filter: which serial devices Autoflash uses.
 *
 * The Rust server sends it in `/runtime-config.js`. This default is used only
 * when the server sends no filter, for example with the Vite dev server.
 *
 * Format: "vvvv:pppp", the USB vendor ID (VID) and product ID (PID) in
 * hexadecimal. "vvvv:*" matches every product of one vendor. "*" matches
 * every authorized serial port.
 *
 * Common examples:
 *   Espressif native USB   303a:*
 *   Silicon Labs CP210x    10c4:ea60
 *   WCH CH340              1a86:7523
 */
export const SERIAL_PORT_SEARCH =
  window.__ESP_AUTOFLASH_CONFIG__?.serialPortSearch?.trim() || "303a:*";

/** The baud rate of the serial log. */
export const MONITOR_BAUD_RATE = 115_200;
/** The baud rate while flashing. */
export const FLASH_BAUD_RATE = 460_800;
/** The flash address of the firmware file. A merged image belongs at 0x0. */
export const FLASH_ADDRESS = 0x0;

/**
 * How often the page checks the size and modification time of the firmware
 * file. The page polls, because not all browsers have `FileSystemObserver` yet.
 */
export const FILE_POLL_INTERVAL_MS = 400;
/** How long the file must stay the same after a change before the page flashes it. */
export const FILE_STABLE_FOR_MS = 700;

/** A panic backtrace ends when no serial output arrives for this long. */
export const BACKTRACE_QUIET_MS = 300;

/** The largest size of each log, so that a tab that nobody watches does not use more and more memory. */
export const MAX_LOG_CHARACTERS = 350_000;
