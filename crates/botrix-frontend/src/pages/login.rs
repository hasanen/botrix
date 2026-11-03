//! Login page — authenticates an existing admin.

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::auth::{LoginRequest, SessionInfo};

use crate::api;

/// Login page component.
#[component]
pub fn LoginPage() -> impl IntoView {
    let (username, set_username) = signal(String::new());
    let (password, set_password) = signal(String::new());
    let (error, set_error) = signal(Option::<String>::None);
    let (submitting, set_submitting) = signal(false);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        set_error.set(None);
        set_submitting.set(true);

        let req = LoginRequest {
            username: username.get(),
            password: password.get(),
        };

        spawn_local(async move {
            match api::post_csrf::<_, SessionInfo>("/api/v1/auth/login", &req).await {
                Ok(_) => {
                    // Full navigation so the auth guard re-checks the session.
                    let _ = window().location().set_href("/");
                }
                Err(api::ApiError::Server { status: 401, .. }) => {
                    set_error.set(Some("Invalid username or password.".to_owned()));
                    set_submitting.set(false);
                }
                Err(e) => {
                    set_error.set(Some(format!("Login failed: {e}")));
                    set_submitting.set(false);
                }
            }
        });
    };

    view! {
        <main class="min-h-screen flex items-center justify-center bg-gray-50">
            <div class="max-w-sm w-full p-6 bg-white rounded-lg shadow">
                <h1 class="text-xl font-semibold mb-6">"Sign in to Botrix"</h1>

                {move || error.get().map(|msg| view! {
                    <div class="mb-4 rounded-md bg-red-50 border border-red-200 p-3 text-sm text-red-700">
                        {msg}
                    </div>
                })}

                <form on:submit=on_submit class="space-y-4">
                    <div>
                        <label class="block text-sm font-medium text-gray-700 mb-1">"Username"</label>
                        <input
                            type="text"
                            class="w-full border border-gray-300 rounded px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                            prop:value=move || username.get()
                            on:input=move |ev| set_username.set(event_target_value(&ev))
                            required
                        />
                    </div>
                    <div>
                        <label class="block text-sm font-medium text-gray-700 mb-1">"Password"</label>
                        <input
                            type="password"
                            class="w-full border border-gray-300 rounded px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                            prop:value=move || password.get()
                            on:input=move |ev| set_password.set(event_target_value(&ev))
                            required
                        />
                    </div>
                    <button
                        type="submit"
                        disabled=move || submitting.get()
                        class="w-full bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium hover:bg-blue-700 disabled:opacity-50"
                    >
                        {move || if submitting.get() { "Signing in..." } else { "Sign in" }}
                    </button>
                </form>
            </div>
        </main>
    }
}
