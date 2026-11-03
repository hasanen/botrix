//! WASM entry point for the Botrix frontend.
//!
//! Trunk compiles this binary to WebAssembly and mounts the Leptos app into
//! the document body.

use botrix_frontend::app::App;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}
