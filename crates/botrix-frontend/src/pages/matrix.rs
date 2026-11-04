//! Matrix account and rooms management page.

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::matrix::{
    ConnectResponse, DisconnectRequest, MatrixStatus, RecoveryKeyRequest, RoomInfo,
    TestMessageRequest,
};
use botrix_common::types::{MatrixConnectionStatus, RoomMembershipState};

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::modal::ConfirmModal;
use crate::components::status_badge::StatusBadge;
use crate::util::{connection_badge, format_unix, membership_badge};

/// Matrix page component.
#[component]
pub fn MatrixPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <MatrixContent />
        </AuthLayout>
    }
}

/// Read a `?key=` value from the current URL's query string.
fn query_param(key: &str) -> Option<String> {
    let search = window().location().search().ok()?;
    let query = search.strip_prefix('?').unwrap_or(&search);
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_owned())
    })
}

#[component]
fn MatrixContent() -> impl IntoView {
    let (status, set_status) = signal(Option::<MatrixStatus>::None);
    let (rooms, set_rooms) = signal(Vec::<RoomInfo>::new());
    let (error, set_error) = signal(Option::<String>::None);
    let (notice, set_notice) = signal(Option::<String>::None);
    let (connecting, set_connecting) = signal(false);
    let (reload, set_reload) = signal(0_u32);

    let (confirm_disconnect, set_confirm_disconnect) = signal(false);
    let (recovery_key, set_recovery_key) = signal(String::new());
    let (test_destination, set_test_destination) = signal(String::new());

    // Surface the outcome the callback redirect reported via the query string.
    if query_param("connected").is_some() {
        set_notice.set(Some("Matrix account connected.".to_owned()));
    } else if let Some(err) = query_param("error") {
        set_error.set(Some(match err.as_str() {
            "denied" => "Authorization was denied on the homeserver.".to_owned(),
            _ => "Matrix login failed. Check the logs and try again.".to_owned(),
        }));
    }

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

    // Start login: POST /connect, then send the browser to the homeserver's
    // authorization URL. It redirects back to /matrix?connected=1 when done.
    let on_connect = move |_| {
        set_error.set(None);
        set_notice.set(None);
        set_connecting.set(true);
        spawn_local(async move {
            match api::post_csrf::<_, ConnectResponse>("/api/v1/matrix/connect", &()).await {
                Ok(resp) => {
                    let _ = window().location().set_href(&resp.authorization_url);
                }
                Err(e) => {
                    set_connecting.set(false);
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
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
                                            on:click=on_connect
                                            disabled=move || connecting.get()
                                            class="bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium"
                                        >
                                            {move || if connecting.get() { "Redirecting…" } else { "Connect Matrix account" }}
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
                                {(!connected).then(|| view! {
                                    <p class="text-gray-500">
                                        "You will be sent to your homeserver to authorize the bot account, "
                                        "then returned here."
                                    </p>
                                })}
                            </div>
                        }.into_any()
                    }
                }}
            </section>

            // Recovery key prompt: only when connected without E2EE bootstrapped.
            {move || status.get().filter(|s| {
                s.status == MatrixConnectionStatus::Connected && !s.e2ee_bootstrapped
            }).map(|_| view! {
                <section class="bg-white rounded-lg shadow p-6" aria-labelledby="recovery-heading">
                    <h2 id="recovery-heading" class="text-xl font-semibold mb-2">"Encryption recovery key"</h2>
                    <p class="text-sm text-gray-600 mb-4">
                        "Enter your recovery key so Botrix can send messages to encrypted rooms. "
                        "It is used once to import the encryption keys into Botrix's local, "
                        "passphrase-encrypted store, then discarded — it is never saved, and "
                        "restarts do not ask for it again."
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
