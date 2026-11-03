//! Copy-to-clipboard button component.

use leptos::prelude::*;
use web_sys::window;

/// A button that copies the given text to the clipboard with visual feedback.
#[component]
pub fn CopyButton(text: String) -> impl IntoView {
    let (copied, set_copied) = signal(false);
    let text_clone = text.clone();

    let on_click = move |_| {
        if let Some(win) = window() {
            let _ = win.navigator().clipboard().write_text(&text_clone);
            set_copied.set(true);
            // Reset after 2 seconds — best-effort, ignore errors.
            let set_copied = set_copied;
            wasm_bindgen_futures::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(2_000).await;
                set_copied.set(false);
            });
        }
    };

    view! {
        <button
            type="button"
            on:click=on_click
            class="inline-flex items-center px-3 py-1.5 text-xs font-medium rounded border border-gray-300 hover:bg-gray-50 transition-colors"
        >
            {move || if copied.get() { "Copied!" } else { "Copy" }}
        </button>
    }
}
