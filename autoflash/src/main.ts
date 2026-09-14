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
import "./multi-device.css";

type ConnectionState = "searching" | "needs-permission" | "connected" | "flashing" | "error";
type FileState = "empty" | "permission" | "watching" | "settling" | "queued" | "paused";
type LogLevel = "info" | "success" | "error";

type TransportInternals = {
  buffer: Uint8Array;
  reader?: ReadableStreamDefaultReader<Uint8Array>;
  onDeviceLostCallback: (() => void) | null;
  detectPanicHandler(input: Uint8Array): void;
  SLIP_END: number;
  SLIP_ESC: number;
  SLIP_ESC_END: number;
  SLIP_ESC_ESC: number;
};

type DeviceSession = {
  port: SerialPort;
  number: number;
  portId: string;
  connected: boolean;
  state: ConnectionState;
  status: string;
  log: string;
  monitorOpened: boolean;
  disconnectedAt?: number;
  monitorReader?: ReadableStreamDefaultReader<Uint8Array>;
  monitorStart?: Promise<void>;
  monitorLoop?: Promise<void>;
  stopRequested: boolean;
};

/**
 * esptool-js 0.6.1 polls its receive buffer with a 1 ms setTimeout. Chromium
 * throttles that timer in background tabs, which can make a valid stub reply
 * look like a serial timeout. This transport wakes reads when Web Serial
 * delivers bytes instead, while keeping esptool-js's SLIP parsing behavior.
 */
class EventDrivenTransport extends Transport {
  private readonly dataWaiters = new Set<() => void>();

  private get internals(): TransportInternals {
    return this as unknown as TransportInternals;
  }

  private signalData(): void {
    for (const wake of this.dataWaiters) wake();
    this.dataWaiters.clear();
  }

  private waitForData(timeout: number): Promise<boolean> {
    if (this.internals.buffer.length > 0) return Promise.resolve(true);

    return new Promise<boolean>((resolve) => {
      let finished = false;
      const finish = (available: boolean): void => {
        if (finished) return;
        finished = true;
        window.clearTimeout(timeoutId);
        this.dataWaiters.delete(wake);
        resolve(available);
      };
      const wake = (): void => finish(this.internals.buffer.length > 0);
      const timeoutId = window.setTimeout(() => finish(false), timeout);
      this.dataWaiters.add(wake);

      // Avoid missing bytes delivered between the initial check and waiter setup.
      if (this.internals.buffer.length > 0) wake();
    });
  }

  override async readLoop(): Promise<void> {
    const state = this.internals;

    while (this.device.readable) {
      const reader = this.device.readable.getReader();
      state.reader = reader;
      try {
        const { value, done } = await reader.read();
        if (done) break;
        if (value?.length) {
          state.buffer = this.appendArray(state.buffer, Uint8Array.from(value));
          this.signalData();
        }
      } catch (error) {
        if (error instanceof DOMException) {
          state.onDeviceLostCallback?.();
          break;
        }
        if (error instanceof Error) {
          const recoverable = ["BufferOverrunError", "FramingError", "BreakError", "ParityError"];
          if (recoverable.includes(error.name)) continue;
        }
        break;
      } finally {
        reader.releaseLock();
        if (state.reader === reader) state.reader = undefined;
      }
    }

    // Release a pending read immediately if the stream ends.
    this.signalData();
  }

  override async read(timeout: number): Promise<Uint8Array> {
    const state = this.internals;
    let partialPacket: Uint8Array | null = null;
    let isEscaping = false;

    while (true) {
      await this.waitForData(timeout);
      const readBytes = state.buffer;
      state.buffer = new Uint8Array(0);

      if (readBytes.length === 0) {
        throw new Error(partialPacket === null
          ? "Serial data stream stopped: Possible serial noise or corruption."
          : "No serial data received.");
      }

      for (let index = 0; index < readBytes.length; index += 1) {
        const byte = readBytes[index];
        if (partialPacket === null) {
          if (byte === state.SLIP_END) {
            partialPacket = new Uint8Array(0);
          } else {
            const remainingData = state.buffer;
            state.detectPanicHandler(new Uint8Array([...readBytes, ...remainingData]));
            throw new Error(`Invalid head of packet (0x${byte.toString(16)}): Possible serial noise or corruption.`);
          }
        } else if (isEscaping) {
          isEscaping = false;
          if (byte === state.SLIP_ESC_END) {
            partialPacket = this.appendArray(partialPacket, new Uint8Array([state.SLIP_END]));
          } else if (byte === state.SLIP_ESC_ESC) {
            partialPacket = this.appendArray(partialPacket, new Uint8Array([state.SLIP_ESC]));
          } else {
            const remainingData = state.buffer;
            state.detectPanicHandler(new Uint8Array([...readBytes, ...remainingData]));
            throw new Error(`Invalid SLIP escape (0xdb, 0x${byte.toString(16)})`);
          }
        } else if (byte === state.SLIP_ESC) {
          isEscaping = true;
        } else if (byte === state.SLIP_END) {
          if (index + 1 < readBytes.length) {
            state.buffer = this.appendArray(readBytes.slice(index + 1), state.buffer);
          }
          return partialPacket;
        } else {
          partialPacket = this.appendArray(partialPacket, new Uint8Array([byte]));
        }
      }
    }
  }
}

const deviceSearch = parseDeviceSearch(SERIAL_PORT_SEARCH);
const FINAL_FLASH_WRITE_LINE = /^Writing at 0x[0-9a-f]+\.\.\. \(100%\)$/i;
const RECONNECT_RETRY_DELAY_MS = 250;
const RECONNECT_EVENT_DELAY_MS = 0;

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
                <h2>Serial devices</h2>
              </div>
              <span class="state-chip searching" id="device-state">Searching</span>
            </div>
            <div class="target-row">
              <div class="target-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24"><path d="M8 6V3m8 3V3M7 10h10m-9 8h8a3 3 0 0 0 3-3V9a3 3 0 0 0-3-3H8a3 3 0 0 0-3 3v6a3 3 0 0 0 3 3Zm4 0v3"/></svg>
              </div>
              <div class="target-copy">
                <strong id="device-name">Looking for authorized ports…</strong>
                <span>Match <code>${SERIAL_PORT_SEARCH}</code> · ${MONITOR_BAUD_RATE.toLocaleString()} baud monitor</span>
              </div>
            </div>
            <div class="button-row device-actions">
              <button class="button primary" id="authorize-button" type="button">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14m-7-7h14"/></svg>
                Authorize device
              </button>
              <button class="button secondary" id="reset-button" type="button" disabled title="Select a connected device tab to reset it">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6"/></svg>
                Reset selected
              </button>
              <button class="button secondary remove-device" id="remove-device-button" type="button" disabled title="Select a device tab to remove it">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"/></svg>
                Remove selected
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

            <button class="button flash-button" id="flash-button" type="button" disabled title="Flash all connected devices">
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
              <label class="autoscroll-control" title="Clear each device log after its serial monitor reconnects">
                <input type="checkbox" id="clear-on-reconnect-toggle" checked />
                <span>Clear on reconnect</span>
              </label>
              <button class="icon-button" id="copy-log-button" type="button" aria-label="Copy active log" title="Copy active log">
                <svg viewBox="0 0 24 24"><rect x="9" y="9" width="11" height="11" rx="1"/><path d="M15 9V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v9a1 1 0 0 0 1 1h4"/></svg>
              </button>
              <button class="icon-button" id="download-log-button" type="button" aria-label="Download active log" title="Download active log">
                <svg viewBox="0 0 24 24"><path d="M12 3v12m0 0 5-5m-5 5-5-5M5 21h14"/></svg>
              </button>
              <button class="icon-button" id="clear-log-button" type="button" aria-label="Clear active log" title="Clear active log">
                <svg viewBox="0 0 24 24"><path d="M4 7h16M10 11v6m4-6v6M9 7l1-3h4l1 3m3 0-1 14H7L6 7"/></svg>
              </button>
            </div>
          </div>
          <div class="terminal-tabs" id="terminal-tabs" role="tablist" aria-label="Device and Autoflash logs"></div>
          <div class="terminal-body" id="terminal-body" role="tabpanel" aria-live="polite" aria-label="Serial output">
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
  removeDeviceButton: element<HTMLButtonElement>("remove-device-button"),
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
  terminalTabs: element("terminal-tabs"),
  terminalBody: element("terminal-body"),
  terminalOutput: element<HTMLPreElement>("terminal-output"),
  terminalStatus: element("terminal-status"),
  autoscrollToggle: element<HTMLInputElement>("autoscroll-toggle"),
  clearOnReconnectToggle: element<HTMLInputElement>("clear-on-reconnect-toggle"),
  clearLogButton: element<HTMLButtonElement>("clear-log-button"),
  copyLogButton: element<HTMLButtonElement>("copy-log-button"),
  downloadLogButton: element<HTMLButtonElement>("download-log-button"),
};

const devices = new Map<SerialPort, DeviceSession>();
const attachingPorts = new Map<SerialPort, Promise<void>>();
const reconnectingSessions = new Set<DeviceSession>();
const pendingConnectedPorts = new Set<SerialPort>();
const removedPorts = new WeakSet<SerialPort>();
let nextDeviceNumber = 1;
let activeDevice: DeviceSession | undefined;
let applicationTabActive = true;
let applicationLog = "";
let searchingForDevices = true;
let unsupportedBrowser = false;
let firmwareHandle: FileSystemFileHandle | undefined;
let restoredHandle: FileSystemFileHandle | undefined;
let observedFileSignature: string | undefined;
let settlingSignature: string | undefined;
let settlingSince = 0;
let watcherTimer: number | undefined;
let flashing = false;
let queuedFlash = false;
let queuedFlashReason: "change" | "manual" = "change";
let fileState: FileState = "empty";
let reconnectTimer: number | undefined;
let reconnectTimerDueAt: number | undefined;
let reconnectSweep: Promise<void> | undefined;

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

function trimLog(value: string): string {
  return value.length > MAX_LOG_CHARACTERS
    ? `… log trimmed …\n${value.slice(-MAX_LOG_CHARACTERS)}`
    : value;
}

function renderTerminalOutput(): void {
  if (applicationTabActive || !activeDevice) {
    ui.terminalOutput.textContent = applicationLog;
    ui.terminalStatus.textContent = "Autoflash activity";
    ui.terminalBody.setAttribute("aria-labelledby", "terminal-tab-autoflash");
  } else {
    ui.terminalOutput.textContent = activeDevice.log;
    ui.terminalStatus.textContent = activeDevice.status;
    ui.terminalBody.setAttribute("aria-labelledby", `terminal-tab-${activeDevice.number}`);
  }
  if (ui.autoscrollToggle.checked) ui.terminalBody.scrollTop = ui.terminalBody.scrollHeight;
}

function appendOutput(session: DeviceSession, value: string): void {
  session.log = trimLog(`${session.log}${value}`);
  if (!applicationTabActive && activeDevice === session) renderTerminalOutput();
}

function appendAutoflashOutput(value: string): void {
  applicationLog = trimLog(`${applicationLog}${value}`);
  if (applicationTabActive || !activeDevice) renderTerminalOutput();
}

function deviceLogContext(session: DeviceSession): string {
  return `${deviceLabel(session)} (${session.portId})`;
}

function appendToolOutput(session: DeviceSession, value: string): void {
  const prefix = `[${deviceLogContext(session)}] `;
  const tagged = value.split("\n").map((line) => line ? `${prefix}${line}` : "").join("\n");
  appendAutoflashOutput(tagged);
}

function systemLine(message: string, level: LogLevel): string {
  const marker = level === "success" ? "✓" : level === "error" ? "!" : "›";
  return `[${clockTime()}] ${marker} ${message}\n`;
}

function appendSystem(message: string, level: LogLevel = "info", session?: DeviceSession): void {
  const context = session ? `${deviceLogContext(session)} · ` : "";
  const separator = applicationLog.length > 0 && !applicationLog.endsWith("\n") ? "\n" : "";
  appendAutoflashOutput(`${separator}${systemLine(`${context}${message}`, level)}`);
}

function allDeviceSessions(): DeviceSession[] {
  return [...devices.values()].sort((left, right) => left.number - right.number);
}

function connectedDevices(): DeviceSession[] {
  return allDeviceSessions().filter((session) => session.connected);
}

function offlineDevices(): DeviceSession[] {
  return allDeviceSessions().filter((session) => !session.connected);
}

function scheduleReconnectSweep(delay = RECONNECT_RETRY_DELAY_MS): void {
  if (!("serial" in navigator) || unsupportedBrowser) return;
  if (offlineDevices().length === 0 && pendingConnectedPorts.size === 0) return;

  const normalizedDelay = Math.max(0, delay);
  const dueAt = performance.now() + normalizedDelay;
  if (reconnectTimer !== undefined) {
    // A real USB connect event must be able to preempt a slower background retry.
    // Otherwise a device plugged in just after a retry was scheduled can sit
    // offline until that old timer expires.
    if (reconnectTimerDueAt !== undefined && reconnectTimerDueAt <= dueAt) return;
    window.clearTimeout(reconnectTimer);
  }

  reconnectTimerDueAt = dueAt;
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = undefined;
    reconnectTimerDueAt = undefined;
    void reconnectOfflineDevices();
  }, normalizedDelay);
}

async function reconnectOfflineDevices(): Promise<void> {
  if (!("serial" in navigator)) return;
  if (reconnectSweep) return reconnectSweep;
  if (flashing) {
    scheduleReconnectSweep(RECONNECT_RETRY_DELAY_MS);
    return;
  }

  const sweep = (async (): Promise<void> => {
    const candidates = [...pendingConnectedPorts];
    pendingConnectedPorts.clear();

    try {
      for (const candidate of await navigator.serial.getPorts()) {
        if (!candidates.includes(candidate)) candidates.push(candidate);
      }
    } catch {
      // A later retry can recover if enumeration fails transiently.
    }

    // Process candidates sequentially. With two identical VID:PID devices this
    // prevents one reconnect sweep from racing two wrappers into the same offline slot.
    for (const candidate of candidates) {
      if (removedPorts.has(candidate)) continue;
      if (!portMatchesSearch(candidate.getInfo(), deviceSearch)) continue;
      try {
        await attachPort(candidate, "Found", false);
      } catch {
        // USB serial drivers can need a short settling period after enumeration.
        // Keep the logical session offline and retry without spamming the log.
      }
    }
  })();

  reconnectSweep = sweep;
  try {
    await sweep;
  } finally {
    if (reconnectSweep === sweep) reconnectSweep = undefined;
    if (offlineDevices().length > 0 || pendingConnectedPorts.size > 0) {
      scheduleReconnectSweep(RECONNECT_RETRY_DELAY_MS);
    }
  }
}

function deviceLabel(session: DeviceSession): string {
  return `Device ${session.number}`;
}

function renderDeviceTabs(): void {
  ui.terminalTabs.replaceChildren();
  for (const session of allDeviceSessions()) {
    const selected = !applicationTabActive && activeDevice === session;
    const tab = document.createElement("button");
    tab.type = "button";
    tab.id = `terminal-tab-${session.number}`;
    tab.className = `terminal-tab ${session.state}${selected ? " active" : ""}`;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", String(selected));
    tab.setAttribute("aria-controls", "terminal-body");

    const name = document.createElement("span");
    name.textContent = deviceLabel(session);
    const info = document.createElement("small");
    info.textContent = session.connected ? session.portId : `${session.portId} · offline`;
    tab.append(name, info);
    tab.addEventListener("click", () => selectDevice(session));
    ui.terminalTabs.append(tab);
  }

  const autoflashTab = document.createElement("button");
  autoflashTab.type = "button";
  autoflashTab.id = "terminal-tab-autoflash";
  autoflashTab.className = `terminal-tab autoflash${applicationTabActive ? " active" : ""}`;
  autoflashTab.setAttribute("role", "tab");
  autoflashTab.setAttribute("aria-selected", String(applicationTabActive));
  autoflashTab.setAttribute("aria-controls", "terminal-body");
  const name = document.createElement("span");
  name.textContent = "Autoflash";
  const info = document.createElement("small");
  info.textContent = "Application log";
  autoflashTab.append(name, info);
  autoflashTab.addEventListener("click", selectAutoflashLog);
  ui.terminalTabs.append(autoflashTab);
}

function selectDevice(session: DeviceSession): void {
  activeDevice = session;
  applicationTabActive = false;
  renderDeviceTabs();
  renderTerminalOutput();
  refreshActionAvailability();
}

function selectAutoflashLog(): void {
  applicationTabActive = true;
  renderDeviceTabs();
  renderTerminalOutput();
  refreshActionAvailability();
}

function refreshDeviceSummary(): void {
  const connected = connectedDevices();
  const connectedErrors = connected.filter((session) => session.state === "error").length;
  let state: ConnectionState;
  let label: string;

  if (unsupportedBrowser) {
    state = "error";
    label = "Attention";
  } else if (flashing || connected.some((session) => session.state === "flashing")) {
    state = "flashing";
    label = connected.length > 1 ? `${connected.length} Flashing` : "Flashing";
  } else if (connected.length > 0 && connectedErrors > 0) {
    state = "error";
    label = "Attention";
  } else if (connected.length > 0) {
    state = "connected";
    label = connected.length === 1 ? "Connected" : `${connected.length} Connected`;
  } else if (searchingForDevices) {
    state = "searching";
    label = "Searching";
  } else {
    state = "needs-permission";
    label = "Setup required";
  }

  ui.deviceState.className = `state-chip ${state}`;
  ui.deviceState.textContent = label;

  if (connected.length === 1) {
    ui.deviceName.textContent = `USB serial ${connected[0].portId}`;
  } else if (connected.length > 1) {
    ui.deviceName.textContent = `${connected.length} USB serial devices connected`;
  } else if (devices.size > 0) {
    ui.deviceName.textContent = "All devices disconnected";
  } else if (!searchingForDevices) {
    ui.deviceName.textContent = `No authorized ${SERIAL_PORT_SEARCH} device found`;
  }

  refreshActionAvailability();
  renderDeviceTabs();
  renderTerminalOutput();
}

function setDeviceState(session: DeviceSession, state: ConnectionState, detail?: string): void {
  session.state = state;
  const headerLabels: Record<ConnectionState, string> = {
    searching: "Finding device",
    "needs-permission": "Device disconnected",
    connected: "Serial monitor live",
    flashing: "Writing firmware",
    error: "Device needs attention",
  };
  session.status = detail ?? headerLabels[state];
  refreshDeviceSummary();
}

function setFileState(state: FileState): void {
  fileState = state;
  const labels: Record<FileState, string> = {
    empty: "Not selected",
    permission: "Permission needed",
    watching: "Watching",
    settling: "Change detected",
    queued: "Waiting for tab",
    paused: "Paused",
  };
  ui.fileState.className = `state-chip ${state}`;
  ui.fileState.textContent = labels[state];
}

function refreshActionAvailability(): void {
  const hasConnectedDevice = connectedDevices().length > 0;
  const selectedConnected = !applicationTabActive && activeDevice?.connected === true;
  ui.flashButton.disabled = !firmwareHandle || !hasConnectedDevice || flashing;
  ui.filePicker.disabled = flashing;
  ui.chooseAnotherButton.disabled = flashing;
  ui.watchToggle.disabled = !firmwareHandle || flashing;
  ui.resetButton.disabled = !selectedConnected || flashing;
  ui.removeDeviceButton.disabled = applicationTabActive || !activeDevice || flashing;
  ui.authorizeButton.disabled = unsupportedBrowser || flashing;

  ui.resetButton.title = selectedConnected && activeDevice
    ? `Reset ${deviceLabel(activeDevice)} (${activeDevice.portId})`
    : "Select a connected device tab to reset it";
  ui.removeDeviceButton.title = !applicationTabActive && activeDevice
    ? `Disconnect and remove ${deviceLabel(activeDevice)} (${activeDevice.portId}) from this session`
    : "Select a device tab to remove it";
}

function updateProgress(percent: number | undefined, message: string): void {
  ui.progressText.textContent = message;
  ui.progressPercent.textContent = percent === undefined ? "—" : `${Math.round(percent)}%`;
  ui.progressBar.style.width = percent === undefined ? "0%" : `${Math.max(0, Math.min(100, percent))}%`;
  ui.progressBlock.classList.toggle("active", percent !== undefined);
}

async function startMonitor(session: DeviceSession, allowDuringFlash = false): Promise<void> {
  if (!session.connected || (flashing && !allowDuringFlash) || session.monitorLoop) return;
  if (session.monitorStart) return session.monitorStart;

  const start = (async (): Promise<void> => {
    if (!session.port.readable) {
      await session.port.open({ baudRate: MONITOR_BAUD_RATE, bufferSize: 65_536 });
    }

    // A remove/disconnect can happen while port.open() is in flight. A brand-new
    // session is allowed to open before it is committed to the devices map, but an
    // already-registered different session always owns that exact SerialPort object.
    const registeredOwner = devices.get(session.port);
    if (!session.connected || (registeredOwner && registeredOwner !== session)) {
      if (session.port.readable) await session.port.close();
      return;
    }
    if (!session.port.readable) throw new Error("The serial port did not provide a readable stream.");

    session.stopRequested = false;
    const readable = session.port.readable;
    if (!readable) throw new Error("The serial port did not provide a readable stream.");
    const decoder = new TextDecoder();
    const reader = readable.getReader();
    session.disconnectedAt = undefined;
    const reconnecting = session.monitorOpened;
    session.monitorOpened = true;
    if (reconnecting && ui.clearOnReconnectToggle.checked) {
      session.log = "";
      if (!applicationTabActive && activeDevice === session) renderTerminalOutput();
    }
    session.monitorReader = reader;
    setDeviceState(session, "connected", "Receiving serial output");
    appendSystem(`Serial monitor opened at ${MONITOR_BAUD_RATE.toLocaleString()} baud.`, "success", session);

    session.monitorLoop = (async () => {
      let streamEnded = false;
      try {
        while (!session.stopRequested) {
          const { value, done } = await reader.read();
          if (done) {
            streamEnded = true;
            break;
          }
          if (value) appendOutput(session, decoder.decode(value, { stream: true }));
        }
        const remainder = decoder.decode();
        if (remainder) appendOutput(session, remainder);
      } catch (error) {
        if (!session.stopRequested && session.connected) {
          appendSystem(`Serial monitor stopped: ${errorMessage(error)}`, "error", session);
          markSessionDisconnected(session, "Serial stream stopped");
        }
      } finally {
        reader.releaseLock();
        if (session.monitorReader === reader) session.monitorReader = undefined;
      }

      // Chromium does not always deliver a disconnect event through the same
      // SerialPort wrapper that was originally opened. Treat an unexpected EOF as
      // a disconnect as well so this logical device slot can be reused later.
      if (streamEnded && !session.stopRequested && session.connected) {
        markSessionDisconnected(session, "Device disconnected", "Serial stream ended; waiting for reconnect.");
      }
    })().finally(() => {
      session.monitorLoop = undefined;
    });
  })();

  session.monitorStart = start;
  try {
    await start;
  } finally {
    if (session.monitorStart === start) session.monitorStart = undefined;
  }
}

async function stopMonitor(session: DeviceSession, closePort = true): Promise<void> {
  session.stopRequested = true;
  const starting = session.monitorStart;
  if (starting) {
    try {
      await starting;
    } catch {
      // Cleanup below still needs to run if opening the monitor failed.
    }
  }
  const reader = session.monitorReader;
  if (reader) {
    try {
      await reader.cancel();
    } catch {
      // A physical disconnect can invalidate the reader before cancellation.
    }
  }
  if (session.monitorLoop) await session.monitorLoop;
  if (closePort && session.port.readable) {
    try {
      await session.port.close();
    } catch (error) {
      appendSystem(`Could not close serial monitor cleanly: ${errorMessage(error)}`, "error", session);
    }
  }
}


function markSessionDisconnected(
  session: DeviceSession,
  status: string,
  message?: string,
  retryDelay = RECONNECT_EVENT_DELAY_MS,
): void {
  const wasConnected = session.connected;
  session.connected = false;
  session.disconnectedAt = Date.now();
  session.state = "needs-permission";
  session.status = status;
  if (message && wasConnected) appendSystem(message, "error", session);
  refreshDeviceSummary();
  scheduleReconnectSweep(retryDelay);
}

function reconcileClosedWrappers(portId: string): void {
  if (flashing) return;

  for (const session of allDeviceSessions()) {
    if (!session.connected || session.portId !== portId) continue;
    if (session.port.readable || session.port.writable) continue;

    // The logical session still says connected, but Chromium has already detached
    // its old wrapper. Mark it offline now so the new wrapper can reuse this tab.
    session.stopRequested = true;
    markSessionDisconnected(
      session,
      "Device disconnected",
      "Previous serial-port wrapper is no longer available; waiting for reconnect.",
    );
  }
}

function claimReusableOfflineSession(portId: string): DeviceSession | undefined {
  const candidate = allDeviceSessions()
    .filter((session) => !session.connected)
    .filter((session) => session.portId === portId)
    .filter((session) => !reconnectingSessions.has(session))
    .sort((left, right) => (right.disconnectedAt ?? 0) - (left.disconnectedAt ?? 0))[0];

  if (candidate) reconnectingSessions.add(candidate);
  return candidate;
}

async function reuseOfflineSession(
  session: DeviceSession,
  port: SerialPort,
  source: "Authorized" | "Found",
  activate: boolean,
): Promise<void> {
  const previousPort = session.port;

  // Finish cleanup on the old wrapper before rebinding the logical device slot.
  // The physical port is already gone, so do not attempt to close that old wrapper.
  await stopMonitor(session, false);
  if (devices.get(previousPort) !== session) {
    throw new Error(`${deviceLabel(session)} changed while reconnecting.`);
  }

  devices.delete(previousPort);
  session.port = port;
  session.portId = formatPortInfo(port.getInfo());
  session.connected = true;
  session.state = "searching";
  session.status = "Reconnecting serial port";
  session.stopRequested = false;
  devices.set(port, session);

  try {
    await startMonitor(session, true);
  } catch (error) {
    markSessionDisconnected(session, "Device unavailable", undefined, RECONNECT_RETRY_DELAY_MS);
    throw error;
  }

  if (activate) selectDevice(session);
  appendSystem(`${source} device ${session.portId}; reused ${deviceLabel(session)} after reconnect.`, "success", session);
  refreshDeviceSummary();
}

async function attachPort(port: SerialPort, source: "Authorized" | "Found", activate: boolean): Promise<void> {
  // The authorization path, reconnect sweep, and Web Serial connect event can all
  // report the same SerialPort. Make every attachment path single-flight first.
  const pending = attachingPorts.get(port);
  if (pending) {
    await pending;
    const attached = devices.get(port);
    if (activate && attached) selectDevice(attached);
    return;
  }

  const existing = devices.get(port);
  if (existing) {
    if (activate) selectDevice(existing);

    if (!existing.connected) {
      // Chromium often reuses the exact same SerialPort object after unplug/replug.
      // The old monitor loop can still be unwinding at this point; using the same
      // rebind path as a new wrapper guarantees cleanup finishes before reopen.
      reconnectingSessions.add(existing);
      const attach = reuseOfflineSession(existing, port, source, activate);
      attachingPorts.set(port, attach);
      try {
        await attach;
      } finally {
        if (attachingPorts.get(port) === attach) attachingPorts.delete(port);
        reconnectingSessions.delete(existing);
      }
      return;
    }

    if (existing.monitorLoop || existing.monitorStart || port.readable) {
      refreshDeviceSummary();
      return;
    }

    try {
      await startMonitor(existing);
      refreshDeviceSummary();
    } catch (error) {
      markSessionDisconnected(existing, "Device unavailable", undefined, RECONNECT_RETRY_DELAY_MS);
      throw error;
    }
    return;
  }

  // A port that is already open but is not keyed in our session map cannot safely
  // be claimed. It may be another wrapper for a connection that is already live.
  if (port.readable || port.writable) {
    throw new Error("Serial port is already open; duplicate device was not added.");
  }

  const portId = formatPortInfo(port.getInfo());
  reconcileClosedWrappers(portId);
  const reusable = claimReusableOfflineSession(portId);
  if (reusable) {
    const attach = reuseOfflineSession(reusable, port, source, activate);
    attachingPorts.set(port, attach);
    try {
      await attach;
    } finally {
      if (attachingPorts.get(port) === attach) attachingPorts.delete(port);
      reconnectingSessions.delete(reusable);
    }
    return;
  }

  const attach = (async (): Promise<void> => {
    const session: DeviceSession = {
      port,
      number: nextDeviceNumber,
      portId,
      connected: true,
      state: "searching",
      status: "Verifying serial port",
      log: "",
      monitorOpened: false,
      stopRequested: false,
    };
    nextDeviceNumber += 1;

    try {
      // Opening the port is the definitive duplicate/busy check. If another JS
      // wrapper refers to an already-open physical port, this fails and no tab is kept.
      // Verification must never succeed merely because normal monitor startup is
      // suppressed during a flash.
      await startMonitor(session, true);
    } catch (error) {
      session.connected = false;
      session.stopRequested = true;
      try {
        await stopMonitor(session, true);
      } catch {
        // Preserve the original open/reader error.
      }
      throw error;
    }

    devices.set(port, session);
    if (!activeDevice || activate) {
      activeDevice = session;
      applicationTabActive = false;
    }
    appendSystem(`${source} device ${session.portId}.`, "success", session);
    refreshDeviceSummary();
  })();

  attachingPorts.set(port, attach);
  try {
    await attach;
  } finally {
    if (attachingPorts.get(port) === attach) attachingPorts.delete(port);
  }
}

async function connectPorts(interactive: boolean): Promise<void> {
  if (!("serial" in navigator)) throw new Error("Web Serial is not supported by this browser.");

  if (interactive) {
    const filters = requestPortFilters(deviceSearch);
    const selected = filters
      ? await navigator.serial.requestPort({ filters })
      : await navigator.serial.requestPort();
    removedPorts.delete(selected);
    await attachPort(selected, "Authorized", true);
    return;
  }

  searchingForDevices = true;
  refreshDeviceSummary();
  try {
    const authorized = (await navigator.serial.getPorts())
      .filter((candidate) => !removedPorts.has(candidate))
      .filter((candidate) => portMatchesSearch(candidate.getInfo(), deviceSearch));
    const results = await Promise.allSettled(authorized.map((candidate) => attachPort(candidate, "Found", false)));
    if (connectedDevices().length === 0) {
      const failure = results.find((result): result is PromiseRejectedResult => result.status === "rejected");
      if (failure) throw failure.reason;
    }
  } finally {
    searchingForDevices = false;
    refreshDeviceSummary();
  }
}

async function resetDevice(session: DeviceSession, announce = true): Promise<void> {
  if (!session.connected) throw new Error(`${deviceLabel(session)} is disconnected.`);
  if (!session.port.readable) await startMonitor(session);
  if (announce) appendSystem("Resetting device via RTS…", "info", session);

  // RTS drives EN low on standard Espressif auto-reset circuits. Holding DTR
  // inactive keeps GPIO0 high so the chip returns to the application, not ROM download mode.
  await session.port.setSignals({ dataTerminalReady: false, requestToSend: true });
  await sleep(120);
  await session.port.setSignals({ dataTerminalReady: false, requestToSend: false });
  await sleep(100);
  if (announce) appendSystem("Reset released; serial monitor is running.", "success", session);
}

async function resetSelectedDevice(): Promise<void> {
  const session = activeDevice;
  if (applicationTabActive || !session) throw new Error("Select a device tab first.");
  if (!session.connected) throw new Error(`${deviceLabel(session)} is disconnected.`);
  await resetDevice(session);
}

async function removeDevice(session: DeviceSession): Promise<void> {
  if (flashing) throw new Error("Wait for the current flash to finish before removing a device.");
  if (devices.get(session.port) !== session) return;

  const ordered = allDeviceSessions();
  const removedIndex = ordered.indexOf(session);
  const wasActive = !applicationTabActive && activeDevice === session;
  const label = deviceLabel(session);
  const portId = session.portId;

  // Keep browser authorization intact, but do not let automatic connect events
  // re-add this port during the current page session. The user can explicitly
  // authorize it again at any time.
  removedPorts.add(session.port);
  session.connected = false;
  session.stopRequested = true;

  try {
    await stopMonitor(session, true);
  } finally {
    devices.delete(session.port);
    if (wasActive) {
      const remaining = allDeviceSessions();
      activeDevice = remaining.length === 0
        ? undefined
        : remaining[Math.min(Math.max(removedIndex, 0), remaining.length - 1)];
      applicationTabActive = remaining.length === 0;
    }
    refreshDeviceSummary();
  }

  appendSystem(`${label} (${portId}) removed from this session. Browser authorization was kept.`, "success");
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
  refreshDeviceSummary();
  let activeReason: "change" | "manual" = queuedFlashReason;

  try {
    while (queuedFlash) {
      activeReason = queuedFlashReason;
      queuedFlash = false;
      await flashLatestFirmware(activeReason);
    }
  } catch (error) {
    appendSystem(`Flash failed: ${errorMessage(error)}`, "error");
    if (activeReason === "change" && document.hidden && ui.watchToggle.checked) {
      queuedFlash = true;
      queuedFlashReason = "change";
      setFileState("queued");
      updateProgress(undefined, "Retry queued — activate this tab to flash");
      appendSystem("The tab became inactive during automatic flashing. It will retry automatically when active.");
    } else {
      queuedFlash = false;
      updateProgress(undefined, "Flash failed — inspect the Autoflash log");
    }
  } finally {
    flashing = false;
    refreshDeviceSummary();

    // Reconcile USB devices that re-enumerated while flashing before restoring
    // monitors. This also retries transient port-open failures without new tabs.
    await reconnectOfflineDevices();

    for (const session of connectedDevices()) {
      if (!session.monitorLoop) {
        try {
          await startMonitor(session);
        } catch (error) {
          appendSystem(`Could not restore monitor: ${errorMessage(error)}`, "error", session);
          setDeviceState(session, "error", "Monitor restore failed");
        }
      }
    }
  }
}

async function flashLatestFirmware(reason: "change" | "manual"): Promise<void> {
  if (!firmwareHandle) throw new Error("Choose a firmware file first.");
  const targets = connectedDevices();
  if (targets.length === 0) throw new Error("Connect at least one serial device first.");

  const file = await firmwareHandle.getFile();
  if (file.size === 0) throw new Error("The selected firmware file is empty.");
  const image = new Uint8Array(await file.arrayBuffer());
  ui.fileMeta.textContent = `${formatBytes(file.size)} · loaded ${new Date().toLocaleTimeString()}`;
  updateProgress(0, `${reason === "change" ? "Change stable" : "Manual flash"} · loading ${file.name}`);
  appendSystem(`${reason === "change" ? "File changed" : "Manual flash"}: writing ${file.name} (${formatBytes(file.size)}) to ${targets.length} device${targets.length === 1 ? "" : "s"}${targets.length === 1 ? "" : " in parallel"} at 0x${FLASH_ADDRESS.toString(16)}.`);

  const progressByDevice = new Map<DeviceSession, number>(
    targets.map((session) => [session, 0]),
  );
  const completedDevices = new Set<DeviceSession>();

  const refreshParallelProgress = (): void => {
    const aggregate = [...progressByDevice.values()].reduce((sum, value) => sum + value, 0) / targets.length;
    updateProgress(
      aggregate,
      `Flashing ${targets.length} devices in parallel · ${completedDevices.size}/${targets.length} complete`,
    );
  };

  const reportDeviceProgress = (session: DeviceSession, percent: number, detail: string): void => {
    if (targets.length === 1) {
      updateProgress(percent, detail);
      return;
    }

    const maximum = completedDevices.has(session) ? 100 : 99;
    progressByDevice.set(session, Math.max(0, Math.min(maximum, percent)));
    refreshParallelProgress();
  };

  const results = await Promise.allSettled(targets.map(async (session) => {
    if (!session.connected) throw new Error("Device disconnected before flashing started.");
    await flashDevice(
      session,
      image,
      file,
      (percent, detail) => reportDeviceProgress(session, percent, detail),
    );
    completedDevices.add(session);
    progressByDevice.set(session, 100);
    if (targets.length > 1) refreshParallelProgress();
  }));

  const failures: Array<{ session: DeviceSession; error: unknown }> = [];
  results.forEach((result, index) => {
    if (result.status === "fulfilled") return;
    const session = targets[index];
    failures.push({ session, error: result.reason });
    appendSystem(`Flash failed: ${errorMessage(result.reason)}`, "error", session);
    setDeviceState(session, "error", "Flash failed");
  });

  if (failures.length > 0) {
    const first = failures[0];
    throw new Error(`${failures.length} of ${targets.length} devices failed; ${deviceLabel(first.session)}: ${errorMessage(first.error)}`);
  }

  setFileState(ui.watchToggle.checked ? "watching" : "paused");
  updateProgress(undefined, "Flash complete — watching for the next change");
  appendSystem(`${targets.length === 1 ? "Device" : "All devices"} reset; live serial output resumed.`, "success");
}

async function flashDevice(
  session: DeviceSession,
  image: Uint8Array,
  file: File,
  reportProgress: (percent: number, detail: string) => void,
): Promise<void> {
  setDeviceState(session, "flashing", "Entering bootloader");
  appendSystem(`Writing ${file.name} (${formatBytes(file.size)}) at 0x${FLASH_ADDRESS.toString(16)}.`, "info", session);

  await stopMonitor(session, true);
  const activePort = session.port;
  const transport = new EventDrivenTransport(activePort, false);
  let transportOpen = false;
  let finalWriteLineSeen = false;
  let resolveFinalWriteLine!: () => void;
  const finalWriteLine = new Promise<void>((resolve) => {
    resolveFinalWriteLine = resolve;
  });

  const handleFlashLogLine = (data: string): void => {
    appendToolOutput(session, `${data}\n`);
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
      write: (data: string) => appendToolOutput(session, data),
      writeLine: handleFlashLogLine,
    },
  });

  try {
    const chip = await loader.main();
    transportOpen = true;
    appendSystem(`Bootloader connected: ${chip}.`, "success", session);
    setDeviceState(session, "flashing", `Writing flash at 0x${FLASH_ADDRESS.toString(16)}`);

    const writeTask = loader.writeFlash({
      fileArray: [{ data: image, address: FLASH_ADDRESS }],
      flashMode: "keep",
      flashFreq: "keep",
      flashSize: "keep",
      eraseAll: false,
      compress: true,
      reportProgress: (_fileIndex, written, total) => {
        const percent = total === 0 ? 0 : (written / total) * 100;
        reportProgress(percent, `${deviceLabel(session)} · writing ${formatBytes(written)} of ${formatBytes(total)}`);
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
      reportProgress(100, `${deviceLabel(session)} · 100% reached — restoring serial monitor`);
      appendSystem("100% write line received. Reopening the serial monitor now…", "success", session);
    } else {
      reportProgress(100, `${deviceLabel(session)} · firmware written — restoring serial monitor`);
      appendSystem("Firmware written. Reopening the serial monitor…", "success", session);
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

  await startMonitor(session, true);
  await resetDevice(session, false);
  reportProgress(100, `${deviceLabel(session)} · flash complete — monitor live`);
  setDeviceState(session, "connected", "Flash complete · monitor live");
  appendSystem("Device reset; live serial output resumed.", "success", session);
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
  unsupportedBrowser = true;
  ui.filePicker.disabled = true;
  appendSystem("Unsupported browser. Open this site in current Chrome or Edge on desktop; Web Serial and persistent file handles are required.", "error");
  refreshDeviceSummary();
}

ui.authorizeButton.addEventListener("click", () => {
  void connectPorts(true).catch((error) => {
    if (!(error instanceof DOMException && error.name === "NotFoundError")) {
      appendSystem(`Connection failed: ${errorMessage(error)}`, "error");
      refreshDeviceSummary();
    }
  });
});

ui.resetButton.addEventListener("click", () => {
  void resetSelectedDevice().catch((error) => appendSystem(`Reset failed: ${errorMessage(error)}`, "error", activeDevice));
});

ui.removeDeviceButton.addEventListener("click", () => {
  const session = activeDevice;
  if (applicationTabActive || !session) return;
  void removeDevice(session).catch((error) => appendSystem(`Remove failed: ${errorMessage(error)}`, "error", session));
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
  if (!ui.watchToggle.checked && queuedFlash && queuedFlashReason === "change" && !flashing) {
    queuedFlash = false;
  }
  setFileState(ui.watchToggle.checked ? "watching" : "paused");
  updateProgress(undefined, ui.watchToggle.checked ? "Watching for a stable file change" : "File watching is paused");
  appendSystem(`Automatic flashing ${ui.watchToggle.checked ? "enabled" : "paused"}.`);
});

ui.flashButton.addEventListener("click", () => queueFlash("manual"));
ui.autoscrollToggle.addEventListener("change", () => {
  if (ui.autoscrollToggle.checked) ui.terminalBody.scrollTop = ui.terminalBody.scrollHeight;
});
ui.clearLogButton.addEventListener("click", () => {
  if (applicationTabActive || !activeDevice) {
    applicationLog = "";
  } else {
    activeDevice.log = "";
  }
  renderTerminalOutput();
});
ui.copyLogButton.addEventListener("click", () => {
  const log = applicationTabActive || !activeDevice ? applicationLog : activeDevice.log;
  if (!navigator.clipboard) {
    appendSystem("Copy failed: Clipboard API is not available in this browser context.", "error", applicationTabActive ? undefined : activeDevice);
    return;
  }

  void navigator.clipboard.writeText(log).catch((error) => {
    appendSystem(`Copy failed: ${errorMessage(error)}`, "error", applicationTabActive ? undefined : activeDevice);
  });
});
ui.downloadLogButton.addEventListener("click", () => {
  const log = applicationTabActive || !activeDevice ? applicationLog : activeDevice.log;
  const blob = new Blob([log], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  const devicePart = applicationTabActive || !activeDevice ? "-autoflash" : `-device-${activeDevice.number}-${activeDevice.portId.replace(":", "-")}`;
  link.download = `esp-autoflash${devicePart}-${new Date().toISOString().replace(/[:.]/g, "-")}.log`;
  link.click();
  URL.revokeObjectURL(url);
});

document.addEventListener("visibilitychange", () => {
  if (document.hidden) return;

  scheduleReconnectSweep(RECONNECT_EVENT_DELAY_MS);

  // Poll now in case the browser suspended the interval entirely while hidden.
  void pollFirmware().finally(() => {
    if (queuedFlash && queuedFlashReason === "change" && ui.watchToggle.checked && !flashing) {
      setFileState("watching");
      updateProgress(undefined, "Tab active — starting queued firmware");
      appendSystem("Tab active. Starting the queued automatic flash.");
      void runFlashQueue();
    }
  });
});

if ("serial" in navigator) {
  navigator.serial.addEventListener("connect", (event) => {
    const connectedPort = event.target as SerialPort | null;
    if (!connectedPort || removedPorts.has(connectedPort)) return;
    if (!portMatchesSearch(connectedPort.getInfo(), deviceSearch)) return;

    // Native-USB ESP devices can re-enumerate while a flash/reset is still in
    // progress. Do not commit a new wrapper while monitor startup is suppressed;
    // reconcile it with the previous logical slot once the flash batch finishes.
    if (flashing) {
      pendingConnectedPorts.add(connectedPort);
      return;
    }

    // Let the reconnect sweep serialize cleanup/reopen with any old wrapper.
    // The USB event gets an immediate attempt; transient driver-enumeration
    // failures fall back to the short retry interval above.
    pendingConnectedPorts.add(connectedPort);
    scheduleReconnectSweep(RECONNECT_EVENT_DELAY_MS);
  });
  navigator.serial.addEventListener("disconnect", (event) => {
    const disconnectedPort = event.target as SerialPort | null;
    if (!disconnectedPort) return;
    const session = devices.get(disconnectedPort);
    if (!session) return;

    session.stopRequested = true;
    markSessionDisconnected(session, "Device disconnected", "Serial device disconnected; waiting for reconnect.");
  });
}

async function initialize(): Promise<void> {
  appendSystem(`Ready. Target is ${SERIAL_PORT_SEARCH}; firmware address is 0x${FLASH_ADDRESS.toString(16)}.`);
  if (!("serial" in navigator) || !window.showOpenFilePicker || !window.isSecureContext) {
    searchingForDevices = false;
    showUnsupportedBrowser();
    return;
  }
  await Promise.all([
    connectPorts(false).catch((error) => {
      appendSystem(`Automatic connection failed: ${errorMessage(error)}`, "error");
      searchingForDevices = false;
      refreshDeviceSummary();
    }),
    restoreFirmware(),
  ]);
}

void initialize();
