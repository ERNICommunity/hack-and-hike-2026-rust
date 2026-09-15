/**
 * The one value most users need to change.
 *
 * Format: "vvvv:pppp" (hex USB vendor and product IDs).
 * Use "vvvv:*" to match every product from one vendor, or "*" to use every
 * previously-authorized serial port.
 *
 * Common examples:
 *   Espressif native USB   303a:*
 *   Silicon Labs CP210x    10c4:ea60
 *   WCH CH340              1a86:7523
 */
export const SERIAL_PORT_SEARCH =
  window.__ESP_AUTOFLASH_CONFIG__?.serialPortSearch?.trim() || "303a:*";

export const MONITOR_BAUD_RATE = 115_200;
export const FLASH_BAUD_RATE = 460_800;
export const FLASH_ADDRESS = 0x0;

/** Polling is used because FileSystemObserver is not yet consistently shipped. */
export const FILE_POLL_INTERVAL_MS = 400;
export const FILE_STABLE_FOR_MS = 700;
export const MONITOR_RESTART_DELAY_MS = 350;

/** A panic backtrace has ended when the serial output has been quiet this long. */
export const BACKTRACE_QUIET_MS = 300;

/** Prevent an unattended tab from growing without bound. */
export const MAX_LOG_CHARACTERS = 350_000;
