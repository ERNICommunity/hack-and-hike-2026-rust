/**
 * Splits the serial stream of a device into log text and screen packets.
 *
 * The firmware sends both over one USB serial port. Log text is plain UTF-8.
 * A screen packet is a `0x00` byte, the COBS-encoded body and another `0x00`
 * byte. COBS (Consistent Overhead Byte Stuffing) removes every zero byte from
 * the body, so `0x00` only marks packet edges. The body starts with the magic
 * byte `0xfe`, which never occurs in UTF-8, and ends with a CRC-16. The format
 * is described in `crates/core/src/screen.rs` in the firmware repository.
 *
 * The parser splits the stream at every `0x00` into segments. A segment whose
 * second byte is `0xfe` (the COBS code byte comes first, then the magic) is a
 * packet candidate. Every other segment is text, and goes to `onText` as it
 * arrives, without waiting for the next `0x00`.
 */

import { SCREEN_MAX_PACKET_BYTES, SCREEN_PACKET_MAGIC } from "./config";

/** The byte that starts and ends a packet. */
const DELIMITER = 0x00;
/** Size of the CRC at the end of a packet body. */
const CRC_BYTES = 2;

/** Where the parser is in the current segment. */
type Mode =
  /** At the start of a segment: no byte yet. */
  | "start"
  /** One byte is held; the second byte decides between text and packet. */
  | "held"
  /** A text segment: bytes go to the text decoder at once. */
  | "text"
  /** A packet candidate: bytes are buffered until the next `0x00`. */
  | "packet";

/** CRC-16/CCITT-FALSE: polynomial 0x1021, start 0xffff, no reflection, no final XOR. */
export function crc16(data: Uint8Array): number {
  let crc = 0xffff;
  for (const byte of data) {
    crc ^= byte << 8;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
    }
  }
  return crc;
}

/** Decode COBS bytes without delimiters. Returns `null` when the bytes are not valid COBS. */
export function cobsDecode(input: Uint8Array): Uint8Array | null {
  const out = new Uint8Array(input.length);
  let read = 0;
  let write = 0;
  while (read < input.length) {
    const code = input[read];
    if (code === 0) return null;
    read += 1;
    const end = read + code - 1;
    if (end > input.length) return null;
    for (; read < end; read += 1) {
      if (input[read] === 0) return null;
      out[write] = input[read];
      write += 1;
    }
    if (code !== 0xff && read < input.length) {
      out[write] = 0;
      write += 1;
    }
  }
  return out.subarray(0, write);
}

/**
 * Check a COBS-encoded packet candidate and return its body without the CRC,
 * or `null` when it is damaged.
 */
export function decodePacket(encoded: Uint8Array): Uint8Array | null {
  const body = cobsDecode(encoded);
  if (!body || body.length < 2 + CRC_BYTES || body[0] !== SCREEN_PACKET_MAGIC) return null;
  const dataLength = body.length - CRC_BYTES;
  const expected = body[dataLength] | (body[dataLength + 1] << 8);
  if (crc16(body.subarray(0, dataLength)) !== expected) return null;
  return body.subarray(0, dataLength);
}

/** Splits the byte stream of one device into text and screen packets. */
export class DeviceStreamParser {
  /** Packet candidates that failed the COBS, CRC or magic check since the last `reset`. */
  droppedPackets = 0;

  private decoder = new TextDecoder();
  private mode: Mode = "start";
  /** The first byte of the segment while `mode` is "held". */
  private held = 0;
  /** The bytes of the current packet candidate. */
  private readonly packet = new Uint8Array(SCREEN_MAX_PACKET_BYTES);
  /** How many bytes of `packet` are used. */
  private packetLength = 0;

  /**
   * @param onText receives decoded log text, as soon as it arrives.
   * @param onPacket receives each valid packet body, without its CRC.
   */
  constructor(
    private readonly onText: (text: string) => void,
    private readonly onPacket: (body: Uint8Array) => void,
  ) {}

  /** Forget a half-received segment and start like a new stream. Call it when a serial log opens. */
  reset(): void {
    this.decoder = new TextDecoder();
    this.mode = "start";
    this.packetLength = 0;
    this.droppedPackets = 0;
  }

  /** Parse the next bytes of the stream. */
  feed(bytes: Uint8Array): void {
    // Text bytes of this chunk are passed on as whole ranges, not byte by byte.
    let textStart = -1;
    const flushText = (end: number): void => {
      if (textStart >= 0 && end > textStart) this.emitText(bytes.subarray(textStart, end));
      textStart = -1;
    };

    for (let index = 0; index < bytes.length; index += 1) {
      const byte = bytes[index];
      if (byte === DELIMITER) {
        flushText(index);
        this.endSegment();
        continue;
      }
      switch (this.mode) {
        case "start":
          this.held = byte;
          this.mode = "held";
          break;
        case "held":
          if (byte === SCREEN_PACKET_MAGIC) {
            this.packet[0] = this.held;
            this.packet[1] = byte;
            this.packetLength = 2;
            this.mode = "packet";
          } else {
            this.emitText(Uint8Array.of(this.held));
            this.mode = "text";
            textStart = index;
          }
          break;
        case "text":
          if (textStart < 0) textStart = index;
          break;
        case "packet":
          if (this.packetLength < this.packet.length) {
            this.packet[this.packetLength] = byte;
            this.packetLength += 1;
          } else {
            // Far too long for a packet: it was text after all.
            this.emitText(this.packet.slice(0, this.packetLength));
            this.packetLength = 0;
            this.mode = "text";
            textStart = index;
          }
          break;
      }
    }
    flushText(bytes.length);
  }

  /** Pass on what the parser still holds, at the end of the stream. */
  finish(): void {
    if (this.mode === "held") this.emitText(Uint8Array.of(this.held));
    if (this.mode === "packet") this.emitText(this.packet.slice(0, this.packetLength));
    const rest = this.decoder.decode();
    if (rest) this.onText(rest);
    this.mode = "start";
    this.packetLength = 0;
  }

  /** Handle a `0x00`: the current segment is complete. */
  private endSegment(): void {
    if (this.mode === "held") {
      this.emitText(Uint8Array.of(this.held));
    } else if (this.mode === "packet") {
      const body = decodePacket(this.packet.subarray(0, this.packetLength));
      if (body) {
        this.onPacket(body);
      } else {
        this.droppedPackets += 1;
      }
      this.packetLength = 0;
    }
    this.mode = "start";
  }

  /** Decode text bytes and pass them on. */
  private emitText(bytes: Uint8Array): void {
    const text = this.decoder.decode(bytes, { stream: true });
    if (text) this.onText(text);
  }
}
