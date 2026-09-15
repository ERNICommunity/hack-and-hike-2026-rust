import { describe, expect, it } from "vitest";
import { BacktraceCollector, elfSha256Of, formatBacktrace } from "./backtrace";

const DIGEST = "283ce0e9c50be8a9f6529ca5288cfa32a2ed98dfdabbcf68cd32d5ef91b24269";

/** A merged image: bootloader at 0x0, application image with a descriptor at `appOffset`. */
function mergedImage(appOffset: number, digest: string | undefined): Uint8Array {
  const image = new Uint8Array(appOffset + 0x1000).fill(0xff);
  const view = new DataView(image.buffer);
  image[0] = 0xe9;
  image[appOffset] = 0xe9;
  view.setUint32(appOffset + 0x20, 0xabcd5432, true);
  const sha = image.subarray(appOffset + 0x20 + 0x90, appOffset + 0x20 + 0x90 + 32);
  sha.fill(0);
  if (digest) sha.set(digest.match(/../g)!.map((pair) => parseInt(pair, 16)));
  return image;
}

describe("ELF hash in a firmware image", () => {
  it("reads the hash from a merged image", () => {
    expect(elfSha256Of(mergedImage(0x10000, DIGEST))).toBe(DIGEST);
  });

  it("reads the hash from an app-only image", () => {
    expect(elfSha256Of(mergedImage(0, DIGEST))).toBe(DIGEST);
  });

  it("ignores images without a descriptor or with an empty hash", () => {
    expect(elfSha256Of(new Uint8Array(0x20000))).toBeUndefined();
    expect(elfSha256Of(mergedImage(0x10000, undefined))).toBeUndefined();
    expect(elfSha256Of(new Uint8Array(16))).toBeUndefined();
  });
});

describe("backtrace collector", () => {
  const panic =
    "\x1b[31m\r\n\r\n====================== PANIC ======================\r\n" +
    "panicked at src/bin/panic_backtrace.rs:89:5:\r\nindex out of bounds\r\n\x1b[0m\r\n" +
    "\r\nBacktrace:\r\n\r\n0x4209d358\r\n0x4205f584\r\n0x4205f0d8\r\n\r\n\r\n\r\n";

  const panicShown =
    "\x1b[31m\r\n\r\n====================== PANIC ======================\r\n" +
    "panicked at src/bin/panic_backtrace.rs:89:5:\r\nindex out of bounds\r\n";

  it("collects the addresses, hides them and waits for the stream to go quiet", () => {
    const collector = new BacktraceCollector();
    expect(collector.push(panic)).toEqual({ display: panicShown, completed: [] });
    expect(collector.pending).toBe(true);
    expect(collector.finish()).toEqual(["0x4209d358", "0x4205f584", "0x4205f0d8"]);
    expect(collector.pending).toBe(false);
  });

  it("handles text split across chunks and ends at the next log line", () => {
    const collector = new BacktraceCollector();
    const outputs = [...panic, "[INFO] rebooted\r\n"].map((character) => collector.push(character));
    expect(outputs.flatMap((output) => output.completed)).toEqual([["0x4209d358", "0x4205f584", "0x4205f0d8"]]);
    expect(outputs.map((output) => output.display).join("")).toBe(`${panicShown}[INFO] rebooted\r\n`);
    expect(collector.pending).toBe(false);
  });

  it("shows ordinary partial lines at once and blank lines once followed by output", () => {
    const collector = new BacktraceCollector();
    expect(collector.push("Tap a ")).toEqual({ display: "Tap a ", completed: [] });
    expect(collector.push("band\n\n")).toEqual({ display: "band\n", completed: [] });
    expect(collector.push("Back")).toEqual({ display: "", completed: [] });
    expect(collector.push("ground\n")).toEqual({ display: "\nBackground\n", completed: [] });
  });

  it("ignores addresses outside a backtrace", () => {
    const collector = new BacktraceCollector();
    expect(collector.push("0x40000000\nregister dump\nBacktrace:\n\nsomething else\n")).toEqual({
      display: "0x40000000\nregister dump\nsomething else\n",
      completed: [],
    });
    expect(collector.finish()).toBeUndefined();
  });
});

describe("backtrace formatting", () => {
  it("lists each address with its function, inlined callers and unknown frames", () => {
    const text = formatBacktrace({
      elf: "/target/release/panic_backtrace",
      frames: [
        {
          address: "0x4205f584",
          locations: [
            { function: "panic_backtrace::band_name", file: "src/bin/panic_backtrace.rs", line: 89 },
            { function: "panic_backtrace::on_tap", file: "src/bin/panic_backtrace.rs", line: 78 },
          ],
        },
        { address: "0x00000012", locations: [] },
      ],
    });
    expect(text).toBe(
      "Decoded backtrace (/target/release/panic_backtrace):\n" +
        "0x4205f584  panic_backtrace::band_name\n" +
        "            at src/bin/panic_backtrace.rs:89\n" +
        "            inlined into panic_backtrace::on_tap\n" +
        "            at src/bin/panic_backtrace.rs:78\n" +
        "0x00000012  (no debug information)\n",
    );
  });
});
