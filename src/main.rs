#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
fn main() {
    // Only a recognized messaging subcommand enters the CLI. Every other
    // launch, including no arguments, is the existing GUI.
    if main::cli_requested() {
        std::process::exit(main::run_cli());
    }
    main::main();
}

#[cfg(any(target_os = "ios", target_arch = "wasm32"))]
fn main() {main::maverick_main()}

#[cfg(target_os = "android")]
fn main() {}
