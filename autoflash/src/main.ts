import { ESPLoader, Transport } from "esptool-js";
import {
  FILE_POLL_INTERVAL_MS,
  FILE_STABLE_FOR_MS,
  FLASH_ADDRESS,
  FLASH_BAUD_RATE,
  MAX_LOG_CHARACTERS,
  MONITOR_BAUD_RATE,
  SERIAL_PORT_SEARCH,
} from "./config";
import {
  formatPortInfo,
  parseDeviceSearch,
  portMatchesSearch,
  requestPortFilters,
} from "./device-search";
import { loadFirmwareHandle, saveFirmwareHandle } from "./file-store";
import "./styles.css";

type ConnectionState = "searching" | "needs-permission" | "connected" | "flashing" | "error";
type FileState = "empty" | "permission" | "watching" | "settling" | "paused";

const deviceSearch = parseDeviceSearch(SERIAL_PORT_SEARCH);
const FINAL_FLASH_WRITE_LINE = /^Writing at 0x[0-9a-f]+\.\.\. \(100%\)$/i;

document.querySelector<HTMLDivElement>("#app")!.innerHTML = `
  <div class="page-shell">
    <main>
      <section class="workspace" aria-label="ESP AutoFlash controls and serial monitor">
        <div class="controls-column">
          <article class="control-card" id="device-card">
            <div class="card-heading">
              <span class="step-number">01</span>
              <div>
                <p class="card-kicker">Target</p>
                <h2>Serial device</h2>
              </div>
              <span class="state-chip searching" id="device-state">Searching</span>
            </div>
            <div class="target-row">
              <div class="target-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24"><path d="M8 6V3m8 3V3M7 10h10m-9 8h8a3 3 0 0 0 3-3V9a3 3 0 0 0-3-3H8a3 3 0 0 0-3 3v6a3 3 0 0 0 3 3Zm4 0v3"/></svg>
              </div>
              <div class="target-copy">
                <strong id="device-name">Looking for an authorized port…</strong>
                <span>Match <code>${SERIAL_PORT_SEARCH}</code> · ${MONITOR_BAUD_RATE.toLocaleString()} baud monitor</span>
              </div>
            </div>
            <div class="button-row">
              <button class="button primary" id="authorize-button" type="button">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14m-7-7h14"/></svg>
                Authorize device
              </button>
              <button class="button secondary" id="reset-button" type="button" disabled>
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6"/></svg>
                Reset
              </button>
            </div>
          </article>

          <article class="control-card" id="firmware-card">
            <div class="card-heading">
              <span class="step-number">02</span>
              <div>
                <p class="card-kicker">Source</p>
                <h2>Firmware file</h2>
              </div>
              <span class="state-chip empty" id="file-state">Not selected</span>
            </div>

            <button class="file-picker" id="file-picker" type="button">
              <span class="file-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24"><path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7Zm0 0v5h5M9 13h6m-6 4h4"/></svg>
              </span>
              <span class="file-picker-copy">
                <strong id="file-name">Choose a .bin file</strong>
                <small id="file-meta">A persistent file handle enables change detection</small>
              </span>
              <span class="file-picker-action">Browse</span>
            </button>
            <button class="text-button hidden" id="choose-another-button" type="button">Choose a different file</button>
          </article>

          <article class="control-card automation-card">
            <div class="card-heading">
              <span class="step-number accent">03</span>
              <div>
                <p class="card-kicker">Loop</p>
                <h2>Automatic flashing</h2>
              </div>
              <label class="switch" aria-label="Watch file changes">
                <input id="watch-toggle" type="checkbox" checked />
                <span></span>
              </label>
            </div>

            <div class="loop-path" aria-label="Automatic workflow">
              <div><span class="loop-node">1</span><small>Detect</small></div>
              <span class="loop-line"></span>
              <div><span class="loop-node">2</span><small>Flash</small></div>
              <span class="loop-line"></span>
              <div><span class="loop-node">3</span><small>Reset</small></div>
              <span class="loop-line"></span>
              <div><span class="loop-node">4</span><small>Monitor</small></div>
            </div>

            <div class="progress-block" id="progress-block">
              <div class="progress-label">
                <span id="progress-text">Waiting for a file change</span>
                <strong id="progress-percent">—</strong>
              </div>
              <div class="progress-track"><span id="progress-bar"></span></div>
            </div>

            <button class="button flash-button" id="flash-button" type="button" disabled>
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m13 2-9 12h7l-1 8 9-12h-7Z"/></svg>
              Flash latest now
            </button>
          </article>
        </div>

        <article class="terminal-card">
          <div class="terminal-header">
            <div class="terminal-title">
              <span class="terminal-glyph">›_</span>
              <div>
                <p id="terminal-status">Waiting for port</p>
                <h2>Serial monitor</h2>
              </div>
            </div>
            <div class="terminal-actions">
              <label class="autoscroll-control">
                <input type="checkbox" id="autoscroll-toggle" checked />
                <span>Auto-scroll</span>
              </label>
              <button class="icon-button" id="download-log-button" type="button" aria-label="Download log" title="Download log">
                <svg viewBox="0 0 24 24"><path d="M12 3v12m0 0 5-5m-5 5-5-5M5 21h14"/></svg>
              </button>
              <button class="icon-button" id="clear-log-button" type="button" aria-label="Clear log" title="Clear log">
                <svg viewBox="0 0 24 24"><path d="M4 7h16M10 11v6m4-6v6M9 7l1-3h4l1 3m3 0-1 14H7L6 7"/></svg>
              </button>
            </div>
          </div>
          <div class="terminal-body" id="terminal-body" role="log" aria-live="polite" aria-label="Serial output">
            <pre id="terminal-output"></pre>
          </div>
        </article>
      </section>
    </main>
  </div>
`;

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`Missing element #${id}`);
  return found as T;
}

const ui = {
  deviceState: element("device-state"),
  deviceName: element("device-name"),
  authorizeButton: element<HTMLButtonElement>("authorize-button"),
  resetButton: element<HTMLButtonElement>("reset-button"),
  fileState: element("file-state"),
  filePicker: element<HTMLButtonElement>("file-picker"),
  fileName: element("file-name"),
  fileMeta: element("file-meta"),
  chooseAnotherButton: element<HTMLButtonElement>("choose-another-button"),
  watchToggle: element<HTMLInputElement>("watch-toggle"),
  progressBlock: element("progress-block"),
  progressText: element("progress-text"),
  progressPercent: element("progress-percent"),
  progressBar: element("progress-bar"),
  flashButton: element<HTMLButtonElement>("flash-button"),
  terminalBody: element("terminal-body"),
  terminalOutput: element<HTMLPreElement>("terminal-output"),
  terminalStatus: element("terminal-status"),
  autoscrollToggle: element<HTMLInputElement>("autoscroll-toggle"),
  clearLogButton: element<HTMLButtonElement>("clear-log-button"),
  downloadLogButton: element<HTMLButtonElement>("download-log-button"),
};

let port: SerialPort | undefined;
let monitorReader: ReadableStreamDefaultReader<Uint8Array> | undefined;
let monitorLoop: Promise<void> | undefined;
let stopRequested = false;
let firmwareHandle: FileSystemFileHandle | undefined;
let restoredHandle: FileSystemFileHandle | undefined;
let observedFileSignature: string | undefined;
let settlingSignature: string | undefined;
let settlingSince = 0;
let watcherTimer: number | undefined;
let flashing = false;
let queuedFlash = false;
let queuedFlashReason: "change" | "manual" = "change";
let deviceState: ConnectionState = "searching";
let fileState: FileState = "empty";

const sleep = (milliseconds: number) => new Promise<void>((resolve) => window.setTimeout(resolve, milliseconds));

function errorMessage(error: unknown): string {
  if (error instanceof DOMException && error.name === "NotFoundError") return "Selection cancelled.";
  if (error instanceof Error) return error.message;
  return String(error);
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${bytes} B`;
  if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KB`;
  return `${(bytes / 1_048_576).toFixed(2)} MB`;
}

function signatureOf(file: File): string {
  return `${file.lastModified}:${file.size}`;
}

function clockTime(): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(new Date());
}

function appendOutput(value: string): void {
  const next = `${ui.terminalOutput.textContent ?? ""}${value}`;
  ui.terminalOutput.textContent = next.length > MAX_LOG_CHARACTERS
    ? `… log trimmed …\n${next.slice(-MAX_LOG_CHARACTERS)}`
    : next;
  if (ui.autoscrollToggle.checked) ui.terminalBody.scrollTop = ui.terminalBody.scrollHeight;
}

function appendSystem(message: string, level: "info" | "success" | "error" = "info"): void {
  const marker = level === "success" ? "✓" : level === "error" ? "!" : "›";
  appendOutput(`\n[${clockTime()}] ${marker} ${message}\n`);
}

function setConnectionState(state: ConnectionState, detail?: string): void {
  deviceState = state;
  const labels: Record<ConnectionState, string> = {
    searching: "Searching",
    "needs-permission": "Setup required",
    connected: "Connected",
    flashing: "Flashing",
    error: "Attention",
  };
  const headerLabels: Record<ConnectionState, string> = {
    searching: "Finding device",
    "needs-permission": "Device permission needed",
    connected: "Serial monitor live",
    flashing: "Writing firmware",
    error: "Device needs attention",
  };

  ui.deviceState.className = `state-chip ${state}`;
  ui.deviceState.textContent = labels[state];
  ui.terminalStatus.textContent = detail ?? headerLabels[state];
  ui.resetButton.disabled = !port || flashing || state === "searching";
  ui.authorizeButton.classList.toggle("hidden", state === "connected" || state === "flashing");
  refreshActionAvailability();
}

function setFileState(state: FileState): void {
  fileState = state;
  const labels: Record<FileState, string> = {
    empty: "Not selected",
    permission: "Permission needed",
    watching: "Watching",
    settling: "Change detected",
    paused: "Paused",
  };
  ui.fileState.className = `state-chip ${state}`;
  ui.fileState.textContent = labels[state];
}

function refreshActionAvailability(): void {
  ui.flashButton.disabled = !firmwareHandle || !port || flashing;
  ui.filePicker.disabled = flashing;
  ui.chooseAnotherButton.disabled = flashing;
  ui.watchToggle.disabled = !firmwareHandle || flashing;
}

function updateProgress(percent: number | undefined, message: string): void {
  ui.progressText.textContent = message;
  ui.progressPercent.textContent = percent === undefined ? "—" : `${Math.round(percent)}%`;
  ui.progressBar.style.width = percent === undefined ? "0%" : `${Math.max(0, Math.min(100, percent))}%`;
  ui.progressBlock.classList.toggle("active", percent !== undefined);
}

async function startMonitor(allowDuringFlash = false): Promise<void> {
  if (!port || (flashing && !allowDuringFlash) || monitorLoop) return;

  if (!port.readable) {
    await port.open({ baudRate: MONITOR_BAUD_RATE, bufferSize: 65_536 });
  }
  if (!port.readable) throw new Error("The serial port did not provide a readable stream.");

  stopRequested = false;
  const activePort = port;
  const readable = activePort.readable;
  if (!readable) throw new Error("The serial port did not provide a readable stream.");
  const decoder = new TextDecoder();
  monitorReader = readable.getReader();
  const reader = monitorReader;
  setConnectionState("connected", "Receiving serial output");
  appendSystem(`Serial monitor opened at ${MONITOR_BAUD_RATE.toLocaleString()} baud.`, "success");

  monitorLoop = (async () => {
    try {
      while (!stopRequested) {
        const { value, done } = await reader.read();
        if (done) break;
        if (value) appendOutput(decoder.decode(value, { stream: true }));
      }
      const remainder = decoder.decode();
      if (remainder) appendOutput(remainder);
    } catch (error) {
      if (!stopRequested) {
        appendSystem(`Serial monitor stopped: ${errorMessage(error)}`, "error");
        setConnectionState("error", "Serial stream stopped");
      }
    } finally {
      reader.releaseLock();
      if (monitorReader === reader) monitorReader = undefined;
    }
  })().finally(() => {
    monitorLoop = undefined;
  });
}

async function stopMonitor(closePort = true): Promise<void> {
  stopRequested = true;
  const reader = monitorReader;
  if (reader) {
    try {
      await reader.cancel();
    } catch {
      // A physical disconnect can invalidate the reader before cancellation.
    }
  }
  if (monitorLoop) await monitorLoop;
  if (closePort && port?.readable) {
    try {
      await port.close();
    } catch (error) {
      appendSystem(`Could not close serial monitor cleanly: ${errorMessage(error)}`, "error");
    }
  }
}

async function connectPort(interactive: boolean): Promise<void> {
  if (!("serial" in navigator)) throw new Error("Web Serial is not supported by this browser.");

  setConnectionState("searching");
  let selected: SerialPort | undefined;
  if (interactive) {
    const filters = requestPortFilters(deviceSearch);
    selected = filters
      ? await navigator.serial.requestPort({ filters })
      : await navigator.serial.requestPort();
  } else {
    const authorized = await navigator.serial.getPorts();
    selected = authorized.find((candidate) => portMatchesSearch(candidate.getInfo(), deviceSearch));
  }

  if (!selected) {
    ui.deviceName.textContent = `No authorized ${SERIAL_PORT_SEARCH} device found`;
    setConnectionState("needs-permission");
    return;
  }

  if (port && port !== selected) await stopMonitor(true);
  port = selected;
  const portId = formatPortInfo(port.getInfo());
  ui.deviceName.textContent = `USB serial ${portId}`;
  appendSystem(`${interactive ? "Authorized" : "Found"} device ${portId}.`, "success");
  await startMonitor();
}

async function resetDevice(announce = true): Promise<void> {
  if (!port) throw new Error("No serial device is connected.");
  if (!port.readable) await startMonitor();
  if (announce) appendSystem("Resetting device via RTS…");

  // RTS drives EN low on standard Espressif auto-reset circuits. Holding DTR
  // inactive keeps GPIO0 high so the chip returns to the application, not ROM download mode.
  await port.setSignals({ dataTerminalReady: false, requestToSend: true });
  await sleep(120);
  await port.setSignals({ dataTerminalReady: false, requestToSend: false });
  await sleep(100);
  if (announce) appendSystem("Reset released; serial monitor is running.", "success");
}

async function chooseFirmware(): Promise<void> {
  if (!window.showOpenFilePicker) throw new Error("Live file watching requires Chrome or Edge with the File System Access API.");
  const [handle] = await window.showOpenFilePicker({
    multiple: false,
    excludeAcceptAllOption: false,
    types: [{ description: "ESP firmware binary", accept: { "application/octet-stream": [".bin"] } }],
  });
  if (!handle) return;
  await saveFirmwareHandle(handle);
  restoredHandle = undefined;
  await attachFirmware(handle);
}

async function attachFirmware(handle: FileSystemFileHandle): Promise<void> {
  const file = await handle.getFile();
  firmwareHandle = handle;
  observedFileSignature = signatureOf(file);
  settlingSignature = undefined;
  ui.fileName.textContent = file.name;
  ui.fileMeta.textContent = `${formatBytes(file.size)} · modified ${new Date(file.lastModified).toLocaleTimeString()}`;
  ui.filePicker.querySelector<HTMLElement>(".file-picker-action")!.textContent = "Change";
  ui.chooseAnotherButton.classList.add("hidden");
  setFileState(ui.watchToggle.checked ? "watching" : "paused");
  updateProgress(undefined, ui.watchToggle.checked ? "Watching for a stable file change" : "File watching is paused");
  appendSystem(`Watching ${file.name} (${formatBytes(file.size)}).`, "success");
  refreshActionAvailability();
  startWatcher();
}

async function resumeFirmwarePermission(): Promise<void> {
  if (!restoredHandle?.requestPermission) {
    await chooseFirmware();
    return;
  }
  const permission = await restoredHandle.requestPermission({ mode: "read" });
  if (permission !== "granted") {
    appendSystem("File permission was not granted. Choose the firmware again to continue.", "error");
    ui.chooseAnotherButton.classList.remove("hidden");
    return;
  }
  await attachFirmware(restoredHandle);
  restoredHandle = undefined;
}

function startWatcher(): void {
  if (watcherTimer !== undefined) window.clearInterval(watcherTimer);
  watcherTimer = window.setInterval(() => void pollFirmware(), FILE_POLL_INTERVAL_MS);
}

async function pollFirmware(): Promise<void> {
  if (!firmwareHandle || !ui.watchToggle.checked) return;
  try {
    const file = await firmwareHandle.getFile();
    const signature = signatureOf(file);

    if (signature === observedFileSignature) {
      if (settlingSignature === signature && Date.now() - settlingSince >= FILE_STABLE_FOR_MS) {
        settlingSignature = undefined;
        setFileState("watching");
        queueFlash("change");
      }
      return;
    }

    observedFileSignature = signature;
    settlingSignature = signature;
    settlingSince = Date.now();
    setFileState("settling");
    ui.fileMeta.textContent = `${formatBytes(file.size)} · change settling…`;
    updateProgress(undefined, "Change detected — waiting for the file to settle");
  } catch (error) {
    setFileState("permission");
    appendSystem(`Cannot read firmware: ${errorMessage(error)}`, "error");
  }
}

function queueFlash(reason: "change" | "manual"): void {
  queuedFlash = true;
  queuedFlashReason = reason;
  if (flashing) {
    updateProgress(undefined, "Newer firmware queued after the current flash");
    return;
  }
  void runFlashQueue();
}

async function runFlashQueue(): Promise<void> {
  if (flashing) return;
  flashing = true;
  refreshActionAvailability();

  try {
    while (queuedFlash) {
      const reason = queuedFlashReason;
      queuedFlash = false;
      await flashLatestFirmware(reason);
    }
  } catch (error) {
    queuedFlash = false;
    appendSystem(`Flash failed: ${errorMessage(error)}`, "error");
    updateProgress(undefined, "Flash failed — inspect the serial output");
    setConnectionState("error", "Flash failed");
  } finally {
    flashing = false;
    refreshActionAvailability();
    if (port && deviceState !== "connected") {
      try {
        await startMonitor();
      } catch (error) {
        appendSystem(`Could not restore monitor: ${errorMessage(error)}`, "error");
      }
    }
  }
}

async function flashLatestFirmware(reason: "change" | "manual"): Promise<void> {
  if (!firmwareHandle) throw new Error("Choose a firmware file first.");
  if (!port) throw new Error("Connect the serial device first.");

  const file = await firmwareHandle.getFile();
  if (file.size === 0) throw new Error("The selected firmware file is empty.");
  const image = new Uint8Array(await file.arrayBuffer());
  ui.fileMeta.textContent = `${formatBytes(file.size)} · loaded ${new Date().toLocaleTimeString()}`;
  setConnectionState("flashing", "Entering bootloader");
  updateProgress(0, `${reason === "change" ? "Change stable" : "Manual flash"} · loading ${file.name}`);
  appendSystem(`${reason === "change" ? "File changed" : "Manual flash"}: writing ${file.name} (${formatBytes(file.size)}) at 0x${FLASH_ADDRESS.toString(16)}.`);

  await stopMonitor(true);
  const activePort = port;
  const transport = new Transport(activePort, false);
  let transportOpen = false;
  let finalWriteLineSeen = false;
  let resolveFinalWriteLine!: () => void;
  const finalWriteLine = new Promise<void>((resolve) => {
    resolveFinalWriteLine = resolve;
  });

  const handleFlashLogLine = (data: string): void => {
    appendOutput(`${data}\n`);
    if (!finalWriteLineSeen && FINAL_FLASH_WRITE_LINE.test(data.trim())) {
      finalWriteLineSeen = true;
      resolveFinalWriteLine();
    }
  };

  const loader = new ESPLoader({
    transport,
    baudrate: FLASH_BAUD_RATE,
    debugLogging: false,
    terminal: {
      clean: () => undefined,
      write: (data: string) => appendOutput(data),
      writeLine: handleFlashLogLine,
    },
  });

  try {
    const chip = await loader.main();
    transportOpen = true;
    appendSystem(`Bootloader connected: ${chip}.`, "success");
    setConnectionState("flashing", "Writing flash at 0x0");

    const writeTask = loader.writeFlash({
      fileArray: [{ data: image, address: FLASH_ADDRESS }],
      flashMode: "keep",
      flashFreq: "keep",
      flashSize: "keep",
      eraseAll: false,
      compress: true,
      reportProgress: (_fileIndex, written, total) => {
        const percent = total === 0 ? 0 : (written / total) * 100;
        updateProgress(percent, `Writing ${formatBytes(written)} of ${formatBytes(total)}`);
      },
    });

    const completion = await Promise.race([
      finalWriteLine.then(() => "final-write-line" as const),
      writeTask.then(() => "write-finished" as const),
    ]);

    if (completion === "final-write-line") {
      // The requested handoff intentionally stops waiting for writeFlash as soon
      // as esptool-js prints its final 100% write line. Closing the transport below
      // ends any remaining flasher work while preventing an unhandled rejection.
      void writeTask.catch(() => undefined);
      updateProgress(100, "100% reached — restoring serial monitor");
      appendSystem("100% write line received. Reopening the serial monitor now…", "success");
    } else {
      updateProgress(100, "Firmware written — restoring serial monitor");
      appendSystem("Firmware written. Reopening the serial monitor…", "success");
    }
  } finally {
    if (transportOpen || activePort.readable) {
      try {
        await transport.disconnect();
      } catch {
        // The error from the actual flash operation is more useful than a cleanup error.
      }
    }
  }

  await startMonitor(true);
  await resetDevice(false);
  setConnectionState("connected", "Flash complete · monitor live");
  setFileState(ui.watchToggle.checked ? "watching" : "paused");
  updateProgress(undefined, "Flash complete — watching for the next change");
  appendSystem("Device reset; live serial output resumed.", "success");
}

async function restoreFirmware(): Promise<void> {
  try {
    const handle = await loadFirmwareHandle();
    if (!handle) return;
    const permission = handle.queryPermission
      ? await handle.queryPermission({ mode: "read" })
      : "prompt";
    if (permission === "granted") {
      await attachFirmware(handle);
      return;
    }

    restoredHandle = handle;
    setFileState("permission");
    ui.fileName.textContent = handle.name;
    ui.fileMeta.textContent = "Click to resume read access";
    ui.filePicker.querySelector<HTMLElement>(".file-picker-action")!.textContent = "Resume";
    ui.chooseAnotherButton.classList.remove("hidden");
  } catch (error) {
    appendSystem(`Could not restore firmware selection: ${errorMessage(error)}`, "error");
  }
}

function showUnsupportedBrowser(): void {
  ui.authorizeButton.disabled = true;
  ui.filePicker.disabled = true;
  appendSystem("Unsupported browser. Open this site in current Chrome or Edge on desktop; Web Serial and persistent file handles are required.", "error");
  setConnectionState("error", "Browser APIs unavailable");
}

ui.authorizeButton.addEventListener("click", () => {
  void connectPort(true).catch((error) => {
    if (!(error instanceof DOMException && error.name === "NotFoundError")) {
      appendSystem(`Connection failed: ${errorMessage(error)}`, "error");
      setConnectionState("error", "Connection failed");
    }
  });
});

ui.resetButton.addEventListener("click", () => {
  void resetDevice().catch((error) => appendSystem(`Reset failed: ${errorMessage(error)}`, "error"));
});

ui.filePicker.addEventListener("click", () => {
  const action = restoredHandle ? resumeFirmwarePermission() : chooseFirmware();
  void action.catch((error) => {
    if (!(error instanceof DOMException && error.name === "AbortError")) {
      appendSystem(`File selection failed: ${errorMessage(error)}`, "error");
    }
  });
});

ui.chooseAnotherButton.addEventListener("click", () => {
  restoredHandle = undefined;
  void chooseFirmware().catch((error) => {
    if (!(error instanceof DOMException && error.name === "AbortError")) {
      appendSystem(`File selection failed: ${errorMessage(error)}`, "error");
    }
  });
});

ui.watchToggle.addEventListener("change", () => {
  if (!firmwareHandle) return;
  settlingSignature = undefined;
  setFileState(ui.watchToggle.checked ? "watching" : "paused");
  updateProgress(undefined, ui.watchToggle.checked ? "Watching for a stable file change" : "File watching is paused");
  appendSystem(`Automatic flashing ${ui.watchToggle.checked ? "enabled" : "paused"}.`);
});

ui.flashButton.addEventListener("click", () => queueFlash("manual"));
ui.autoscrollToggle.addEventListener("change", () => {
  if (ui.autoscrollToggle.checked) ui.terminalBody.scrollTop = ui.terminalBody.scrollHeight;
});
ui.clearLogButton.addEventListener("click", () => { ui.terminalOutput.textContent = ""; });
ui.downloadLogButton.addEventListener("click", () => {
  const blob = new Blob([ui.terminalOutput.textContent ?? ""], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `esp-autoflash-${new Date().toISOString().replace(/[:.]/g, "-")}.log`;
  link.click();
  URL.revokeObjectURL(url);
});

if ("serial" in navigator) {
  navigator.serial.addEventListener("connect", () => {
    if (!port) void connectPort(false).catch(() => undefined);
  });
  navigator.serial.addEventListener("disconnect", (event) => {
    if (port && event.target === port) {
      stopRequested = true;
      port = undefined;
      ui.deviceName.textContent = "Device disconnected";
      appendSystem("Serial device disconnected.", "error");
      setConnectionState("needs-permission", "Device disconnected");
    }
  });
}

async function initialize(): Promise<void> {
  appendSystem(`Ready. Target is ${SERIAL_PORT_SEARCH}; firmware address is 0x${FLASH_ADDRESS.toString(16)}.`);
  if (!("serial" in navigator) || !window.showOpenFilePicker || !window.isSecureContext) {
    showUnsupportedBrowser();
    return;
  }
  await Promise.all([
    connectPort(false).catch((error) => {
      appendSystem(`Automatic connection failed: ${errorMessage(error)}`, "error");
      setConnectionState("needs-permission");
    }),
    restoreFirmware(),
  ]);
}

void initialize();
