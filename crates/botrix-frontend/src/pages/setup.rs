//! First-run setup page — creates the initial admin account.

use leptos::prelude::*;

/// Setup page component.
#[component]
pub fn SetupPage() -> impl IntoView {
    view! {
        <main class="min-h-screen flex items-center justify-center bg-gray-50">
            <div class="max-w-sm w-full p-6 bg-white rounded-lg shadow">
                <h1 class="text-xl font-semibold mb-4">"Welcome to Botrix"</h1>
                <p class="text-sm text-gray-600 mb-6">"Create your admin account to get started."</p>
                // TODO: implement in Phase 3
                <p class="text-xs text-gray-400">"Setup form — Phase 3"</p>
            </div>
        </main>
    }
}
