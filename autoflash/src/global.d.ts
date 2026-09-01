interface Window {
  __ESP_AUTOFLASH_CONFIG__?: {
    serialPortSearch?: string;
  };
  showOpenFilePicker?: (options?: {
    multiple?: boolean;
    excludeAcceptAllOption?: boolean;
    types?: Array<{
      description?: string;
      accept: Record<string, string[]>;
    }>;
  }) => Promise<FileSystemFileHandle[]>;
}

interface FileSystemHandle {
  queryPermission?: (descriptor?: { mode?: "read" | "readwrite" }) => Promise<PermissionState>;
  requestPermission?: (descriptor?: { mode?: "read" | "readwrite" }) => Promise<PermissionState>;
}

declare module "spark-md5" {
  const SparkMD5: {
    ArrayBuffer: {
      hash(buffer: ArrayBuffer): string;
    };
  };
  export default SparkMD5;
}

declare module "*.css";
