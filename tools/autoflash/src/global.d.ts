/** Browser features that the TypeScript DOM types do not declare. */
interface Window {
  /** Set by `/runtime-config.js`, which the Rust server generates. */
  __ESP_AUTOFLASH_CONFIG__?: {
    /** The device filter, for example `303a:*`. */
    serialPortSearch?: string;
  };
  /** The file picker of the File System Access API. Only Chromium-based browsers have it. */
  showOpenFilePicker?: (options?: {
    multiple?: boolean;
    excludeAcceptAllOption?: boolean;
    types?: Array<{
      description?: string;
      accept: Record<string, string[]>;
    }>;
  }) => Promise<FileSystemFileHandle[]>;
}

/** Permission methods of the File System Access API, which not every browser has. */
interface FileSystemHandle {
  queryPermission?: (descriptor?: { mode?: "read" | "readwrite" }) => Promise<PermissionState>;
  requestPermission?: (descriptor?: { mode?: "read" | "readwrite" }) => Promise<PermissionState>;
}

/** CSS files are imported only for their side effect: Vite adds them to the page. */
declare module "*.css";
