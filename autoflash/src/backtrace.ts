/**
 * Decoded panic backtraces in the device log.
 *
 * When ESP firmware panics, esp-backtrace prints "Backtrace:" and then one
 * code address per line. The flashed image has no debug information, so the
 * addresses have no names. The ELF (Executable and Linkable Format) file of
 * the build has this information. The image contains the SHA-256 hash of
 * that ELF file. With this hash, the server finds the ELF file and looks up
 * the addresses in it.
 */

/** The first byte of an ESP-IDF application image. */
const IMAGE_MAGIC = 0xe9;
/**
 * The offset of the application descriptor in an application image. It
 * comes after the image header (24 bytes) and the first segment header
 * (8 bytes).
 */
const APP_DESC_OFFSET = 0x20;
/** The first four bytes of an application descriptor, little-endian. */
const APP_DESC_MAGIC = 0xabcd5432;
/** The offset of `app_elf_sha256` inside the application descriptor. */
const ELF_SHA256_OFFSET = 0x90;
/** Partitions start at multiples of 4 KiB, so application images in a merged file do too. */
const PARTITION_ALIGNMENT = 0x1000;
/** The largest number of addresses in one decode request. The server has the same limit. */
const MAX_ADDRESSES = 64;

/** A line with only a code address: `0x` and one to eight hexadecimal digits. */
const ADDRESS_LINE = /^0x[0-9a-f]{1,8}$/i;
/** An ANSI escape sequence that sets the text colour in a terminal, such as `ESC[31m`. */
const ANSI_ESCAPE = /\x1b\[[0-9;]*m/g;

/** One source location of a frame, as the server sends it. */
export type BacktraceLocation = { function: string; file: string; line: number };
/**
 * One backtrace address and its source locations. With inlined functions,
 * there are several locations, the innermost first. The list is empty when
 * there is no debug information.
 */
export type BacktraceFrame = { address: string; locations: BacktraceLocation[] };
/** The server's answer: the path of the ELF file and one frame for each address. */
export type DecodedBacktrace = { elf: string; frames: BacktraceFrame[] };

/**
 * The SHA-256 hash of the ELF file that an image was built from, as
 * hexadecimal text. It comes from the application descriptor.
 *
 * Works with application-only images and with merged images. Returns
 * `undefined` when there is no descriptor, or when the hash in it is all
 * zeros (the tool that made the image did not fill it in).
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
/** The line that esp-backtrace prints before the addresses. */
const BACKTRACE_HEADER = "Backtrace:";
/** The start of a line that can still become an address line. */
const PARTIAL_ADDRESS = /^(0(x[0-9a-f]{0,8})?)?$/i;

/** The result of `BacktraceCollector.push` or `BacktraceCollector.end`. */
export type CollectedOutput = {
  /** The text to show: the received text without the raw backtraces. */
  display: string;
  /** The address lists of the backtraces that ended. */
  completed: string[][];
};

/**
 * Finds backtraces in the serial output and removes them from the text that
 * the log shows. The output can arrive in chunks of any size.
 *
 * The collector removes the "Backtrace:" header, the blank lines around it
 * and the address lines. The caller shows the decoded backtrace instead.
 *
 * A backtrace ends at the first line that is not an address. The panic
 * handler prints nothing after the last address. So the caller also ends a
 * `pending` backtrace with `finish()` when no output arrives for a short
 * time, and calls `end()` when the serial stream stops.
 *
 * The collector holds back blank lines and the start of a line that can
 * still become a header or an address. It shows them when it is clear that
 * they do not belong to a backtrace. A header without addresses after it is
 * shown too.
 */
export class BacktraceCollector {
  /** The unfinished last line, not shown yet. */
  private partialLine = "";
  /** True when the start of the unfinished last line was already shown, because it is ordinary output. */
  private partialLineShown = false;
  /** Blank lines that can come before a header, not shown yet. */
  private heldLines = "";
  /** The header of the current backtrace and the blank lines around it. Shown when no address follows. */
  private header = "";
  /** True after a header, until the backtrace ends. */
  private collecting = false;
  /** The addresses of the current backtrace. */
  private addresses: string[] = [];

  /** Add received text. Returns the text to show and the backtraces that this text ended. */
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

  /** Handle one complete line, including its `\n`. */
  private pushLine(rawLine: string, output: CollectedOutput): void {
    const line = rawLine.replace(ANSI_ESCAPE, "").trim();
    if (line === BACKTRACE_HEADER) {
      output.display += this.unusedHeader();
      this.finishInto(output.completed);
      this.header = `${this.heldLines}${rawLine}`;
      this.heldLines = "";
      this.collecting = true;
    } else if (line === "") {
      if (!this.collecting) this.heldLines += rawLine;
      else if (this.addresses.length === 0) this.header += rawLine;
    } else if (this.collecting && ADDRESS_LINE.test(line) && this.addresses.length < MAX_ADDRESSES) {
      this.addresses.push(line);
    } else {
      this.showOrdinary(rawLine, output);
    }
  }

  /**
   * Show `text` from an ordinary line. An ordinary line ends the current
   * backtrace, and the held blank lines are shown before it.
   */
  private showOrdinary(text: string, output: CollectedOutput): void {
    output.display += this.unusedHeader();
    this.finishInto(output.completed);
    output.display += `${this.heldLines}${text}`;
    this.heldLines = "";
  }

  /** The held header when no address followed it, so that it goes into the log; otherwise "". */
  private unusedHeader(): string {
    return this.collecting && this.addresses.length === 0 ? this.header : "";
  }

  /** Whether the start of a line can still become a blank line, a header or an address. */
  private mayBeSpecial(partialLine: string): boolean {
    const line = partialLine.replace(ANSI_ESCAPE, "").replace(PARTIAL_ANSI_ESCAPE, "").trim();
    return BACKTRACE_HEADER.startsWith(line) || (this.collecting && PARTIAL_ADDRESS.test(line));
  }

  /**
   * Call this when the serial stream stops. Returns the text that is still
   * held back, and the backtrace at the end of the stream, if any. Then the
   * collector is ready for the next stream.
   */
  end(): CollectedOutput {
    const output: CollectedOutput = { display: this.unusedHeader(), completed: [] };
    this.finishInto(output.completed);
    output.display += `${this.heldLines}${this.partialLine}`;
    this.heldLines = "";
    this.partialLine = "";
    this.partialLineShown = false;
    return output;
  }

  /** True when a backtrace has addresses and has not ended yet. */
  get pending(): boolean {
    return this.addresses.length > 0;
  }

  /**
   * End the current backtrace. Returns its addresses, or `undefined` when it
   * has none. A held header without addresses is dropped, so call this only
   * when `pending` is true.
   */
  finish(): string[] | undefined {
    const addresses = this.addresses;
    this.collecting = false;
    this.header = "";
    this.addresses = [];
    return addresses.length > 0 ? addresses : undefined;
  }

  /** End the current backtrace, and add its addresses to `completed` if it has any. */
  private finishInto(completed: string[][]): void {
    const addresses = this.finish();
    if (addresses) completed.push(addresses);
  }
}

/**
 * Ask the server to look up `addresses` in the ELF file with the hash
 * `elfSha256`. Throws an `Error` with a short reason when decoding fails.
 */
export async function decodeBacktrace(elfSha256: string, addresses: string[]): Promise<DecodedBacktrace> {
  const query = new URLSearchParams({ elf: elfSha256, addresses: addresses.join(",") });
  const response = await fetch(`/api/backtrace?${query}`, { cache: "no-store" });
  // Only the Rust server has this path. The Vite dev server and the Cloudflare
  // Worker answer unknown paths with the page (HTML), not with JSON.
  if (!response.headers.get("Content-Type")?.includes("application/json")) {
    throw new Error("this server does not decode backtraces; run the Rust server from autoflash/ where the firmware is built");
  }
  const body =(await response.json().catch(() => undefined)) as
    | (Partial<DecodedBacktrace> & { error?: string })
    | undefined;
  if (!response.ok || !body?.frames || !body.elf) {
    throw new Error(body?.error ?? `the server answered ${response.status} ${response.statusText}`);
  }
  return { elf: body.elf, frames: body.frames };
}

/** The decoded backtrace as text for the device log: each address with its function and source line. */
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

/** `file:line`, or only the file when the line is unknown (0). */
function place(location: BacktraceLocation): string {
  return location.line > 0 ? `${location.file}:${location.line}` : location.file;
}
