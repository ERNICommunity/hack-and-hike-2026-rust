//! Decodes firmware backtraces for the device log of the page.
//!
//! When ESP firmware panics, it prints its backtrace as code addresses only.
//! The flashed image has no debug information, so the addresses have no
//! names. The ELF (Executable and Linkable Format) file of the build has this
//! information.
//!
//! The image contains an application descriptor with the SHA-256 hash of the
//! ELF file. The browser reads this hash from the image and sends it with the
//! addresses. This module finds the ELF file with that hash in the Cargo
//! target directories. Then it runs `addr2line` from the ESP toolchain to
//! find the function, file and line of each address.

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
/// The Cargo profile directories that the server searches for ELF files.
const PROFILES: [&str; 2] = ["release", "debug"];

/// The ELF machine number of Xtensa, for example the ESP32-S3. It needs
/// `xtensa-esp-elf-addr2line`.
const EM_XTENSA: u16 = 94;
/// The ELF machine number of RISC-V, for example the ESP32-C3. It needs
/// `riscv32-esp-elf-addr2line`.
const EM_RISCV: u16 = 243;

/// A decode request: the ELF hash and the addresses from the backtrace.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    /// The SHA-256 hash of the ELF file that the firmware was built from.
    pub elf_sha256: [u8; 32],
    /// The code addresses, in the order of the backtrace.
    pub addresses: Vec<u32>,
}

/// One source location of a frame.
///
/// A frame has several locations when the compiler inlined functions, that
/// is, copied them into their caller. The innermost function comes first.
#[derive(Debug, PartialEq, Eq)]
pub struct Location {
    pub function: String,
    pub file: String,
    pub line: u32,
}

/// One backtrace address and its place in the source code. `locations` is
/// empty when the ELF file has no debug information for the address.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub address: u32,
    pub locations: Vec<Location>,
}

/// Why a backtrace could not be decoded. [`DecodeError::message`] gives a
/// text for the device log.
#[derive(Debug)]
pub enum DecodeError {
    /// No ELF file in these directories has the requested hash.
    NoMatchingElf(Vec<PathBuf>),
    /// The ELF file with the hash is neither Xtensa nor RISC-V.
    UnsupportedElf(PathBuf),
    /// The `addr2line` program with this name was not found.
    NoAddr2line(&'static str),
    /// `addr2line` did not start or failed, with this detail.
    Addr2line(String),
}

impl DecodeError {
    /// The HTTP status code and reason phrase for this error.
    pub fn http_status(&self) -> (u16, &'static str) {
        match self {
            DecodeError::NoMatchingElf(_) => (404, "Not Found"),
            _ => (500, "Internal Server Error"),
        }
    }

    /// A short explanation for the device log.
    pub fn message(&self) -> String {
        match self {
            DecodeError::NoMatchingElf(directories) => format!(
                "no ELF file matching the flashed firmware in {} (rebuilt since flashing?)",
                join_paths(directories)
            ),
            DecodeError::UnsupportedElf(path) => {
                format!(
                    "{} is neither an Xtensa nor a RISC-V ELF file",
                    path.display()
                )
            }
            DecodeError::NoAddr2line(tool) => format!(
                "{tool} not found; install the ESP toolchain or set ESP_AUTOFLASH_ADDR2LINE"
            ),
            DecodeError::Addr2line(detail) => format!("addr2line failed: {detail}"),
        }
    }
}

/// The SHA-256 hash of one ELF file, and the file state it belongs to.
struct CachedDigest {
    /// The modification time of the file when it was hashed.
    modified: SystemTime,
    /// The file size in bytes when it was hashed.
    len: u64,
    /// The SHA-256 hash of the file.
    digest: [u8; 32],
}

/// Finds ELF files by hash and decodes backtrace addresses with `addr2line`.
pub struct Symbolizer {
    /// The Cargo target directories to search for ELF files.
    elf_directories: Vec<PathBuf>,
    /// The `addr2line` program from `ESP_AUTOFLASH_ADDR2LINE`. When `None`,
    /// the server searches for the program that matches the ELF file.
    addr2line: Option<PathBuf>,
    /// The hash of each ELF file that was hashed before.
    ///
    /// Hashing a 20 MB ELF file takes some time. So the next panics use the
    /// stored hash, until the modification time or the size of the file
    /// changes.
    digests: Mutex<HashMap<PathBuf, CachedDigest>>,
}

impl Symbolizer {
    /// Read the settings from the environment variables.
    ///
    /// `ESP_AUTOFLASH_ELF_DIRS` lists the Cargo target directories, separated
    /// like in `PATH`. When it is not set, the server searches
    /// `$CARGO_TARGET_DIR` (only when set), `./target` and `../target`.
    /// `ESP_AUTOFLASH_ADDR2LINE` sets one `addr2line` program for all ELF
    /// files.
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

    /// The directories that the server searches for ELF files.
    pub fn elf_directories(&self) -> &[PathBuf] {
        &self.elf_directories
    }

    /// Find the ELF file for `request` and decode its addresses. Returns the
    /// path of the ELF file and one frame for each address.
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
            .args(
                request
                    .addresses
                    .iter()
                    .map(|address| format!("0x{address:08x}")),
            )
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

    /// The ELF file whose SHA-256 hash is `digest`.
    ///
    /// In each ELF directory, the search covers `release/`, `debug/`,
    /// `*/release/` and `*/debug/`, but no deeper directories. Only Xtensa and
    /// RISC-V ELF files are hashed. The newest file is checked first.
    fn find_elf(&self, digest: &[u8; 32]) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        for directory in &self.elf_directories {
            let mut profile_directories: Vec<PathBuf> = PROFILES
                .iter()
                .map(|profile| directory.join(profile))
                .collect();
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
                    // Only firmware ELF files. The same target directories
                    // also contain programs for the computer, and hashing
                    // them would only waste time.
                    let firmware = matches!(elf_machine(&entry.path()), Some(EM_XTENSA | EM_RISCV));
                    if metadata.is_file() && firmware {
                        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                        candidates.push((modified, metadata.len(), entry.path()));
                    }
                }
            }
        }
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));

        candidates
            .into_iter()
            .find(|(modified, len, path)| {
                self.digest_of(path, *modified, *len).as_ref() == Some(digest)
            })
            .map(|(_, _, path)| path)
    }

    /// The SHA-256 hash of the file at `path`, from `digests` when `modified`
    /// and `len` still match. Returns `None` when the file cannot be read.
    fn digest_of(&self, path: &Path, modified: SystemTime, len: u64) -> Option<[u8; 32]> {
        let lock = || {
            self.digests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        };
        if let Some(cached) = lock().get(path) {
            if cached.modified == modified && cached.len == len {
                return Some(cached.digest);
            }
        }
        // Hash without holding the lock, so that parallel requests do not wait
        // for each other.
        let digest = sha256::digest_reader(io::BufReader::new(File::open(path).ok()?)).ok()?;
        lock().insert(
            path.to_owned(),
            CachedDigest {
                modified,
                len,
                digest,
            },
        );
        Some(digest)
    }
}

/// The machine field (`e_machine`) of a little-endian ELF file, or `None` for
/// every other file.
fn elf_machine(path: &Path) -> Option<u16> {
    let mut header = [0_u8; 20];
    File::open(path).ok()?.read_exact(&mut header).ok()?;
    let little_endian = header[5] == 1;
    (header.starts_with(b"\x7fELF") && little_endian)
        .then(|| u16::from_le_bytes([header[18], header[19]]))
}

/// Find the program `name`.
///
/// The search covers the directories in `PATH` first. Then it covers the ESP
/// toolchain that `espup` installs under rustup, highest version name first.
/// The second step is needed because the toolchain is often not in the `PATH`
/// of a non-interactive shell.
fn find_tool(name: &str) -> Option<PathBuf> {
    let on_path = env::var_os("PATH")
        .map(|path| {
            env::split_paths(&path)
                .map(|directory| directory.join(name))
                .collect::<Vec<_>>()
        })
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

    on_path
        .into_iter()
        .chain(in_toolchain)
        .find(|candidate| candidate.is_file())
}

/// Parse `elf=<sha256 hex>&addresses=<hex>,<hex>,...` from a query string.
///
/// Each address has one to eight hexadecimal digits, with or without `0x`.
/// Returns `None` when a part is missing or wrong, or when there are more
/// than `MAX_ADDRESSES` addresses. Unknown keys are ignored.
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
                        let address = address.trim();
                        let digits = address.strip_prefix("0x").unwrap_or(address);
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
    Some(Request {
        elf_sha256: elf_sha256?,
        addresses,
    })
}

/// Parse a SHA-256 hash written as 64 hexadecimal digits.
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

/// Parse the output of `addr2line --functions --inlines --addresses`.
///
/// Each address is on its own line. After it come pairs of lines: a function
/// name, then `file:line`. A pair `??` and `??:0` means that there is no debug
/// information, so it adds no location.
fn parse_addr2line(output: &str) -> Vec<Frame> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut lines = output.lines();
    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if let Some(address) = line
            .strip_prefix("0x")
            .filter(|digits| {
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        {
            frames.push(Frame {
                address,
                locations: Vec::new(),
            });
            continue;
        }
        let Some(frame) = frames.last_mut() else {
            continue;
        };
        let place = lines.next().unwrap_or("??:0").trim_end();
        // `addr2line` writes "file:line (discriminator 2)" when one source
        // line has several blocks of machine code. Keep only "file:line".
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

/// The JSON (JavaScript Object Notation) response body for a decoded
/// backtrace.
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

/// The JSON response body for an error: `{"error": message}`.
pub fn error_json(message: &str) -> String {
    format!("{{\"error\":{}}}\n", json_string(message))
}

/// `value` as a JSON string: in double quotes, with special characters
/// escaped.
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
            control if (control as u32) < 0x20 => {
                quoted.push_str(&format!("\\u{:04x}", control as u32))
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// The paths as one text, separated by commas.
fn join_paths(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "283ce0e9c50be8a9f6529ca5288cfa32a2ed98dfdabbcf68cd32d5ef91b24269";

    #[test]
    fn parses_a_request() {
        let request =
            parse_request(&format!("elf={DIGEST}&addresses=0x4205f584%2C4209D358")).unwrap();
        assert_eq!(request.elf_sha256[0], 0x28);
        assert_eq!(request.elf_sha256[31], 0x69);
        assert_eq!(request.addresses, vec![0x4205f584, 0x4209d358]);
    }

    #[test]
    fn rejects_malformed_requests() {
        assert_eq!(parse_request("addresses=0x40000000"), None);
        assert_eq!(parse_request(&format!("elf={DIGEST}")), None);
        assert_eq!(
            parse_request(&format!("elf={}&addresses=1", &DIGEST[..62])),
            None
        );
        assert_eq!(parse_request(&format!("elf={DIGEST}&addresses=0x1g")), None);
        assert_eq!(
            parse_request(&format!("elf={DIGEST}&addresses=0x123456789")),
            None
        );
        assert_eq!(parse_request(&format!("elf={DIGEST}&addresses=1;rm")), None);
        let too_many = vec!["1"; MAX_ADDRESSES + 1].join(",");
        assert_eq!(
            parse_request(&format!("elf={DIGEST}&addresses={too_many}")),
            None
        );
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
                Frame {
                    address: 0x12,
                    locations: vec![]
                },
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
