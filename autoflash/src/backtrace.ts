/**
 * Decoded panic backtraces in the serial console.
 *
 * A panicking ESP firmware (esp-backtrace) prints "Backtrace:" and then one
 * code address per line; the flashed image has no debug information to name
 * them. ESP-IDF images carry the SHA-256 of the ELF file they were built from,
 * so the server can find that ELF and look the addresses up in it.
 */

/** The first byte of an ESP-IDF application image. */
const IMAGE_MAGIC = 0xe9;
/** The application descriptor follows the image and first segment headers. */
const APP_DESC_OFFSET = 0x20;
const APP_DESC_MAGIC = 0xabcd5432;
/** Where `app_elf_sha256` sits inside the application descriptor. */
const ELF_SHA256_OFFSET = 0x90;
/** Partitions, and so application images in a merged file, start on 4 KiB boundaries. */
const PARTITION_ALIGNMENT = 0x1000;
/** The most addresses one decode request may carry; the server has the same limit. */
const MAX_ADDRESSES = 64;

const ADDRESS_LINE = /^0x[0-9a-f]{1,8}$/i;
const ANSI_ESCAPE = /\x1b\[[0-9;]*m/g;

export type BacktraceLocation = { function: string; file: string; line: number };
export type BacktraceFrame = { address: string; locations: BacktraceLocation[] };
export type DecodedBacktrace = { elf: string; frames: BacktraceFrame[] };

/**
 * The hex SHA-256 of the ELF file an image was built from, read from the
 * application descriptor. Works on app-only and merged images; `undefined`
 * when there is no descriptor or the tool that made the image left it empty.
 */
export function elfSha256Of(image: Uint8Array): string | undefined {
  const view = new DataView(image.buffer, image.byteOffset, image.byteLength);
  const end = image.length - (APP_DESC_OFFSET + ELF_SHA256_OFFSET + 32);
  for (let offset = 0; offset <= end; offset += PARTITION_ALIGNMENT) {
    if (image[offset] !== IMAGE_MAGIC || view.getUint32(offset + APP_DESC_OFFSET, true) !== APP_DESC_MAGIC) {
      continue;
    }
    const start = offset + APP_DESC_OFFSET + ELF_SHA256_OFFSET;
    const digest = image.subarray(start, start + 32);
    if (digest.every((byte) => byte === 0)) return undefined;
    return [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  }
  return undefined;
}

/** An unfinished ANSI escape sequence at the end of a partial line. */
const PARTIAL_ANSI_ESCAPE = /\x1b(\[[0-9;]*)?$/;
const BACKTRACE_HEADER = "Backtrace:";
const PARTIAL_ADDRESS = /^(0(x[0-9a-f]{0,8})?)?$/i;

/** What `BacktraceCollector.push` made of received text. */
export type CollectedOutput = {
  /** The text to show: the received text without the raw backtraces. */
  display: string;
  /** The address lists of the backtraces that ended. */
  completed: string[][];
};

/**
 * Picks backtraces out of a serial stream that arrives in arbitrary chunks,
 * and removes them from the text shown: the "Backtrace:" header, the blank
 * lines around it and the bare addresses. The caller shows the decoded
 * backtrace in their place.
 *
 * A backtrace ends at the first line that is not an address. The panic
 * handler prints nothing after the last address, so the caller also ends a
 * `pending` backtrace with `finish()` once the stream has been quiet briefly.
 *
 * Blank lines, and the start of a line that could still become a header or
 * an address, are held back until it is clear whether they belong to a
 * backtrace.
 */
export class BacktraceCollector {
  /** The unfinished last line, not shown yet. */
  private partialLine = "";
  /** The start of the unfinished last line was shown: the line is ordinary output. */
  private partialLineShown = false;
  /** Blank lines that may precede a header, not shown yet. */
  private heldLines = "";
  private collecting = false;
  private addresses: string[] = [];

  /** Feed received text; returns the text to show and the backtraces it completed. */
  push(text: string): CollectedOutput {
    const output: CollectedOutput = { display: "", completed: [] };
    const lines = `${this.partialLine}${text}`.split("\n");
    this.partialLine = lines.pop() ?? "";

    for (const rawLine of lines) {
      if (this.partialLineShown) {
        output.display += `${rawLine}\n`;
        this.partialLineShown = false;
      } else {
        this.pushLine(`${rawLine}\n`, output);
      }
    }

    if (this.partialLine !== "" && !this.partialLineShown && !this.mayBeSpecial(this.partialLine)) {
      this.showOrdinary(this.partialLine, output);
      this.partialLine = "";
      this.partialLineShown = true;
    } else if (this.partialLineShown) {
      output.display += this.partialLine;
      this.partialLine = "";
    }
    return output;
  }

  private pushLine(rawLine: string, output: CollectedOutput): void {
    const line = rawLine.replace(ANSI_ESCAPE, "").trim();
    if (line === BACKTRACE_HEADER) {
      this.finishInto(output.completed);
      this.heldLines = "";
      this.collecting = true;
    } else if (line === "") {
      if (!this.collecting) this.heldLines += rawLine;
    } else if (this.collecting && ADDRESS_LINE.test(line) && this.addresses.length < MAX_ADDRESSES) {
      this.addresses.push(line);
    } else {
      this.showOrdinary(rawLine, output);
    }
  }

  /** Show `text` from an ordinary line, which ends a backtrace and releases held lines. */
  private showOrdinary(text: string, output: CollectedOutput): void {
    this.finishInto(output.completed);
    output.display += `${this.heldLines}${text}`;
    this.heldLines = "";
  }

  /** The start of a line could still turn out blank, a header or an address. */
  private mayBeSpecial(partialLine: string): boolean {
    const line = partialLine.replace(ANSI_ESCAPE, "").replace(PARTIAL_ANSI_ESCAPE, "").trim();
    return BACKTRACE_HEADER.startsWith(line) || (this.collecting && PARTIAL_ADDRESS.test(line));
  }

  /** A backtrace has addresses and has not ended yet. */
  get pending(): boolean {
    return this.addresses.length > 0;
  }

  /** End the current backtrace; its addresses, if it had any. */
  finish(): string[] | undefined {
    const addresses = this.addresses;
    this.collecting = false;
    this.addresses = [];
    return addresses.length > 0 ? addresses : undefined;
  }

  private finishInto(completed: string[][]): void {
    const addresses = this.finish();
    if (addresses) completed.push(addresses);
  }
}

/** Ask the server to look `addresses` up in the ELF file with hash `elfSha256`. */
export async function decodeBacktrace(elfSha256: string, addresses: string[]): Promise<DecodedBacktrace> {
  const query = new URLSearchParams({ elf: elfSha256, addresses: addresses.join(",") });
  const response = await fetch(`/api/backtrace?${query}`, { cache: "no-store" });
  const body = (await response.json().catch(() => undefined)) as
    | (Partial<DecodedBacktrace> & { error?: string })
    | undefined;
  if (!response.ok || !body?.frames || !body.elf) {
    throw new Error(body?.error ?? `the server answered ${response.status} ${response.statusText}`);
  }
  return { elf: body.elf, frames: body.frames };
}

/** The decoded backtrace as console text: each address with its function and source line. */
export function formatBacktrace(decoded: DecodedBacktrace): string {
  const width = Math.max(10, ...decoded.frames.map((frame) => frame.address.length));
  const indent = " ".repeat(width + 2);
  const lines = [`Decoded backtrace (${decoded.elf}):`];

  for (const frame of decoded.frames) {
    const [innermost, ...callers] = frame.locations;
    const address = frame.address.padEnd(width);
    if (!innermost) {
      lines.push(`${address}  (no debug information)`);
      continue;
    }
    lines.push(`${address}  ${innermost.function}`, `${indent}at ${place(innermost)}`);
    for (const caller of callers) {
      lines.push(`${indent}inlined into ${caller.function}`, `${indent}at ${place(caller)}`);
    }
  }
  return `${lines.join("\n")}\n`;
}

function place(location: BacktraceLocation): string {
  return location.line > 0 ? `${location.file}:${location.line}` : location.file;
}
