import { describe, expect, it } from "vitest";
import { decodeRunsInto, parseScreenBody, ScreenState } from "./screen";

/** A packet body without its CRC: magic, kind and the 16-bit fields, little-endian. */
function body(kind: number, fields: number[], tail: number[] = []): Uint8Array {
  const out = [0xfe, kind];
  for (const field of fields) out.push(field & 0xff, field >> 8);
  return Uint8Array.from([...out, ...tail]);
}

/** A Hello body; its version is a single byte before the size. */
function hello(width: number, height: number): Uint8Array {
  return Uint8Array.of(0xfe, 0x01, 1, width & 0xff, width >> 8, height & 0xff, height >> 8);
}

/** The RGBA value of pixel (x, y) of a state. */
function rgba(state: ScreenState, x: number, y: number): number[] {
  const at = (y * state.width + x) * 4;
  return [...state.pixels.subarray(at, at + 4)];
}

describe("pixel runs", () => {
  it("decodes literal and repeat runs in row order", () => {
    const pixels = new Uint8ClampedArray(4 * 2 * 4);
    // A literal run of 2 (white, red), then 6 copies of blue.
    const runs = Uint8Array.of(0x01, 0xff, 0xff, 0xf8, 0x00, 0x85, 0x00, 0x1f);
    expect(decodeRunsInto(runs, { x: 0, y: 0, w: 4, h: 2 }, pixels, 4, 2)).toBe(true);
    expect([...pixels.subarray(0, 8)]).toEqual([255, 255, 255, 255, 255, 0, 0, 255]);
    expect([...pixels.subarray(28, 32)]).toEqual([0, 0, 255, 255]);
  });

  it("refuses runs that are short or overfill the rectangle", () => {
    const pixels = new Uint8ClampedArray(4 * 4);
    expect(decodeRunsInto(Uint8Array.of(0x01, 0xff, 0xff), { x: 0, y: 0, w: 2, h: 1 }, pixels, 4, 1)).toBe(false);
    expect(decodeRunsInto(Uint8Array.of(0x84, 0x00, 0x01), { x: 0, y: 0, w: 2, h: 1 }, pixels, 4, 1)).toBe(false);
  });
});

describe("screen state", () => {
  it("resizes and clears to black on Hello", () => {
    const state = new ScreenState();
    expect(parseScreenBody(hello(320, 240))).toEqual({ kind: "hello", version: 1, width: 320, height: 240 });
    expect(state.apply(hello(320, 240))).toBe("resized");
    expect(state).toMatchObject({ version: 1, width: 320, height: 240 });
    expect(state.pixels.length).toBe(320 * 240 * 4);
    expect(rgba(state, 319, 239)).toEqual([0, 0, 0, 255]);
  });

  it("draws a rectangle inside the image", () => {
    const state = new ScreenState();
    state.apply(hello(8, 4));
    expect(state.apply(body(0x02, [2, 1, 3, 2], [0x85, 0x07, 0xe0]))).toBe("pixels");
    expect(rgba(state, 2, 1)).toEqual([0, 255, 0, 255]);
    expect(rgba(state, 4, 2)).toEqual([0, 255, 0, 255]);
    expect(rgba(state, 5, 2)).toEqual([0, 0, 0, 255]);
  });

  it("ignores a rectangle that does not fit", () => {
    const state = new ScreenState();
    state.apply(hello(8, 4));
    const before = state.pixels.slice();
    expect(state.apply(body(0x02, [6, 0, 3, 1], [0x82, 0xff, 0xff]))).toBeNull();
    expect(state.pixels).toEqual(before);
  });

  it("ignores a rectangle before the first Hello", () => {
    expect(new ScreenState().apply(body(0x02, [0, 0, 1, 1], [0x00, 0xff, 0xff]))).toBeNull();
  });
});
