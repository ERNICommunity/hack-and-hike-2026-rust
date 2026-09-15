mod backtrace;
mod sha256;

use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use backtrace::Symbolizer;

const DEFAULT_BIND: &str = "0.0.0.0";
const DEFAULT_PORT: &str = "8080";
const DEFAULT_SERIAL_PORT_SEARCH: &str = "303a:*";
const MAX_REQUEST_HEADER_BYTES: usize = 32 * 1024;
const BACKTRACE_API_PATH: &str = "/api/backtrace";

struct AppState {
    serial_port_search: String,
    symbolizer: Symbolizer,
}

struct EmbeddedAsset {
    path: &'static str,
    bytes: &'static [u8],
}

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
    println!("Backtraces are decoded with ELF files from: {}", elf_directories.join(", "));
    println!("Serving {} embedded files; no runtime assets are required", EMBEDDED_ASSETS.len());

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
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

fn handle_connection(mut stream: TcpStream, state: &AppState) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
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
        None => write_text_response(&mut stream, 404, "Not Found", "Not found\n", method == "HEAD"),
    }
}

fn read_request_headers(stream: &mut TcpStream) -> io::Result<String> {
    let mut bytes = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 2048];

    loop {
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

fn normalized_request_path(target: &str) -> Option<PathBuf> {
    let encoded_path = target
        .split(|character| character == '?' || character == '#')
        .next()?;
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

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

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

    parts.next().is_none()
        && is_hex_id(vendor.trim_start_matches("0x"))
        && (product == "*" || is_hex_id(product.trim_start_matches("0x")))
}

fn is_hex_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn select_asset(relative: &Path) -> Option<&'static EmbeddedAsset> {
    let key = relative.to_str()?.replace('\\', "/");

    if let Some(asset) = asset_by_path(&key) {
        return Some(asset);
    }

    let directory_index = format!("{}/index.html", key.trim_end_matches('/'));
    if let Some(asset) = asset_by_path(&directory_index) {
        return Some(asset);
    }

    // Keep missing assets visible as 404s, while supporting client-side routes.
    if relative.extension().is_none() {
        return asset_by_path("index.html");
    }

    None
}

fn asset_by_path(path: &str) -> Option<&'static EmbeddedAsset> {
    EMBEDDED_ASSETS.iter().find(|asset| asset.path == path)
}

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

fn write_runtime_config_response(
    stream: &mut TcpStream,
    serial_port_search: &str,
    head_only: bool,
) -> io::Result<()> {
    // The search string is validated as a small ASCII-only grammar at startup,
    // so Rust's quoted debug representation is also a safe JavaScript string.
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
