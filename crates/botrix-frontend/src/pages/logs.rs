//! Operational log viewer page.

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::logs::LogListResponse;

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::pagination::Pagination;
use crate::components::status_badge::StatusBadge;
use crate::util::{format_unix, log_level_badge, page_path};

/// Logs page component.
#[component]
pub fn LogsPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <LogsContent />
        </AuthLayout>
    }
}

#[component]
fn LogsContent() -> impl IntoView {
    let (page, set_page) = signal(1_u32);
    let (data, set_data) = signal(Option::<LogListResponse>::None);
    let (error, set_error) = signal(Option::<String>::None);

    // Reload whenever the page changes.
    Effect::new(move |_| {
        let page = page.get();
        spawn_local(async move {
            match api::get::<LogListResponse>(&page_path("/api/v1/logs", page)).await {
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
        <div class="bg-white rounded-lg shadow p-6">
            <h2 class="text-xl font-semibold mb-4">"Logs"</h2>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || match data.get() {
                None => view! { <LoadingSpinner /> }.into_any(),
                Some(resp) if resp.items.is_empty() => {
                    view! { <p class="text-sm text-gray-500">"No log events yet."</p> }.into_any()
                }
                Some(resp) => {
                    let (current, total, per_page) = (resp.page, resp.total, resp.per_page);
                    view! {
                        <table class="w-full text-sm text-left">
                            <thead>
                                <tr class="text-gray-500 border-b border-gray-200">
                                    <th class="py-2 pr-4">"Time"</th>
                                    <th class="py-2 pr-4">"Level"</th>
                                    <th class="py-2 pr-4">"Component"</th>
                                    <th class="py-2 pr-4">"Message"</th>
                                    <th class="py-2">"Details"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {resp.items.into_iter().map(|entry| {
                                    let (badge, label) = log_level_badge(&entry.level);
                                    view! {
                                        <tr class="border-b border-gray-200">
                                            <td class="py-2 pr-4 nowrap">{format_unix(entry.created_at)}</td>
                                            <td class="py-2 pr-4">
                                                <StatusBadge status=badge label=label.to_string() />
                                            </td>
                                            <td class="py-2 pr-4">{entry.component}</td>
                                            <td class="py-2 pr-4">{entry.message}</td>
                                            <td class="py-2 text-gray-600">{entry.details.unwrap_or_default()}</td>
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
        </div>
    }
}
