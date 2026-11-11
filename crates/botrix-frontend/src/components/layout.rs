//! Authenticated page shell: auth guard, navigation, and logout.

use botrix_common::api::auth::SessionInfo;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

use crate::api;

/// Navigation entries as `(path, label)`.
const NAV: [(&str, &str); 6] = [
    ("/", "Dashboard"),
    ("/matrix", "Matrix"),
    ("/webhooks", "Webhooks"),
    ("/triggers", "Triggers"),
    ("/logs", "Logs"),
    ("/settings", "Settings"),
];

/// Send the browser to the login page.
pub fn redirect_to_login() {
    let _ = window().location().set_href("/login");
}

/// If `err` is a 401, redirect to the login page and return `true`.
///
/// Pages call this on every failed API call so an expired session sends the
/// user to `/login` instead of showing a confusing error.
pub fn redirect_if_unauthorized(err: &api::ApiError) -> bool {
    if err.is_unauthorized() {
        redirect_to_login();
        true
    } else {
        false
    }
}

/// Wraps a page with the auth guard and the app header.
///
/// On mount it checks `GET /api/v1/auth/me`; unauthenticated visitors are sent
/// to `/login`. Children are rendered only once the session is confirmed.
#[component]
pub fn AuthLayout(children: ChildrenFn) -> impl IntoView {
    let (username, set_username) = signal(Option::<String>::None);

    Effect::new(move |_| {
        spawn_local(async move {
            match api::get::<SessionInfo>("/api/v1/auth/me").await {
                Ok(info) => set_username.set(Some(info.username)),
                Err(_) => redirect_to_login(),
            }
        });
    });

    let on_logout = move |_| {
        spawn_local(async move {
            let _ = api::post_csrf::<_, serde_json::Value>("/api/v1/auth/logout", &()).await;
            redirect_to_login();
        });
    };

    view! {
        <div class="min-h-screen bg-gray-50">
            <header class="bg-white border-b border-gray-200">
                <div class="max-w-5xl mx-auto px-6 py-4 flex items-center justify-between">
                    <div class="flex items-center gap-6">
                        <span class="text-lg font-semibold">"Botrix"</span>
                        <nav aria-label="Main" class="flex items-center gap-4 text-sm">
                            {NAV
                                .iter()
                                .map(|(path, label)| {
                                    view! { <A href=*path attr:class="nav-link">{*label}</A> }
                                })
                                .collect_view()}
                        </nav>
                    </div>
                    {move || username.get().map(|u| view! {
                        <div class="flex items-center gap-4 text-sm">
                            <span class="text-gray-600">"Signed in as " <b>{u}</b></span>
                            <button
                                type="button"
                                on:click=on_logout
                                class="px-3 py-1.5 border border-gray-300 rounded hover:bg-gray-50"
                            >
                                "Log out"
                            </button>
                        </div>
                    })}
                </div>
            </header>

            <main class="max-w-5xl mx-auto px-6 py-10">
                <Show
                    when=move || username.get().is_some()
                    fallback=|| view! { <crate::components::loading_spinner::LoadingSpinner /> }
                >
                    {children()}
                </Show>
            </main>
        </div>
    }
}
