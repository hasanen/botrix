//! Loading spinner component.

use leptos::prelude::*;

/// Displays an animated spinner to indicate pending work.
#[component]
pub fn LoadingSpinner() -> impl IntoView {
    view! {
        <div class="flex justify-center items-center p-4">
            <div class="animate-spin rounded-full h-6 w-6 border-2 border-blue-600 border-t-transparent" />
        </div>
    }
}
