import { describe, expect, it } from "vitest";
import { parseDeviceSearch, portMatchesSearch, requestPortFilters } from "./device-search";

describe("serial device search", () => {
  it("parses exact VID:PID matches", () => {
    expect(parseDeviceSearch("303a:1001")).toEqual({ vendorId: 0x303a, productId: 0x1001 });
  });

  it("accepts a 0x prefix on each ID, like the server", () => {
    expect(parseDeviceSearch("0x303a:0x1001")).toEqual({ vendorId: 0x303a, productId: 0x1001 });
    expect(parseDeviceSearch("303A:0x1001")).toEqual({ vendorId: 0x303a, productId: 0x1001 });
    expect(() => parseDeviceSearch("0x0x303a:*")).toThrow(/Invalid serial search string/);
    expect(() => parseDeviceSearch("0x:*")).toThrow(/Invalid serial search string/);
  });

  it("parses vendor wildcards", () => {
    expect(parseDeviceSearch("10c4:*")).toEqual({ vendorId: 0x10c4 });
  });

  it("supports the first authorized port wildcard", () => {
    expect(parseDeviceSearch("*")).toEqual({});
    expect(requestPortFilters({})).toBeUndefined();
  });

  it("matches the exposed Web Serial IDs", () => {
    expect(portMatchesSearch({ usbVendorId: 0x303a, usbProductId: 0x1001 }, { vendorId: 0x303a })).toBe(true);
    expect(portMatchesSearch({ usbVendorId: 0x10c4, usbProductId: 0xea60 }, { vendorId: 0x303a })).toBe(false);
  });

  it("rejects ambiguous names", () => {
    expect(() => parseDeviceSearch("USB JTAG/serial debug unit")).toThrow(/Invalid serial search string/);
  });
});
