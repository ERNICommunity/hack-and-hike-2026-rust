//! The Autoflash server: a small HTTP server without other crates.
//!
//! It sends the page files that `build.rs` embeds from `site/`. It also
//! decodes panic backtraces at `/api/backtrace` (see `backtrace.rs`).
//! Each connection gets its own thread and one response.

mod backtrace;
mod sha256;

use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use backtrace::Symbolizer;

/// The address to listen on when `ESP_AUTOFLASH_BIND` is not set.
///
/// With `127.0.0.1`, only programs on the same computer can connect. The port
/// forwarding of VS Code Dev Containers still works: it connects to
/// `127.0.0.1` inside the container.
const DEFAULT_BIND: &str = "127.0.0.1";
/// The TCP port when `ESP_AUTOFLASH_PORT` is not set.
const DEFAULT_PORT: &str = "8080";
/// The device filter when `ESP_AUTOFLASH_SERIAL_PORT_SEARCH` is not set:
/// every device with the Espressif USB vendor ID.
const DEFAULT_SERIAL_PORT_SEARCH: &str = "303a:*";
/// The largest request header that the server accepts (32 KiB).
const MAX_REQUEST_HEADER_BYTES: usize = 32 * 1024;
/// The time a client has to send its whole request header.
///
/// A timeout for each read is not enough. A client that sends one byte every
/// few seconds could then keep a connection open for hours.
const REQUEST_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
/// The largest number of connections that the server serves at the same time.
/// The server closes more connections at once. A browser opens about six
/// connections to one server.
const MAX_CONNECTIONS: usize = 32;
/// The path of the backtrace decoder.
const BACKTRACE_API_PATH: &str = "/api/backtrace";

/// The settings and tools that all connection threads share.
struct AppState {
    /// The device filter that `/runtime-config.js` sends to the page.
    serial_port_search: String,
    /// Decodes the backtraces for `/api/backtrace`.
    symbolizer: Symbolizer,
}

/// One file from `site/`, embedded into the executable by `build.rs`.
struct EmbeddedAsset {
    /// The path relative to `site/`, with `/` as separator.
    path: &'static str,
    /// The file content.
    bytes: &'static [u8],
}

// Defines `EMBEDDED_ASSETS`, the table of all files that `build.rs` found.
include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));

fn main() -> io::Result<()> {
    let bind = env::var("ESP_AUTOFLASH_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let port = env::var("ESP_AUTOFLASH_PORT").unwrap_or_else(|_| DEFAULT_PORT.to_owned());
    let serial_port_search = env::var("ESP_AUTOFLASH_SERIAL_PORT_SEARCH")
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|_| DEFAULT_SERIAL_PORT_SEARCH.to_owned());
    if !is_valid_serial_port_search(&serial_port_search) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ESP_AUTOFLASH_SERIAL_PORT_SEARCH must be vvvv:pppp, vvvv:* or *",
        ));
    }

    let address = format!("{bind}:{port}");
    let listener = TcpListener::bind(&address)?;
    let state = Arc::new(AppState {
        serial_port_search,
        symbolizer: Symbolizer::from_env(),
    });

    println!("ESP AutoFlash listening on http://{address}");
    println!("From the host, open the forwarded port at http://localhost:{port}");
    println!("Serial device match: {}", state.serial_port_search);
    let elf_directories: Vec<String> = state
        .symbolizer
        .elf_directories()
        .iter()
        .map(|directory| directory.display().to_string())
        .collect();
    println!(
        "Backtraces are decoded with ELF files from: {}",
        elf_directories.join(", ")
    );
    println!(
        "Serving {} embedded files; no runtime assets are required",
        EMBEDDED_ASSETS.len()
    );

    let open_connections = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let Some(slot) = ConnectionSlot::take(&open_connections) else {
                    // Dropping the stream closes the connection.
                    continue;
                };
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    let _slot = slot;
                    if let Err(error) = handle_connection(stream, &state) {
                        eprintln!("request failed: {error}");
                    }
                });
            }
            Err(error) => eprintln!("connection failed: {error}"),
        }
    }

    Ok(())
}

/// The right to serve one of the `MAX_CONNECTIONS` connections. The slot is
/// free again when this value is dropped.
struct ConnectionSlot(Arc<AtomicUsize>);

impl ConnectionSlot {
    /// Take a free slot and count it in `open_connections`. Returns `None`
    /// when all `MAX_CONNECTIONS` slots are in use.
    fn take(open_connections: &Arc<AtomicUsize>) -> Option<Self> {
        open_connections
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |open| {
                (open < MAX_CONNECTIONS).then_some(open + 1)
            })
            .ok()
            .map(|_| ConnectionSlot(Arc::clone(open_connections)))
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Read one request from `stream`, send one response, and close the
/// connection.
fn handle_connection(mut stream: TcpStream, state: &AppState) -> io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;

    let request = match read_request_headers(&mut stream) {
        Ok(request) => request,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return write_text_response(&mut stream, 400, "Bad Request", "Bad request\n", false);
        }
        Err(error) => return Err(error),
    };

    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or("");
    let target = request_parts.next().unwrap_or("");
    let version = request_parts.next().unwrap_or("");

    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") || request_parts.next().is_some() {
        return write_text_response(&mut stream, 400, "Bad Request", "Bad request\n", false);
    }

    if !matches!(method, "GET" | "HEAD") {
        return write_text_response(
            &mut stream,
            405,
            "Method Not Allowed",
            "Only GET and HEAD are supported\n",
            method == "HEAD",
        );
    }

    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path == BACKTRACE_API_PATH {
        return write_backtrace_response(&mut stream, &state.symbolizer, query, method == "HEAD");
    }

    let Some(relative_path) = normalized_request_path(target) else {
        return write_text_response(
            &mut stream,
            400,
            "Bad Request",
            "Invalid path\n",
            method == "HEAD",
        );
    };

    if relative_path == Path::new("runtime-config.js") {
        return write_runtime_config_response(
            &mut stream,
            &state.serial_port_search,
            method == "HEAD",
        );
    }

    match select_asset(&relative_path) {
        Some(asset) => write_asset_response(&mut stream, asset, method == "HEAD"),
        None => write_text_response(
            &mut stream,
            404,
            "Not Found",
            "Not found\n",
            method == "HEAD",
        ),
    }
}

/// Read the request line and the header lines, up to the first empty line.
///
/// The server supports only `GET` and `HEAD`, so it ignores a request body.
/// Returns an `InvalidData` error when the header is larger than
/// `MAX_REQUEST_HEADER_BYTES` or is not UTF-8. Returns another error when the
/// client needs more than `REQUEST_HEADER_TIMEOUT`.
fn read_request_headers(stream: &mut TcpStream) -> io::Result<String> {
    let mut bytes = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 2048];

    let deadline = Instant::now() + REQUEST_HEADER_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "request headers took too long",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);

        if bytes.len() > MAX_REQUEST_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers are too large",
            ));
        }
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request headers are not UTF-8"))
}

/// The file path of a request target, relative to `site/`.
///
/// Removes the query and the fragment, and decodes `%XX` escapes. `/` becomes
/// `index.html`. Returns `None` for a path with `..`, a backslash or a zero
/// byte, so that a request cannot leave `site/`.
fn normalized_request_path(target: &str) -> Option<PathBuf> {
    let encoded_path = target.split(['?', '#']).next()?;
    if !encoded_path.starts_with('/') {
        return None;
    }

    let decoded_path = percent_decode(encoded_path)?;
    let mut relative = PathBuf::new();

    for segment in decoded_path.trim_start_matches('/').split('/') {
        match segment {
            "" | "." => continue,
            ".." => return None,
            _ if segment.contains('\\') || segment.contains('\0') => return None,
            _ => relative.push(segment),
        }
    }

    if relative.as_os_str().is_empty() {
        relative.push("index.html");
    }
    Some(relative)
}

/// Decode the `%XX` escapes of a URL part. Returns `None` for an incomplete
/// escape, or when the result is not UTF-8.
fn percent_decode(value: &str) -> Option<String> {
    let input = value.as_bytes();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;

    while index < input.len() {
        if input[index] == b'%' {
            let high = *input.get(index + 1)?;
            let low = *input.get(index + 2)?;
            output.push(hex_value(high)? * 16 + hex_value(low)?);
            index += 3;
        } else {
            output.push(input[index]);
            index += 1;
        }
    }

    String::from_utf8(output).ok()
}

/// The value of one hexadecimal digit.
fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Whether `value` is a valid device filter: `vvvv:pppp`, `vvvv:*` or `*`.
/// `vvvv` is the USB vendor ID (VID) and `pppp` the product ID (PID).
fn is_valid_serial_port_search(value: &str) -> bool {
    if value == "*" {
        return true;
    }

    let mut parts = value.split(':');
    let Some(vendor) = parts.next() else {
        return false;
    };
    let Some(product) = parts.next() else {
        return false;
    };

    parts.next().is_none() && is_hex_id(vendor) && (product == "*" || is_hex_id(product))
}

/// Whether `value` is a USB ID: one to four hexadecimal digits, with one
/// optional `0x` in front. The page parses the filter in the same way
/// (`src/device-search.ts`).
fn is_hex_id(value: &str) -> bool {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    !digits.is_empty() && digits.len() <= 4 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The embedded file for a request path: the file itself, or the
/// `index.html` of a directory with this name.
fn select_asset(relative: &Path) -> Option<&'static EmbeddedAsset> {
    let key = relative.to_str()?.replace('\\', "/");

    if let Some(asset) = asset_by_path(&key) {
        return Some(asset);
    }

    let directory_index = format!("{}/index.html", key.trim_end_matches('/'));
    if let Some(asset) = asset_by_path(&directory_index) {
        return Some(asset);
    }

    // A path without a file extension can be a route inside the page, so it
    // gets `index.html`. A missing file with an extension still gets a 404,
    // so that a missing asset stays visible.
    if relative.extension().is_none() {
        return asset_by_path("index.html");
    }

    None
}

/// The embedded file with exactly this path.
fn asset_by_path(path: &str) -> Option<&'static EmbeddedAsset> {
    EMBEDDED_ASSETS.iter().find(|asset| asset.path == path)
}

/// Send an embedded file. With `head_only`, send only the header (for `HEAD`).
///
/// The response has a Content Security Policy (CSP): the browser loads
/// scripts only from this server. The browser checks `index.html` again on
/// every load. It caches the other files for one year, because their names
/// contain a hash of their content.
fn write_asset_response(
    stream: &mut TcpStream,
    asset: &EmbeddedAsset,
    head_only: bool,
) -> io::Result<()> {
    let content_type = content_type(Path::new(asset.path));
    let cache_control = if asset.path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };

    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: {cache_control}\r\nX-Content-Type-Options: nosniff\r\nPermissions-Policy: serial=(self)\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        asset.bytes.len()
    )?;

    if !head_only {
        stream.write_all(asset.bytes)?;
    }
    stream.flush()
}

/// Send a short plain-text response, for example an error. With `head_only`,
/// send only the header.
fn write_text_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &str,
    head_only: bool,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nPermissions-Policy: serial=(self)\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head_only {
        stream.write_all(body.as_bytes())?;
    }
    stream.flush()
}

/// Decode the backtrace that `query` describes, and send the result as JSON.
/// The status is 400 for a wrong query, 404 when no ELF file matches, and
/// 500 for other errors.
fn write_backtrace_response(
    stream: &mut TcpStream,
    symbolizer: &Symbolizer,
    query: &str,
    head_only: bool,
) -> io::Result<()> {
    let (status, reason, body) = match backtrace::parse_request(query) {
        None => (
            400,
            "Bad Request",
            backtrace::error_json("expected elf=<sha256 hex>&addresses=<hex>,<hex>,..."),
        ),
        Some(request) => match symbolizer.decode(&request) {
            Ok((elf, frames)) => (200, "OK", backtrace::frames_json(&elf, &frames)),
            Err(error) => {
                let (status, reason) = error.http_status();
                (status, reason, backtrace::error_json(&error.message()))
            }
        },
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head_only {
        stream.write_all(body.as_bytes())?;
    }
    stream.flush()
}

/// Send `/runtime-config.js`, a script that gives the device filter to the
/// page.
fn write_runtime_config_response(
    stream: &mut TcpStream,
    serial_port_search: &str,
    head_only: bool,
) -> io::Result<()> {
    // `main` checks the filter at startup: it contains only hexadecimal
    // digits, `x`, `:` and `*`. So the Rust debug format (`{:?}`), a string
    // in double quotes, is also a safe JavaScript string.
    let body = format!(
        "window.__ESP_AUTOFLASH_CONFIG__ = {{ serialPortSearch: {serial_port_search:?} }};\n"
    );
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nPermissions-Policy: serial=(self)\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head_only {
        stream.write_all(body.as_bytes())?;
    }
    stream.flush()
}

/// The `Content-Type` header value for a file, chosen by its extension.
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("wasm") => "application/wasm",
        Some("bin") => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_root_and_assets() {
        assert_eq!(
            normalized_request_path("/"),
            Some(PathBuf::from("index.html"))
        );
        assert_eq!(
            normalized_request_path("/assets/app.js?v=1"),
            Some(PathBuf::from("assets/app.js"))
        );
    }

    #[test]
    fn decodes_safe_url_segments() {
        assert_eq!(
            normalized_request_path("/firmware%20tools/index.html"),
            Some(PathBuf::from("firmware tools/index.html"))
        );
    }

    #[test]
    fn rejects_traversal_and_invalid_encoding() {
        assert_eq!(normalized_request_path("/../secret"), None);
        assert_eq!(normalized_request_path("/%2e%2e/secret"), None);
        assert_eq!(normalized_request_path("/folder%5csecret"), None);
        assert_eq!(normalized_request_path("/%XX"), None);
    }

    #[test]
    fn assigns_browser_content_types() {
        assert_eq!(
            content_type(Path::new("app.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("firmware.bin")),
            "application/octet-stream"
        );
    }

    #[test]
    fn validates_runtime_usb_search_values() {
        assert!(is_valid_serial_port_search("*"));
        assert!(is_valid_serial_port_search("303a:*"));
        assert!(is_valid_serial_port_search("10c4:ea60"));
        assert!(!is_valid_serial_port_search("USB serial"));
        assert!(!is_valid_serial_port_search("303a:1001:extra"));
        assert!(is_valid_serial_port_search("0x303a:0x1001"));
        assert!(!is_valid_serial_port_search("0x0x303a:*"));
        assert!(!is_valid_serial_port_search("0x:*"));
    }

    #[test]
    fn embeds_a_complete_browser_application() {
        assert!(asset_by_path("index.html").is_some());
        assert!(EMBEDDED_ASSETS
            .iter()
            .any(|asset| asset.path.starts_with("assets/") && asset.path.ends_with(".js")));
        assert!(EMBEDDED_ASSETS
            .iter()
            .any(|asset| asset.path.starts_with("assets/") && asset.path.ends_with(".css")));
    }
}
