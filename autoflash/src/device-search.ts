/**
 * A parsed device filter. A missing ID matches every value, so `{}` matches
 * every serial port.
 */
export type DeviceSearch = {
  /** The USB vendor ID (VID). */
  vendorId?: number;
  /** The USB product ID (PID). */
  productId?: number;
};

/** A USB ID of one to four hexadecimal digits, with one optional `0x` in front, as the server accepts it. */
const HEX_ID = /^(?:0x)?[0-9a-f]{1,4}$/i;

/**
 * Parse a device filter: `vvvv:pppp`, `vvvv:*` or `*`. Each ID can have its
 * own `0x` prefix. Throws an `Error` for any other format.
 */
export function parseDeviceSearch(search: string): DeviceSearch {
  const value = search.trim().toLowerCase();
  if (value === "*") return {};

  const [vendorPart, productPart, extra] = value.split(":");
  if (extra !== undefined || !vendorPart || !HEX_ID.test(vendorPart)) {
    throw new Error(`Invalid serial search string “${search}”. Expected vvvv:pppp, vvvv:* or *.`);
  }

  if (!productPart || (productPart !== "*" && !HEX_ID.test(productPart))) {
    throw new Error(`Invalid serial search string “${search}”. Expected vvvv:pppp, vvvv:* or *.`);
  }

  // parseInt with radix 16 also accepts a leading "0x".
  return {
    vendorId: Number.parseInt(vendorPart, 16),
    ...(productPart === "*" ? {} : { productId: Number.parseInt(productPart, 16) }),
  };
}

/** Whether a serial port with these USB IDs matches the filter. */
export function portMatchesSearch(info: SerialPortInfo, search: DeviceSearch): boolean {
  if (search.vendorId !== undefined && info.usbVendorId !== search.vendorId) return false;
  if (search.productId !== undefined && info.usbProductId !== search.productId) return false;
  return true;
}

/**
 * The filters for `navigator.serial.requestPort()`. Returns `undefined` for
 * `*`, so that the browser dialog lists all serial ports.
 */
export function requestPortFilters(search: DeviceSearch): SerialPortFilter[] | undefined {
  if (search.vendorId === undefined) return undefined;
  return [{
    usbVendorId: search.vendorId,
    ...(search.productId === undefined ? {} : { usbProductId: search.productId }),
  }];
}

/**
 * The USB ID of a port as `vvvv:pppp`, for example `303a:1001`. An unknown ID
 * (for example of a port that is not USB) is shown as `????`.
 */
export function formatPortInfo(info: SerialPortInfo): string {
  const hex = (value: number | undefined) => value === undefined
    ? "????"
    : value.toString(16).padStart(4, "0");
  return `${hex(info.usbVendorId)}:${hex(info.usbProductId)}`;
}
