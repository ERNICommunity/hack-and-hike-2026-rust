/**
 * The live copy of a device screen, built from screen packets.
 *
 * `DeviceStreamParser` in `stream.ts` finds the packets in the serial stream
 * and checks them. This module applies them to an image:
 *
 * - Hello (`0x01`): `version u8`, `width u16`, `height u16`. The image gets
 *   this size and turns black.
 * - Rect (`0x02`): `x u16`, `y u16`, `w u16`, `h u16`, then the pixels of
 *   the rectangle in row order, as pixel runs. A control byte `c < 128` is
 *   followed by `c + 1` pixels; a control byte `c >= 128` is followed by one
 *   pixel that repeats `c - 127` times. Each pixel is RGB565, big-endian.
 *
 * All integers in the header are little-endian. The firmware side is in
 * `crates/core/src/screen.rs`.
 */

import { SCREEN_PACKET_MAGIC } from "./config";

/** Kind byte of a Hello packet. */
const KIND_HELLO = 0x01;
/** Kind byte of a Rect packet. */
const KIND_RECT = 0x02;
/** Size of the Rect header: magic, kind and four 16-bit fields. */
const RECT_HEADER_BYTES = 10;
/** The largest screen side the page accepts in a Hello. */
const MAX_SIDE = 2048;
/** The time window of the rate counters, in milliseconds. */
const RATE_WINDOW_MS = 1000;

/** A rectangle of the screen, in pixels. */
export type ScreenRect = { x: number; y: number; w: number; h: number };

/** A parsed packet body. */
export type ScreenPacket =
  | { kind: "hello"; version: number; width: number; height: number }
  | ({ kind: "rect"; runs: Uint8Array } & ScreenRect);

/** Split a checked packet body (without CRC) into its fields. `null` for an unknown or short packet. */
export function parseScreenBody(body: Uint8Array): ScreenPacket | null {
  if (body.length < 2 || body[0] !== SCREEN_PACKET_MAGIC) return null;
  const u16 = (at: number): number => body[at] | (body[at + 1] << 8);
  if (body[1] === KIND_HELLO && body.length >= 7) {
    return { kind: "hello", version: body[2], width: u16(3), height: u16(5) };
  }
  if (body[1] === KIND_RECT && body.length >= RECT_HEADER_BYTES) {
    return { kind: "rect", x: u16(2), y: u16(4), w: u16(6), h: u16(8), runs: body.subarray(RECT_HEADER_BYTES) };
  }
  return null;
}

/**
 * Decode the pixel runs of `rect` into `pixels`, an RGBA image of
 * `imageWidth` x `imageHeight` pixels. Returns `false` without a change when
 * the rectangle does not fit the image, and `false` when the runs are cut short
 * or hold too many pixels (then the pixels before the error are written).
 */
export function decodeRunsInto(
  runs: Uint8Array,
  rect: ScreenRect,
  pixels: Uint8ClampedArray,
  imageWidth: number,
  imageHeight: number,
): boolean {
  if (rect.x + rect.w > imageWidth || rect.y + rect.h > imageHeight) return false;
  const total = rect.w * rect.h;
  let filled = 0;
  let read = 0;

  const put = (value: number): void => {
    const row = Math.floor(filled / rect.w);
    const column = filled - row * rect.w;
    const at = ((rect.y + row) * imageWidth + rect.x + column) * 4;
    const red = (value >> 11) & 0x1f;
    const green = (value >> 5) & 0x3f;
    const blue = value & 0x1f;
    // Repeat the high bits in the low bits, so that full intensity is 255.
    pixels[at] = (red << 3) | (red >> 2);
    pixels[at + 1] = (green << 2) | (green >> 4);
    pixels[at + 2] = (blue << 3) | (blue >> 2);
    pixels[at + 3] = 255;
    filled += 1;
  };

  while (filled < total) {
    if (read >= runs.length) return false;
    const control = runs[read];
    read += 1;
    if (control < 128) {
      const count = control + 1;
      if (read + count * 2 > runs.length || filled + count > total) return false;
      for (let index = 0; index < count; index += 1) {
        put((runs[read] << 8) | runs[read + 1]);
        read += 2;
      }
    } else {
      const count = control - 127;
      if (read + 2 > runs.length || filled + count > total) return false;
      const value = (runs[read] << 8) | runs[read + 1];
      read += 2;
      for (let index = 0; index < count; index += 1) put(value);
    }
  }
  return true;
}

/** The screen image of one device, without the DOM, so that tests can use it. */
export class ScreenState {
  /** The format version from the last Hello; 0 before the first Hello. */
  version = 0;
  width = 0;
  height = 0;
  /** RGBA pixels, `width * height * 4` bytes. */
  pixels: Uint8ClampedArray<ArrayBuffer> = new Uint8ClampedArray(0);

  /**
   * Apply one packet body. Returns what changed: "resized" after a Hello,
   * "pixels" after a Rect that fits, `null` when the packet was ignored.
   */
  apply(body: Uint8Array): "resized" | "pixels" | null {
    const packet = parseScreenBody(body);
    if (!packet) return null;
    if (packet.kind === "hello") {
      if (packet.width < 1 || packet.height < 1 || packet.width > MAX_SIDE || packet.height > MAX_SIDE) return null;
      this.version = packet.version;
      this.width = packet.width;
      this.height = packet.height;
      this.pixels = new Uint8ClampedArray(packet.width * packet.height * 4);
      // Black, fully opaque.
      for (let at = 3; at < this.pixels.length; at += 4) this.pixels[at] = 255;
      return "resized";
    }
    return decodeRunsInto(packet.runs, packet, this.pixels, this.width, this.height) ? "pixels" : null;
  }
}

/**
 * The screen of one device on a `<canvas>`. Each device has its own mirror
 * and canvas; the screen dock shows the canvas of the active device.
 */
export class ScreenMirror {
  readonly state = new ScreenState();
  readonly canvas = document.createElement("canvas");
  /** True after the first valid packet. The dock stays hidden until then. */
  received = false;

  private image?: ImageData;
  private framePending = false;
  /** Arrival time and size of each applied packet in the last `RATE_WINDOW_MS`. */
  private samples: Array<{ time: number; bytes: number }> = [];

  constructor() {
    this.canvas.className = "screen-canvas";
    this.canvas.width = 320;
    this.canvas.height = 240;
  }

  /** Apply one packet body. The canvas is updated once per animation frame, not once per packet. */
  apply(body: Uint8Array): void {
    const change = this.state.apply(body);
    if (!change) return;
    this.received = true;
    // The body, its CRC and the two delimiters; the COBS overhead is left out.
    this.samples.push({ time: performance.now(), bytes: body.length + 4 });
    if (change === "resized") {
      this.canvas.width = this.state.width;
      this.canvas.height = this.state.height;
      // The ImageData uses the same pixel array, so the canvas copies from it
      // without an extra copy on the page side.
      this.image = new ImageData(this.state.pixels, this.state.width, this.state.height);
    }
    if (!this.framePending) {
      this.framePending = true;
      requestAnimationFrame(() => this.draw());
    }
  }

  /** Packets applied per second, over the last second. */
  updatesPerSecond(): number {
    this.pruneSamples();
    return this.samples.length;
  }

  /** Packet bytes per second, over the last second. */
  bytesPerSecond(): number {
    this.pruneSamples();
    return this.samples.reduce((sum, sample) => sum + sample.bytes, 0);
  }

  private pruneSamples(): void {
    const oldest = performance.now() - RATE_WINDOW_MS;
    const firstKept = this.samples.findIndex((sample) => sample.time >= oldest);
    this.samples = firstKept < 0 ? [] : this.samples.slice(firstKept);
  }

  private draw(): void {
    this.framePending = false;
    if (this.image) this.canvas.getContext("2d")?.putImageData(this.image, 0, 0);
  }
}
