//! Dashboard page — minimal authenticated landing for the login slice.
//!
//! Checks the session via `GET /api/v1/auth/me`. Unauthenticated visitors are
//! redirected to `/login`. Authenticated visitors see their username and a
//! logout button. (Full dashboard content arrives in Phase 8.)

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::auth::SessionInfo;

use crate::api;

/// Dashboard page component.
#[component]
pub fn DashboardPage() -> impl IntoView {
    let (username, set_username) = signal(Option::<String>::None);
    let (loading, set_loading) = signal(true);

    // On mount, check the session.
    Effect::new(move |_| {
        spawn_local(async move {
            match api::get::<SessionInfo>("/api/v1/auth/me").await {
                Ok(info) => {
                    set_username.set(Some(info.username));
                    set_loading.set(false);
                }
                Err(_) => {
                    // Not authenticated — send to login.
                    let _ = window().location().set_href("/login");
                }
            }
        });
    });

    let on_logout = move |_| {
        spawn_local(async move {
            let _ = api::post_csrf::<_, serde_json::Value>("/api/v1/auth/logout", &()).await;
            let _ = window().location().set_href("/login");
        });
    };

    view! {
        <main class="min-h-screen bg-gray-50">
            <header class="bg-white border-b border-gray-200">
                <div class="max-w-5xl mx-auto px-6 py-4 flex items-center justify-between">
                    <h1 class="text-lg font-semibold">"Botrix"</h1>
                    {move || username.get().map(|u| view! {
                        <div class="flex items-center gap-4 text-sm">
                            <span class="text-gray-600">"Signed in as " <b>{u}</b></span>
                            <button
                                on:click=on_logout
                                class="px-3 py-1.5 border border-gray-300 rounded hover:bg-gray-50"
                            >
                                "Log out"
                            </button>
                        </div>
                    })}
                </div>
            </header>

            <div class="max-w-5xl mx-auto px-6 py-10">
                {move || if loading.get() {
                    view! { <p class="text-sm text-gray-500">"Loading..."</p> }.into_any()
                } else {
                    view! {
                        <div class="bg-white rounded-lg shadow p-6">
                            <h2 class="text-xl font-semibold mb-2">"Dashboard"</h2>
                            <p class="text-sm text-gray-600">
                                "You are signed in. The full dashboard, Matrix, webhooks, "
                                "and settings pages arrive in Phase 8."
                            </p>
                        </div>
                    }.into_any()
                }}
            </div>
        </main>
    }
}
