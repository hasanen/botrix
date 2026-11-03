//! Inline error display component.

use leptos::prelude::*;

/// Displays a dismissible error message banner.
#[component]
pub fn ErrorBanner(message: String) -> impl IntoView {
    view! {
        <div class="rounded-md bg-red-50 border border-red-200 p-4 text-sm text-red-700">
            {message}
        </div>
    }
}
