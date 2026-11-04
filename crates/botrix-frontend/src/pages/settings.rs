//! Settings page — password, session timeouts, app info.

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::settings::{
    AppInfo, ChangePasswordRequest, Settings, UpdateSettingsRequest,
};

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::util::parse_session_settings;

/// Minimum length enforced by the server for a new password.
const MIN_PASSWORD_LEN: usize = 8;

/// Settings page component.
#[component]
pub fn SettingsPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <div class="space-y-6">
                <SessionSettingsCard />
                <PasswordCard />
                <AppInfoCard />
            </div>
        </AuthLayout>
    }
}

#[component]
fn SessionSettingsCard() -> impl IntoView {
    let (idle_minutes, set_idle_minutes) = signal(String::new());
    let (absolute_hours, set_absolute_hours) = signal(String::new());
    let (error, set_error) = signal(Option::<String>::None);
    let (notice, set_notice) = signal(Option::<String>::None);

    Effect::new(move |_| {
        spawn_local(async move {
            match api::get::<Settings>("/api/v1/settings").await {
                Ok(s) => {
                    set_idle_minutes.set((s.idle_timeout_secs / 60).to_string());
                    set_absolute_hours.set((s.absolute_lifetime_secs / 3600).to_string());
                }
                Err(e) => {
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    });

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        set_error.set(None);
        set_notice.set(None);

        let (idle_timeout_secs, absolute_lifetime_secs) =
            match parse_session_settings(&idle_minutes.get(), &absolute_hours.get()) {
                Ok(v) => v,
                Err(e) => {
                    set_error.set(Some(e.to_string()));
                    return;
                }
            };

        let req = UpdateSettingsRequest {
            idle_timeout_secs,
            absolute_lifetime_secs,
        };
        spawn_local(async move {
            match api::put_csrf::<_, Settings>("/api/v1/settings", &req).await {
                Ok(_) => set_notice.set(Some(
                    "Saved. New idle timeout applies after the server restarts.".to_owned(),
                )),
                Err(e) => {
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    };

    view! {
        <section class="bg-white rounded-lg shadow p-6" aria-labelledby="session-heading">
            <h2 id="session-heading" class="text-xl font-semibold mb-4">"Session"</h2>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || notice.get().map(|msg| view! {
                <div class="rounded-md bg-green-50 border border-green-200 p-3 text-sm text-green-800 mb-4" role="status">
                    {msg}
                </div>
            })}
            <form on:submit=on_submit class="space-y-4 max-w-sm">
                <div>
                    <label for="idle-timeout" class="block text-sm font-medium text-gray-700 mb-1">
                        "Idle timeout (minutes)"
                    </label>
                    <input
                        id="idle-timeout"
                        type="number"
                        min="1"
                        class="w-full border border-gray-300 rounded px-3 py-2 text-sm"
                        prop:value=move || idle_minutes.get()
                        on:input=move |ev| set_idle_minutes.set(event_target_value(&ev))
                        required
                    />
                </div>
                <div>
                    <label for="absolute-lifetime" class="block text-sm font-medium text-gray-700 mb-1">
                        "Absolute session lifetime (hours)"
                    </label>
                    <input
                        id="absolute-lifetime"
                        type="number"
                        min="1"
                        class="w-full border border-gray-300 rounded px-3 py-2 text-sm"
                        prop:value=move || absolute_hours.get()
                        on:input=move |ev| set_absolute_hours.set(event_target_value(&ev))
                        required
                    />
                </div>
                <button type="submit" class="bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium">
                    "Save"
                </button>
            </form>
        </section>
    }
}

#[component]
fn PasswordCard() -> impl IntoView {
    let (current, set_current) = signal(String::new());
    let (new_password, set_new_password) = signal(String::new());
    let (confirm, set_confirm) = signal(String::new());
    let (error, set_error) = signal(Option::<String>::None);
    let (notice, set_notice) = signal(Option::<String>::None);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        set_error.set(None);
        set_notice.set(None);

        if new_password.get() != confirm.get() {
            set_error.set(Some("New passwords do not match.".to_owned()));
            return;
        }
        if new_password.get().len() < MIN_PASSWORD_LEN {
            set_error.set(Some(format!(
                "New password must be at least {MIN_PASSWORD_LEN} characters."
            )));
            return;
        }

        let req = ChangePasswordRequest {
            current_password: current.get(),
            new_password: new_password.get(),
        };
        spawn_local(async move {
            match api::post_csrf::<_, serde_json::Value>("/api/v1/settings/password", &req).await {
                Ok(_) => {
                    set_current.set(String::new());
                    set_new_password.set(String::new());
                    set_confirm.set(String::new());
                    set_notice.set(Some("Password changed.".to_owned()));
                }
                Err(e) => {
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    };

    view! {
        <section class="bg-white rounded-lg shadow p-6" aria-labelledby="password-heading">
            <h2 id="password-heading" class="text-xl font-semibold mb-4">"Change password"</h2>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || notice.get().map(|msg| view! {
                <div class="rounded-md bg-green-50 border border-green-200 p-3 text-sm text-green-800 mb-4" role="status">
                    {msg}
                </div>
            })}
            <form on:submit=on_submit class="space-y-4 max-w-sm">
                <div>
                    <label for="current-password" class="block text-sm font-medium text-gray-700 mb-1">
                        "Current password"
                    </label>
                    <input
                        id="current-password"
                        type="password"
                        autocomplete="current-password"
                        class="w-full border border-gray-300 rounded px-3 py-2 text-sm"
                        prop:value=move || current.get()
                        on:input=move |ev| set_current.set(event_target_value(&ev))
                        required
                    />
                </div>
                <div>
                    <label for="new-password" class="block text-sm font-medium text-gray-700 mb-1">
                        "New password"
                    </label>
                    <input
                        id="new-password"
                        type="password"
                        autocomplete="new-password"
                        class="w-full border border-gray-300 rounded px-3 py-2 text-sm"
                        prop:value=move || new_password.get()
                        on:input=move |ev| set_new_password.set(event_target_value(&ev))
                        required
                    />
                </div>
                <div>
                    <label for="confirm-password" class="block text-sm font-medium text-gray-700 mb-1">
                        "Confirm new password"
                    </label>
                    <input
                        id="confirm-password"
                        type="password"
                        autocomplete="new-password"
                        class="w-full border border-gray-300 rounded px-3 py-2 text-sm"
                        prop:value=move || confirm.get()
                        on:input=move |ev| set_confirm.set(event_target_value(&ev))
                        required
                    />
                </div>
                <button type="submit" class="bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium">
                    "Change password"
                </button>
            </form>
        </section>
    }
}

#[component]
fn AppInfoCard() -> impl IntoView {
    let (info, set_info) = signal(Option::<AppInfo>::None);

    Effect::new(move |_| {
        spawn_local(async move {
            match api::get::<AppInfo>("/api/v1/settings/info").await {
                Ok(i) => set_info.set(Some(i)),
                Err(e) => {
                    redirect_if_unauthorized(&e);
                }
            }
        });
    });

    view! {
        <section class="bg-white rounded-lg shadow p-6" aria-labelledby="about-heading">
            <h2 id="about-heading" class="text-xl font-semibold mb-4">"About"</h2>
            {move || info.get().map(|i| view! {
                <dl class="text-sm space-y-1">
                    <div class="flex gap-2">
                        <dt class="text-gray-600">"Version:"</dt>
                        <dd>{i.version}</dd>
                    </div>
                    <div class="flex gap-2">
                        <dt class="text-gray-600">"Build:"</dt>
                        <dd>{i.build_hash.unwrap_or_else(|| "unknown".to_owned())}</dd>
                    </div>
                </dl>
            })}
        </section>
    }
}
