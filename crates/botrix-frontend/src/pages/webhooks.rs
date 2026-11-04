//! Webhook endpoints and notification history page.

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::notifications::NotificationListResponse;
use botrix_common::api::webhooks::{TokenGenerateResponse, WebhookInfo};
use botrix_common::types::NotificationStatus;

use crate::api;
use crate::components::copy_button::CopyButton;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::modal::ConfirmModal;
use crate::components::pagination::Pagination;
use crate::components::status_badge::StatusBadge;
use crate::util::{format_unix, notification_badge, page_path, preview, webhook_url};

/// Characters shown of a rendered message in the history table.
const MESSAGE_PREVIEW_CHARS: usize = 80;

/// Webhooks page component.
#[component]
pub fn WebhooksPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <div class="space-y-6">
                <EndpointsCard />
                <HistoryCard />
            </div>
        </AuthLayout>
    }
}

#[component]
fn EndpointsCard() -> impl IntoView {
    let (items, set_items) = signal(Vec::<WebhookInfo>::new());
    let (loaded, set_loaded) = signal(false);
    let (error, set_error) = signal(Option::<String>::None);
    let (new_token, set_new_token) = signal(Option::<(String, String)>::None);
    let (revoke_target, set_revoke_target) = signal(Option::<String>::None);
    let (reload, set_reload) = signal(0_u32);

    Effect::new(move |_| {
        reload.get();
        spawn_local(async move {
            match api::get::<Vec<WebhookInfo>>("/api/v1/webhooks").await {
                Ok(list) => {
                    set_error.set(None);
                    set_items.set(list);
                    set_loaded.set(true);
                }
                Err(e) => {
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    });

    let generate = move |id: String| {
        spawn_local(async move {
            let path = format!("/api/v1/webhooks/{id}/token");
            match api::post_csrf::<_, TokenGenerateResponse>(&path, &()).await {
                Ok(resp) => {
                    set_new_token.set(Some((id, resp.token)));
                    set_reload.update(|n| *n += 1);
                }
                Err(e) => {
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    };

    let confirm_revoke = move |()| {
        let Some(id) = revoke_target.get() else {
            return;
        };
        spawn_local(async move {
            let path = format!("/api/v1/webhooks/{id}/token");
            match api::delete_csrf::<serde_json::Value>(&path).await {
                Ok(_) => {
                    set_revoke_target.set(None);
                    set_reload.update(|n| *n += 1);
                }
                Err(e) => {
                    set_revoke_target.set(None);
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    };

    let origin = window()
        .location()
        .origin()
        .unwrap_or_else(|_| String::new());

    view! {
        <section class="bg-white rounded-lg shadow p-6">
            <h2 class="text-xl font-semibold mb-4">"Webhook endpoints"</h2>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || if !loaded.get() {
                view! { <LoadingSpinner /> }.into_any()
            } else {
                let origin = origin.clone();
                view! {
                    <table class="w-full text-sm text-left">
                        <thead>
                            <tr class="text-gray-500 border-b border-gray-200">
                                <th class="py-2 pr-4">"Webhook"</th>
                                <th class="py-2 pr-4">"Endpoint URL"</th>
                                <th class="py-2 pr-4">"Destination"</th>
                                <th class="py-2 pr-4">"Enabled"</th>
                                <th class="py-2 pr-4">"Token"</th>
                                <th class="py-2">"Actions"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {items.get().into_iter().map(|w| {
                                let url = webhook_url(&origin, &w.id);
                                let id_for_generate = w.id.clone();
                                let id_for_revoke = w.id.clone();
                                view! {
                                    <tr class="border-b border-gray-200">
                                        <td class="py-2 pr-4">{w.id.clone()}</td>
                                        <td class="py-2 pr-4">
                                            <code class="text-xs">{url.clone()}</code>
                                            <CopyButton text=url />
                                        </td>
                                        <td class="py-2 pr-4">{w.destination}</td>
                                        <td class="py-2 pr-4">{if w.enabled { "Yes" } else { "No" }}</td>
                                        <td class="py-2 pr-4">{if w.has_token { "Set" } else { "None" }}</td>
                                        <td class="py-2 flex gap-2">
                                            <button
                                                type="button"
                                                on:click=move |_| generate(id_for_generate.clone())
                                                class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50"
                                            >
                                                "Generate token"
                                            </button>
                                            {w.has_token.then(|| view! {
                                                <button
                                                    type="button"
                                                    on:click=move |_| set_revoke_target.set(Some(id_for_revoke.clone()))
                                                    class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50 text-red-700"
                                                >
                                                    "Revoke"
                                                </button>
                                            })}
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                }.into_any()
            }}

            // Plaintext token shown exactly once.
            {move || new_token.get().map(|(id, token)| view! {
                <div class="mt-4 rounded-md bg-blue-50 border border-blue-200 p-4 text-sm" role="alert">
                    <p class="mb-2">
                        "New token for " <b>{id}</b> " (copy it now — it will not be shown again):"
                    </p>
                    <div class="flex items-center gap-2">
                        <code class="text-xs break-all">{token.clone()}</code>
                        <CopyButton text=token />
                    </div>
                    <button
                        type="button"
                        on:click=move |_| set_new_token.set(None)
                        class="mt-2 px-3 py-1 border border-gray-300 rounded hover:bg-gray-50"
                    >
                        "Dismiss"
                    </button>
                </div>
            })}

            <ConfirmModal
                open=Signal::derive(move || revoke_target.get().is_some())
                title="Revoke webhook token".to_string()
                message="The current token will stop working immediately. This cannot be undone.".to_string()
                confirm_label="Revoke".to_string()
                on_confirm=Callback::new(confirm_revoke)
                on_cancel=Callback::new(move |()| set_revoke_target.set(None))
            />
        </section>
    }
}

#[component]
fn HistoryCard() -> impl IntoView {
    let (page, set_page) = signal(1_u32);
    let (data, set_data) = signal(Option::<NotificationListResponse>::None);
    let (error, set_error) = signal(Option::<String>::None);

    Effect::new(move |_| {
        let page = page.get();
        spawn_local(async move {
            match api::get::<NotificationListResponse>(&page_path("/api/v1/notifications", page))
                .await
            {
                Ok(resp) => {
                    set_error.set(None);
                    set_data.set(Some(resp));
                }
                Err(e) => {
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    });

    view! {
        <section class="bg-white rounded-lg shadow p-6">
            <h2 class="text-xl font-semibold mb-4">"Notification history"</h2>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || match data.get() {
                None => view! { <LoadingSpinner /> }.into_any(),
                Some(resp) if resp.items.is_empty() => {
                    view! { <p class="text-sm text-gray-500">"No notifications yet."</p> }.into_any()
                }
                Some(resp) => {
                    let (current, total, per_page) = (resp.page, resp.total, resp.per_page);
                    view! {
                        <table class="w-full text-sm text-left">
                            <thead>
                                <tr class="text-gray-500 border-b border-gray-200">
                                    <th class="py-2 pr-4">"Time"</th>
                                    <th class="py-2 pr-4">"Webhook"</th>
                                    <th class="py-2 pr-4">"Destination"</th>
                                    <th class="py-2 pr-4">"Message"</th>
                                    <th class="py-2">"Status"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {resp.items.into_iter().map(|n| {
                                    let (badge, label) = notification_badge(&n.status);
                                    let row_class = if n.status == NotificationStatus::Failed {
                                        "border-b border-gray-200 bg-red-50"
                                    } else {
                                        "border-b border-gray-200"
                                    };
                                    view! {
                                        <tr class=row_class>
                                            <td class="py-2 pr-4 nowrap">{format_unix(n.created_at)}</td>
                                            <td class="py-2 pr-4">{n.webhook_id}</td>
                                            <td class="py-2 pr-4">{n.destination}</td>
                                            <td class="py-2 pr-4">{preview(&n.rendered_message, MESSAGE_PREVIEW_CHARS)}</td>
                                            <td class="py-2">
                                                <StatusBadge status=badge label=label.to_string() />
                                            </td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                        <Pagination
                            page=current
                            total=total
                            per_page=per_page
                            on_page_change=Callback::new(move |p: u32| set_page.set(p.max(1)))
                        />
                    }.into_any()
                }
            }}
        </section>
    }
}
