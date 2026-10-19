#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
fn main() {main::main()}

#[cfg(any(target_os = "ios", target_arch = "wasm32"))]
fn main() {main::maverick_main()}

#[cfg(target_os = "android")]
fn main() {}
