import { describe, expect, it } from "vitest";
import { parseScreenBody } from "./screen";
import { cobsDecode, crc16, DeviceStreamParser } from "./stream";

/**
 * The golden packet from `golden_packet` in `crates/core/src/screen.rs`: a
 * 3x2 rectangle at (1, 2) with black, red and white pixels, delimiters
 * included.
 */
const GOLDEN = "0004fe02010202020302020101010383f80105ffffdf3c00";

function hex(value: string): Uint8Array {
  return Uint8Array.from(value.match(/../g)!.map((pair) => parseInt(pair, 16)));
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

/** A parser that records what it passes on. */
function recorder() {
  const texts: string[] = [];
  const packets: Uint8Array[] = [];
  const parser = new DeviceStreamParser((text) => texts.push(text), (body) => packets.push(body));
  return { parser, texts, packets, text: () => texts.join("") };
}

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);

describe("checksum and COBS", () => {
  it("matches the CRC-16/CCITT-FALSE check value", () => {
    expect(crc16(utf8("123456789"))).toBe(0x29b1);
  });

  it("decodes zeros at the start, the end and between blocks", () => {
    expect(cobsDecode(Uint8Array.of(0x01, 0x01))).toEqual(Uint8Array.of(0x00));
    expect(cobsDecode(Uint8Array.of(0x03, 0x11, 0x22, 0x02, 0x33))).toEqual(Uint8Array.of(0x11, 0x22, 0x00, 0x33));
    expect(cobsDecode(Uint8Array.of(0x02, 0x00))).toBeNull();
  });
});

describe("device stream parser", () => {
  it("passes text on without waiting for a delimiter", () => {
    const { parser, texts, text } = recorder();
    parser.feed(utf8("[INFO] booting\n"));
    expect(text()).toBe("[INFO] booting\n");

    // A segment's first byte waits for the second byte, and only that byte.
    parser.feed(utf8("["));
    parser.feed(utf8("W"));
    expect(text()).toBe("[INFO] booting\n[W");
    expect(texts.length).toBeGreaterThan(1);
  });

  it("decodes the golden packet from the Rust tests", () => {
    const { parser, packets, text } = recorder();
    parser.feed(hex(GOLDEN));
    expect(text()).toBe("");
    expect(packets).toHaveLength(1);
    const packet = parseScreenBody(packets[0]);
    expect(packet).toMatchObject({ kind: "rect", x: 1, y: 2, w: 3, h: 2 });
  });

  it("separates text and packets across chunk boundaries", () => {
    const stream = concat(
      utf8("boot log\n"),
      hex(GOLDEN),
      utf8("[INFO] grüße\n"),
      hex(GOLDEN),
      hex(GOLDEN),
      utf8("[INFO] done\n"),
    );
    for (const chunkSize of [1, 2, 3, 7, 16, stream.length]) {
      const { parser, packets, text } = recorder();
      for (let at = 0; at < stream.length; at += chunkSize) parser.feed(stream.subarray(at, at + chunkSize));
      expect(text()).toBe("boot log\n[INFO] grüße\n[INFO] done\n");
      expect(packets).toHaveLength(3);
      expect(parser.droppedPackets).toBe(0);
    }
  });

  it("drops a packet with a bad CRC", () => {
    const damaged = hex(GOLDEN);
    // Turn a pixel byte 0xf8 into 0xe8. It stays non-zero, so the frame stays intact.
    damaged[16] ^= 0x10;
    const { parser, packets, text } = recorder();
    parser.feed(concat(damaged, utf8("after\n")));
    expect(packets).toHaveLength(0);
    expect(parser.droppedPackets).toBe(1);
    expect(text()).toBe("after\n");
  });

  it("treats an overlong candidate as text", () => {
    const { parser, packets, text } = recorder();
    const long = new Uint8Array(5000).fill(0x41);
    long[1] = 0xfe;
    parser.feed(long);
    parser.finish();
    expect(packets).toHaveLength(0);
    expect(text().length).toBeGreaterThan(4000);
  });

  it("starts clean after a reset", () => {
    const { parser, packets } = recorder();
    const golden = hex(GOLDEN);
    parser.feed(golden.subarray(0, 10));
    parser.reset();
    parser.feed(golden);
    expect(packets).toHaveLength(1);
  });
});
