const DATABASE_NAME = "esp-autoflash";
const STORE_NAME = "handles";
const FIRMWARE_KEY = "firmware";

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE_NAME, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE_NAME);
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("Could not open IndexedDB."));
  });
}

export async function saveFirmwareHandle(handle: FileSystemFileHandle): Promise<void> {
  const database = await openDatabase();
  try {
    await new Promise<void>((resolve, reject) => {
      const transaction = database.transaction(STORE_NAME, "readwrite");
      transaction.objectStore(STORE_NAME).put(handle, FIRMWARE_KEY);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error ?? new Error("Could not save the file handle."));
    });
  } finally {
    database.close();
  }
}

export async function loadFirmwareHandle(): Promise<FileSystemFileHandle | undefined> {
  const database = await openDatabase();
  try {
    return await new Promise<FileSystemFileHandle | undefined>((resolve, reject) => {
      const request = database.transaction(STORE_NAME, "readonly")
        .objectStore(STORE_NAME)
        .get(FIRMWARE_KEY);
      request.onsuccess = () => resolve(request.result as FileSystemFileHandle | undefined);
      request.onerror = () => reject(request.error ?? new Error("Could not restore the file handle."));
    });
  } finally {
    database.close();
  }
}
