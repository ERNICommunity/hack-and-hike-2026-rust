export type DeviceSearch = {
  vendorId?: number;
  productId?: number;
};

const HEX_ID = /^[0-9a-f]{1,4}$/i;

export function parseDeviceSearch(search: string): DeviceSearch {
  const value = search.trim().toLowerCase().replace(/^0x/, "");
  if (value === "*") return {};

  const [vendorPart, productPart, extra] = value.split(":");
  if (extra !== undefined || !vendorPart || !HEX_ID.test(vendorPart)) {
    throw new Error(`Invalid serial search string “${search}”. Expected vvvv:pppp, vvvv:* or *.`);
  }

  if (!productPart || (productPart !== "*" && !HEX_ID.test(productPart))) {
    throw new Error(`Invalid serial search string “${search}”. Expected vvvv:pppp, vvvv:* or *.`);
  }

  return {
    vendorId: Number.parseInt(vendorPart, 16),
    ...(productPart === "*" ? {} : { productId: Number.parseInt(productPart, 16) }),
  };
}

export function portMatchesSearch(info: SerialPortInfo, search: DeviceSearch): boolean {
  if (search.vendorId !== undefined && info.usbVendorId !== search.vendorId) return false;
  if (search.productId !== undefined && info.usbProductId !== search.productId) return false;
  return true;
}

export function requestPortFilters(search: DeviceSearch): SerialPortFilter[] | undefined {
  if (search.vendorId === undefined) return undefined;
  return [{
    usbVendorId: search.vendorId,
    ...(search.productId === undefined ? {} : { usbProductId: search.productId }),
  }];
}

export function formatPortInfo(info: SerialPortInfo): string {
  const hex = (value: number | undefined) => value === undefined
    ? "????"
    : value.toString(16).padStart(4, "0");
  return `${hex(info.usbVendorId)}:${hex(info.usbProductId)}`;
}
