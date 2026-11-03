//! Login page.

use leptos::prelude::*;

/// Login page component.
#[component]
pub fn LoginPage() -> impl IntoView {
    view! {
        <main class="min-h-screen flex items-center justify-center bg-gray-50">
            <div class="max-w-sm w-full p-6 bg-white rounded-lg shadow">
                <h1 class="text-xl font-semibold mb-6">"Sign in to Botrix"</h1>
                // TODO: implement in Phase 3
                <p class="text-xs text-gray-400">"Login form — Phase 3"</p>
            </div>
        </main>
    }
}
