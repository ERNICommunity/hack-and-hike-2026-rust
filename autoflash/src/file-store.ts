/**
 * Stores the handle of the firmware file in IndexedDB, the database of the
 * browser. A handle is a reference to the file, not its content. So the page
 * can use the same file again after a reload.
 */

const DATABASE_NAME = "esp-autoflash";
/** The object store that holds file handles. */
const STORE_NAME = "handles";
/** The key of the firmware file handle in the store. */
const FIRMWARE_KEY = "firmware";

/** Open the database, and create the store the first time. */
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

/** Save `handle` as the firmware file, in place of an earlier one. */
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

/**
 * The saved firmware file handle, or `undefined` when there is none. The
 * browser can ask for read permission again before the page can use it.
 */
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
