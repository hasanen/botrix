//! Matrix account and rooms management page.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::matrix::{
    ConnectPollResponse, ConnectPollStatus, ConnectResponse, DisconnectRequest, MatrixStatus,
    RecoveryKeyRequest, RoomInfo, TestMessageRequest,
};
use botrix_common::types::{MatrixConnectionStatus, RoomMembershipState};

use crate::api;
use crate::components::copy_button::CopyButton;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::modal::ConfirmModal;
use crate::components::status_badge::StatusBadge;
use crate::util::{connection_badge, format_unix, membership_badge};

/// Delay between polls of the OAuth device-grant outcome.
const POLL_INTERVAL_MS: u32 = 5_000;

/// Matrix page component.
#[component]
pub fn MatrixPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <MatrixContent />
        </AuthLayout>
    }
}

#[component]
fn MatrixContent() -> impl IntoView {
    let (status, set_status) = signal(Option::<MatrixStatus>::None);
    let (rooms, set_rooms) = signal(Vec::<RoomInfo>::new());
    let (error, set_error) = signal(Option::<String>::None);
    let (notice, set_notice) = signal(Option::<String>::None);
    let (reload, set_reload) = signal(0_u32);

    // Device-grant flow state.
    let (grant, set_grant) = signal(Option::<ConnectResponse>::None);
    let alive = Arc::new(AtomicBool::new(true));
    {
        let alive = alive.clone();
        on_cleanup(move || alive.store(false, Ordering::Relaxed));
    }

    let (confirm_disconnect, set_confirm_disconnect) = signal(false);
    let (recovery_key, set_recovery_key) = signal(String::new());
    let (test_destination, set_test_destination) = signal(String::new());

    let report = move |e: api::ApiError| {
        if !redirect_if_unauthorized(&e) {
            set_error.set(Some(e.user_message()));
        }
    };

    Effect::new(move |_| {
        reload.get();
        spawn_local(async move {
            match api::get::<MatrixStatus>("/api/v1/matrix/status").await {
                Ok(s) => set_status.set(Some(s)),
                Err(e) => report(e),
            }
            match api::get::<Vec<RoomInfo>>("/api/v1/matrix/rooms").await {
                Ok(list) => {
                    if test_destination.get_untracked().is_empty() {
                        if let Some(first) = list.iter().find(|r| r.enabled) {
                            set_test_destination.set(first.name.clone());
                        }
                    }
                    set_rooms.set(list);
                }
                Err(e) => report(e),
            }
        });
    });

    let on_connect = {
        let alive = alive.clone();
        move |_| {
            set_error.set(None);
            set_notice.set(None);
            let alive = alive.clone();
            spawn_local(async move {
                match api::post_csrf::<_, ConnectResponse>("/api/v1/matrix/connect", &()).await {
                    Ok(resp) => {
                        set_grant.set(Some(resp));
                        poll_until_done(alive, set_grant, set_error, set_notice, set_reload).await;
                    }
                    Err(e) => report(e),
                }
            });
        }
    };

    let on_disconnect_confirmed = move |()| {
        set_confirm_disconnect.set(false);
        spawn_local(async move {
            let req = DisconnectRequest { confirm: true };
            match api::post_csrf::<_, serde_json::Value>("/api/v1/matrix/disconnect", &req).await {
                Ok(_) => {
                    set_notice.set(Some("Disconnected.".to_owned()));
                    set_reload.update(|n| *n += 1);
                }
                Err(e) => report(e),
            }
        });
    };

    let on_recovery_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        set_error.set(None);
        set_notice.set(None);
        let req = RecoveryKeyRequest {
            recovery_key: recovery_key.get(),
        };
        spawn_local(async move {
            match api::post_csrf::<_, serde_json::Value>("/api/v1/matrix/recovery-key", &req).await
            {
                Ok(_) => {
                    // Never keep the key around once it has been submitted.
                    set_recovery_key.set(String::new());
                    set_notice.set(Some("Recovery key accepted.".to_owned()));
                    set_reload.update(|n| *n += 1);
                }
                Err(e) => report(e),
            }
        });
    };

    let on_join = move |name: String| {
        set_error.set(None);
        set_notice.set(None);
        spawn_local(async move {
            let path = format!("/api/v1/matrix/rooms/{name}/join");
            match api::post_csrf::<_, serde_json::Value>(&path, &()).await {
                Ok(_) => {
                    set_notice.set(Some(format!("Joined '{name}'.")));
                    set_reload.update(|n| *n += 1);
                }
                Err(e) => report(e),
            }
        });
    };

    let on_test = move |_| {
        set_error.set(None);
        set_notice.set(None);
        let req = TestMessageRequest {
            destination: test_destination.get(),
        };
        spawn_local(async move {
            match api::post_csrf::<_, serde_json::Value>("/api/v1/matrix/test-message", &req).await
            {
                Ok(_) => set_notice.set(Some("Test message sent.".to_owned())),
                Err(e) => report(e),
            }
        });
    };

    view! {
        <div class="space-y-6">
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || notice.get().map(|msg| view! {
                <div class="rounded-md bg-green-50 border border-green-200 p-3 text-sm text-green-800" role="status">
                    {msg}
                </div>
            })}

            <section class="bg-white rounded-lg shadow p-6" aria-labelledby="account-heading">
                <h2 id="account-heading" class="text-xl font-semibold mb-4">"Account"</h2>
                {move || match status.get() {
                    None => view! { <LoadingSpinner /> }.into_any(),
                    Some(s) => {
                        let (badge, label) = connection_badge(&s.status);
                        let connected = s.status == MatrixConnectionStatus::Connected;
                        let (e2ee_badge, e2ee_label) = if s.e2ee_bootstrapped {
                            (crate::components::status_badge::BadgeStatus::Connected, "E2EE ready")
                        } else {
                            (crate::components::status_badge::BadgeStatus::Connecting, "E2EE not set up")
                        };
                        view! {
                            <div class="space-y-3 text-sm">
                                <div class="flex items-center gap-2">
                                    <StatusBadge status=badge label=label.to_string() />
                                    {connected.then(|| view! {
                                        <StatusBadge status=e2ee_badge label=e2ee_label.to_string() />
                                    })}
                                </div>
                                {s.user_id.clone().map(|u| view! {
                                    <div class="text-gray-600">"Account: " <span class="text-gray-900">{u}</span></div>
                                })}
                                {s.homeserver.clone().map(|h| view! {
                                    <div class="text-gray-600">"Homeserver: " <span class="text-gray-900">{h}</span></div>
                                })}
                                {s.connected_at.map(|t| view! {
                                    <div class="text-gray-600">"Connected: " <span class="text-gray-900">{format_unix(t)}</span></div>
                                })}
                                <div class="flex gap-2">
                                    {(!connected).then(|| view! {
                                        <button
                                            type="button"
                                            on:click=on_connect.clone()
                                            class="bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium"
                                        >
                                            "Connect account"
                                        </button>
                                    })}
                                    {connected.then(|| view! {
                                        <button
                                            type="button"
                                            on:click=move |_| set_confirm_disconnect.set(true)
                                            class="px-3 py-2 border border-gray-300 rounded text-sm text-red-700 hover:bg-gray-50"
                                        >
                                            "Disconnect"
                                        </button>
                                    })}
                                </div>
                            </div>
                        }.into_any()
                    }
                }}

                {move || grant.get().map(|g| view! {
                    <div class="mt-4 rounded-md bg-blue-50 border border-blue-200 p-4 text-sm" role="status">
                        <p class="mb-2">"Open this address and enter the code to authorize Botrix:"</p>
                        <div class="flex items-center gap-2 mb-2">
                            <a href=g.verification_uri.clone() target="_blank" rel="noopener noreferrer" class="underline break-all">
                                {g.verification_uri.clone()}
                            </a>
                            <CopyButton text=g.verification_uri.clone() />
                        </div>
                        <div class="flex items-center gap-2">
                            <code class="text-lg font-semibold">{g.user_code.clone()}</code>
                            <CopyButton text=g.user_code.clone() />
                        </div>
                        <p class="mt-2 text-gray-600">
                            "Waiting for authorization… expires in about "
                            {g.expires_in / 60} " minutes."
                        </p>
                    </div>
                })}
            </section>

            // Recovery key prompt: only when connected without E2EE bootstrapped.
            {move || status.get().filter(|s| {
                s.status == MatrixConnectionStatus::Connected && !s.e2ee_bootstrapped
            }).map(|_| view! {
                <section class="bg-white rounded-lg shadow p-6" aria-labelledby="recovery-heading">
                    <h2 id="recovery-heading" class="text-xl font-semibold mb-2">"Encryption recovery key"</h2>
                    <p class="text-sm text-gray-600 mb-4">
                        "Enter your recovery key so Botrix can send messages to encrypted rooms. "
                        "The key is used once and is not stored."
                    </p>
                    <form on:submit=on_recovery_submit class="flex items-end gap-2 max-w-lg">
                        <div class="w-full">
                            <label for="recovery-key" class="block text-sm font-medium text-gray-700 mb-1">
                                "Recovery key"
                            </label>
                            <input
                                id="recovery-key"
                                type="password"
                                autocomplete="off"
                                class="w-full border border-gray-300 rounded px-3 py-2 text-sm"
                                prop:value=move || recovery_key.get()
                                on:input=move |ev| set_recovery_key.set(event_target_value(&ev))
                                required
                            />
                        </div>
                        <button type="submit" class="bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium">
                            "Submit"
                        </button>
                    </form>
                </section>
            })}

            <section class="bg-white rounded-lg shadow p-6" aria-labelledby="rooms-heading">
                <h2 id="rooms-heading" class="text-xl font-semibold mb-4">"Destinations"</h2>
                <table class="w-full text-sm text-left">
                    <thead>
                        <tr class="text-gray-500 border-b border-gray-200">
                            <th class="py-2 pr-4">"Name"</th>
                            <th class="py-2 pr-4">"Room ID"</th>
                            <th class="py-2 pr-4">"Room name"</th>
                            <th class="py-2 pr-4">"Membership"</th>
                            <th class="py-2 pr-4">"Encrypted"</th>
                            <th class="py-2">"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || rooms.get().into_iter().map(|r| {
                            let (badge, label) = membership_badge(&r.membership);
                            let invited = r.membership == RoomMembershipState::Invited;
                            let name_for_join = r.name.clone();
                            view! {
                                <tr class="border-b border-gray-200">
                                    <td class="py-2 pr-4">{r.name.clone()}</td>
                                    <td class="py-2 pr-4"><code class="text-xs">{r.room_id.clone()}</code></td>
                                    <td class="py-2 pr-4">{r.display_name.clone().unwrap_or_else(|| "—".to_owned())}</td>
                                    <td class="py-2 pr-4"><StatusBadge status=badge label=label.to_string() /></td>
                                    <td class="py-2 pr-4">{if r.encrypted { "Yes" } else { "No" }}</td>
                                    <td class="py-2">
                                        {invited.then(|| view! {
                                            <button
                                                type="button"
                                                on:click=move |_| on_join(name_for_join.clone())
                                                class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50"
                                            >
                                                "Accept invite"
                                            </button>
                                        })}
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
            </section>

            <section class="bg-white rounded-lg shadow p-6" aria-labelledby="test-heading">
                <h2 id="test-heading" class="text-xl font-semibold mb-4">"Send test message"</h2>
                <div class="flex items-end gap-2">
                    <div>
                        <label for="test-destination" class="block text-sm font-medium text-gray-700 mb-1">
                            "Destination"
                        </label>
                        <select
                            id="test-destination"
                            class="border border-gray-300 rounded px-3 py-2 text-sm"
                            prop:value=move || test_destination.get()
                            on:change=move |ev| set_test_destination.set(event_target_value(&ev))
                        >
                            {move || rooms.get().into_iter().filter(|r| r.enabled).map(|r| {
                                let name = r.name.clone();
                                view! { <option value=name.clone()>{name.clone()}</option> }
                            }).collect_view()}
                        </select>
                    </div>
                    <button
                        type="button"
                        on:click=on_test
                        class="bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium"
                    >
                        "Send"
                    </button>
                </div>
            </section>

            <ConfirmModal
                open=Signal::derive(move || confirm_disconnect.get())
                title="Disconnect Matrix account".to_string()
                message="Botrix will stop delivering notifications until you connect an account again. Webhook tokens and history are kept.".to_string()
                confirm_label="Disconnect".to_string()
                on_confirm=Callback::new(on_disconnect_confirmed)
                on_cancel=Callback::new(move |()| set_confirm_disconnect.set(false))
            />
        </div>
    }
}

/// Poll the device-grant endpoint until it resolves or the page is left.
async fn poll_until_done(
    alive: Arc<AtomicBool>,
    set_grant: WriteSignal<Option<ConnectResponse>>,
    set_error: WriteSignal<Option<String>>,
    set_notice: WriteSignal<Option<String>>,
    set_reload: WriteSignal<u32>,
) {
    while alive.load(Ordering::Relaxed) {
        gloo_timers::future::TimeoutFuture::new(POLL_INTERVAL_MS).await;
        if !alive.load(Ordering::Relaxed) {
            return;
        }
        match api::get::<ConnectPollResponse>("/api/v1/matrix/connect/poll").await {
            Ok(resp) => match resp.status {
                ConnectPollStatus::Pending => continue,
                ConnectPollStatus::Connected => {
                    set_grant.set(None);
                    set_notice.set(Some("Connected.".to_owned()));
                    set_reload.update(|n| *n += 1);
                    return;
                }
                ConnectPollStatus::Expired => {
                    set_grant.set(None);
                    set_error.set(Some(
                        "The authorization code expired. Try again.".to_owned(),
                    ));
                    return;
                }
                ConnectPollStatus::Error => {
                    set_grant.set(None);
                    set_error.set(Some(
                        resp.message
                            .unwrap_or_else(|| "Authorization failed.".to_owned()),
                    ));
                    return;
                }
            },
            Err(e) => {
                set_grant.set(None);
                if !redirect_if_unauthorized(&e) {
                    set_error.set(Some(e.user_message()));
                }
                return;
            }
        }
    }
}
