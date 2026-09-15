//! Decodes firmware backtraces for the serial console.
//!
//! A panicking ESP firmware prints its backtrace as bare code addresses; the
//! flashed image has no debug information. The ELF file it was built from
//! does, and ESP-IDF images carry that file's SHA-256 in their application
//! descriptor. The browser reads the hash from the image it flashed and sends
//! it with the addresses; this module finds the ELF with that hash in the
//! Cargo build directories and looks the addresses up with `addr2line` from
//! the ESP toolchain.

use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::SystemTime;

use crate::sha256;

/// The most addresses one request may ask for.
const MAX_ADDRESSES: usize = 64;
/// Cargo profile directories searched for ELF files.
const PROFILES: [&str; 2] = ["release", "debug"];

/// ELF machine numbers and the ESP toolchain that understands each.
const EM_XTENSA: u16 = 94;
const EM_RISCV: u16 = 243;

/// A decode request: the ELF hash and the addresses from the backtrace.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub elf_sha256: [u8; 32],
    pub addresses: Vec<u32>,
}

/// One source location of a frame. A frame has several when the compiler
/// inlined functions: the innermost comes first.
#[derive(Debug, PartialEq, Eq)]
pub struct Location {
    pub function: String,
    pub file: String,
    pub line: u32,
}

/// One backtrace address and where it is in the source; `locations` is empty
/// when the ELF has no debug information for it.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub address: u32,
    pub locations: Vec<Location>,
}

/// Why a backtrace could not be decoded, phrased for the serial console.
#[derive(Debug)]
pub enum DecodeError {
    NoMatchingElf(Vec<PathBuf>),
    UnsupportedElf(PathBuf),
    NoAddr2line(&'static str),
    Addr2line(String),
}

impl DecodeError {
    pub fn http_status(&self) -> (u16, &'static str) {
        match self {
            DecodeError::NoMatchingElf(_) => (404, "Not Found"),
            _ => (500, "Internal Server Error"),
        }
    }

    pub fn message(&self) -> String {
        match self {
            DecodeError::NoMatchingElf(directories) => format!(
                "no ELF file matching the flashed firmware in {} (rebuilt since flashing?)",
                join_paths(directories)
            ),
            DecodeError::UnsupportedElf(path) => {
                format!("{} is neither an Xtensa nor a RISC-V ELF file", path.display())
            }
            DecodeError::NoAddr2line(tool) => format!(
                "{tool} not found; install the ESP toolchain or set ESP_AUTOFLASH_ADDR2LINE"
            ),
            DecodeError::Addr2line(detail) => format!("addr2line failed: {detail}"),
        }
    }
}

struct CachedDigest {
    modified: SystemTime,
    len: u64,
    digest: [u8; 32],
}

pub struct Symbolizer {
    elf_directories: Vec<PathBuf>,
    addr2line: Option<PathBuf>,
    /// Hashing a 20 MB ELF takes a moment; repeat panics reuse the result
    /// until the file changes.
    digests: Mutex<HashMap<PathBuf, CachedDigest>>,
}

impl Symbolizer {
    /// Configure from `ESP_AUTOFLASH_ELF_DIRS` (Cargo target directories,
    /// separated like `PATH`) and `ESP_AUTOFLASH_ADDR2LINE`. Without the first,
    /// `CARGO_TARGET_DIR`, `./target` and `../target` are searched.
    pub fn from_env() -> Self {
        let elf_directories = match env::var_os("ESP_AUTOFLASH_ELF_DIRS") {
            Some(value) => env::split_paths(&value).collect(),
            None => env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .into_iter()
                .chain([PathBuf::from("target"), PathBuf::from("../target")])
                .collect(),
        };
        Symbolizer {
            elf_directories,
            addr2line: env::var_os("ESP_AUTOFLASH_ADDR2LINE").map(PathBuf::from),
            digests: Mutex::new(HashMap::new()),
        }
    }

    pub fn elf_directories(&self) -> &[PathBuf] {
        &self.elf_directories
    }

    pub fn decode(&self, request: &Request) -> Result<(PathBuf, Vec<Frame>), DecodeError> {
        let elf = self
            .find_elf(&request.elf_sha256)
            .ok_or_else(|| DecodeError::NoMatchingElf(self.elf_directories.clone()))?;
        let tool_name = match elf_machine(&elf) {
            Some(EM_XTENSA) => "xtensa-esp-elf-addr2line",
            Some(EM_RISCV) => "riscv32-esp-elf-addr2line",
            _ => return Err(DecodeError::UnsupportedElf(elf)),
        };
        let tool = match &self.addr2line {
            Some(tool) => tool.clone(),
            None => find_tool(tool_name).ok_or(DecodeError::NoAddr2line(tool_name))?,
        };

        let output = Command::new(&tool)
            .arg("--exe")
            .arg(&elf)
            .args(["--functions", "--inlines", "--addresses", "--demangle"])
            .args(request.addresses.iter().map(|address| format!("0x{address:08x}")))
            .output()
            .map_err(|error| DecodeError::Addr2line(format!("{}: {error}", tool.display())))?;
        if !output.status.success() {
            return Err(DecodeError::Addr2line(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let frames = parse_addr2line(&String::from_utf8_lossy(&output.stdout));
        Ok((elf, frames))
    }

    /// The ELF file whose SHA-256 is `digest`, newest candidates first.
    fn find_elf(&self, digest: &[u8; 32]) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        for directory in &self.elf_directories {
            let mut profile_directories: Vec<PathBuf> =
                PROFILES.iter().map(|profile| directory.join(profile)).collect();
            if let Ok(entries) = fs::read_dir(directory) {
                for entry in entries.flatten() {
                    for profile in PROFILES {
                        profile_directories.push(entry.path().join(profile));
                    }
                }
            }
            for profile_directory in profile_directories {
                let Ok(entries) = fs::read_dir(&profile_directory) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let Ok(metadata) = entry.metadata() else {
                        continue;
                    };
                    if metadata.is_file() && elf_machine(&entry.path()).is_some() {
                        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                        candidates.push((modified, metadata.len(), entry.path()));
                    }
                }
            }
        }
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));

        candidates
            .into_iter()
            .find(|(modified, len, path)| self.digest_of(path, *modified, *len).as_ref() == Some(digest))
            .map(|(_, _, path)| path)
    }

    fn digest_of(&self, path: &Path, modified: SystemTime, len: u64) -> Option<[u8; 32]> {
        let mut digests = self.digests.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = digests.get(path) {
            if cached.modified == modified && cached.len == len {
                return Some(cached.digest);
            }
        }
        let digest = sha256::digest_reader(io::BufReader::new(File::open(path).ok()?)).ok()?;
        digests.insert(path.to_owned(), CachedDigest { modified, len, digest });
        Some(digest)
    }
}

/// The machine field of a little-endian ELF file, or `None` for other files.
fn elf_machine(path: &Path) -> Option<u16> {
    let mut header = [0_u8; 20];
    File::open(path).ok()?.read_exact(&mut header).ok()?;
    let little_endian = header[5] == 1;
    (header.starts_with(b"\x7fELF") && little_endian).then(|| u16::from_le_bytes([header[18], header[19]]))
}

/// `name` on the `PATH`, or in the ESP toolchain that `espup` installs under
/// rustup; non-interactive shells often lack the toolchain on their `PATH`.
fn find_tool(name: &str) -> Option<PathBuf> {
    let on_path = env::var_os("PATH")
        .map(|path| env::split_paths(&path).map(|directory| directory.join(name)).collect::<Vec<_>>())
        .unwrap_or_default();

    let prefix = name.trim_end_matches("-addr2line");
    let rustup_homes = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .into_iter()
        .chain(env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")));
    let mut in_toolchain = Vec::new();
    for rustup_home in rustup_homes {
        let versions = rustup_home.join("toolchains/esp").join(prefix);
        if let Ok(entries) = fs::read_dir(versions) {
            for entry in entries.flatten() {
                in_toolchain.push(entry.path().join(prefix).join("bin").join(name));
            }
        }
    }
    in_toolchain.sort();
    in_toolchain.reverse();

    on_path.into_iter().chain(in_toolchain).find(|candidate| candidate.is_file())
}

/// Parse `elf=<sha256 hex>&addresses=<hex>,<hex>,...` from a query string.
pub fn parse_request(query: &str) -> Option<Request> {
    let mut elf_sha256 = None;
    let mut addresses = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        let value = crate::percent_decode(value)?;
        match key {
            "elf" => elf_sha256 = Some(parse_digest(&value)?),
            "addresses" => {
                let parsed = value
                    .split(',')
                    .map(|address| {
                        let digits = address.trim().trim_start_matches("0x");
                        (1..=8)
                            .contains(&digits.len())
                            .then(|| u32::from_str_radix(digits, 16).ok())
                            .flatten()
                    })
                    .collect::<Option<Vec<u32>>>()?;
                addresses = Some(parsed);
            }
            _ => {}
        }
    }
    let addresses = addresses.filter(|list| (1..=MAX_ADDRESSES).contains(&list.len()))?;
    Some(Request { elf_sha256: elf_sha256?, addresses })
}

fn parse_digest(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(digest)
}

/// Parse the output of `addr2line --functions --inlines --addresses`: each
/// address on its own line, followed by function and `file:line` line pairs.
fn parse_addr2line(output: &str) -> Vec<Frame> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut lines = output.lines();
    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if let Some(address) = line
            .strip_prefix("0x")
            .filter(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        {
            frames.push(Frame { address, locations: Vec::new() });
            continue;
        }
        let Some(frame) = frames.last_mut() else {
            continue;
        };
        let place = lines.next().unwrap_or("??:0").trim_end();
        // "file:line (discriminator 2)" when a line has several code blocks.
        let place = place.split(" (discriminator").next().unwrap_or(place);
        let (file, line_number) = place.rsplit_once(':').unwrap_or((place, "0"));
        if line == "??" && file == "??" {
            continue;
        }
        frame.locations.push(Location {
            function: line.to_owned(),
            file: file.to_owned(),
            line: line_number.parse().unwrap_or(0),
        });
    }
    frames
}

/// The JSON response body for a decoded backtrace.
pub fn frames_json(elf: &Path, frames: &[Frame]) -> String {
    let frames: Vec<String> = frames
        .iter()
        .map(|frame| {
            let locations: Vec<String> = frame
                .locations
                .iter()
                .map(|location| {
                    format!(
                        "{{\"function\":{},\"file\":{},\"line\":{}}}",
                        json_string(&location.function),
                        json_string(&location.file),
                        location.line
                    )
                })
                .collect();
            format!(
                "{{\"address\":\"0x{:08x}\",\"locations\":[{}]}}",
                frame.address,
                locations.join(",")
            )
        })
        .collect();
    format!(
        "{{\"elf\":{},\"frames\":[{}]}}\n",
        json_string(&elf.to_string_lossy()),
        frames.join(",")
    )
}

pub fn error_json(message: &str) -> String {
    format!("{{\"error\":{}}}\n", json_string(message))
}

fn json_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            control if (control as u32) < 0x20 => quoted.push_str(&format!("\\u{:04x}", control as u32)),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn join_paths(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths.iter().map(|path| path.display().to_string()).collect();
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "283ce0e9c50be8a9f6529ca5288cfa32a2ed98dfdabbcf68cd32d5ef91b24269";

    #[test]
    fn parses_a_request() {
        let request = parse_request(&format!("elf={DIGEST}&addresses=0x4205f584%2C4209D358")).unwrap();
        assert_eq!(request.elf_sha256[0], 0x28);
        assert_eq!(request.elf_sha256[31], 0x69);
        assert_eq!(request.addresses, vec![0x4205f584, 0x4209d358]);
    }

    #[test]
    fn rejects_malformed_requests() {
        assert_eq!(parse_request("addresses=0x40000000"), None);
        assert_eq!(parse_request(&format!("elf={DIGEST}")), None);
        assert_eq!(parse_request(&format!("elf={}&addresses=1", &DIGEST[..62])), None);
        assert_eq!(parse_request(&format!("elf={DIGEST}&addresses=0x1g")), None);
        assert_eq!(parse_request(&format!("elf={DIGEST}&addresses=0x123456789")), None);
        assert_eq!(parse_request(&format!("elf={DIGEST}&addresses=1;rm")), None);
        let too_many = vec!["1"; MAX_ADDRESSES + 1].join(",");
        assert_eq!(parse_request(&format!("elf={DIGEST}&addresses={too_many}")), None);
    }

    #[test]
    fn parses_inlined_and_unknown_frames() {
        let output = "0x4205f584\n\
            panic_backtrace::band_name\n\
            /src/bin/panic_backtrace.rs:89\n\
            panic_backtrace::on_tap\n\
            /src/bin/panic_backtrace.rs:78 (discriminator 1)\n\
            0x00000012\n\
            ??\n\
            ??:0\n";
        assert_eq!(
            parse_addr2line(output),
            vec![
                Frame {
                    address: 0x4205f584,
                    locations: vec![
                        Location {
                            function: "panic_backtrace::band_name".into(),
                            file: "/src/bin/panic_backtrace.rs".into(),
                            line: 89,
                        },
                        Location {
                            function: "panic_backtrace::on_tap".into(),
                            file: "/src/bin/panic_backtrace.rs".into(),
                            line: 78,
                        },
                    ],
                },
                Frame { address: 0x12, locations: vec![] },
            ]
        );
    }

    #[test]
    fn writes_escaped_json() {
        let frames = vec![Frame {
            address: 0x42000000,
            locations: vec![Location {
                function: "<a as b>::c".into(),
                file: "C:\\src\\\"x\".rs".into(),
                line: 7,
            }],
        }];
        assert_eq!(
            frames_json(Path::new("/t/app"), &frames),
            "{\"elf\":\"/t/app\",\"frames\":[{\"address\":\"0x42000000\",\"locations\":[{\"function\":\"<a as b>::c\",\"file\":\"C:\\\\src\\\\\\\"x\\\".rs\",\"line\":7}]}]}\n"
        );
        assert_eq!(error_json("a\u{1}b"), "{\"error\":\"a\\u0001b\"}\n");
    }
}
