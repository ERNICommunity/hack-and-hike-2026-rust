//! Build script that prints hints for common linker errors.
//!
//! The compiled build script has two jobs:
//!
//! - Cargo runs it without arguments before the build. It then tells the
//!   linker to run this same program when linking fails.
//! - The linker runs it with two arguments, for example
//!   `undefined-symbol malloc`. For some known missing symbols, it prints a
//!   hint that names the likely cause.
//!
//! Copied from the esp-generate template.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Arguments mean that the linker runs this program after an error.
    if let Some(kind) = args.get(1) {
        let Some(what) = args.get(2) else {
            std::process::exit(1);
        };

        match kind.as_str() {
            "undefined-symbol" => match what.as_str() {
                "_stack_start" => {
                    eprintln!();
                    eprintln!("💡 Is the linker script `linkall.x` missing?");
                    eprintln!();
                }
                what if what.starts_with("esp_rtos_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `esp-radio` has no scheduler enabled. Make sure you have initialized `esp-rtos` or provided an external scheduler."
                    );
                    eprintln!();
                }
                "free"
                | "malloc"
                | "calloc"
                | "get_free_internal_heap_size"
                | "malloc_internal"
                | "realloc_internal"
                | "calloc_internal"
                | "free_internal" => {
                    eprintln!();
                    eprintln!(
                        "💡 Did you forget the `esp-alloc` dependency or didn't enable the `compat` feature on it?"
                    );
                    eprintln!();
                }
                _ => (),
            },
            _ => std::process::exit(1),
        }

        std::process::exit(0);
    }

    // No arguments: Cargo runs the build script. Register this program as the
    // linker's error-handling script.
    println!(
        "cargo:rustc-link-arg=-Wl,--error-handling-script={}",
        std::env::current_exe()
            .expect("build script path")
            .display()
    );
}
