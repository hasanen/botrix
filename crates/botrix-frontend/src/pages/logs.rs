//! Operational log viewer page.
//!
//! Events can be filtered by component and by trigger command. The initial
//! filters come from the URL (`/logs?component=trigger&command=echo`), which
//! is what the Triggers page links to.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_query_map;

use botrix_common::api::logs::LogListResponse;

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::pagination::Pagination;
use crate::components::status_badge::StatusBadge;
use crate::util::{format_unix, log_filter_summary, log_level_badge, logs_path, LOG_COMPONENTS};

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
    let query = use_query_map().get_untracked();
    let initial_component = query.get("component").unwrap_or_default();
    let initial_command = query.get("command").unwrap_or_default();

    let (page, set_page) = signal(1_u32);
    // `applied` drives the request; the `draft_*` signals are what the inputs
    // currently show, applied when the form is submitted.
    let (applied, set_applied) = signal((initial_component.clone(), initial_command.clone()));
    let (draft_component, set_draft_component) = signal(initial_component);
    let (draft_command, set_draft_command) = signal(initial_command);
    let (data, set_data) = signal(Option::<LogListResponse>::None);
    let (error, set_error) = signal(Option::<String>::None);

    // Reload whenever the page or the applied filters change.
    Effect::new(move |_| {
        let page = page.get();
        let (component, command) = applied.get();
        spawn_local(async move {
            match api::get::<LogListResponse>(&logs_path(page, &component, &command)).await {
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

    let apply = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        set_page.set(1);
        set_applied.set((draft_component.get(), draft_command.get().trim().to_owned()));
    };

    let clear = move |_| {
        set_draft_component.set(String::new());
        set_draft_command.set(String::new());
        set_page.set(1);
        set_applied.set((String::new(), String::new()));
    };

    view! {
        <div class="bg-white rounded-lg shadow p-6">
            <h2 class="text-xl font-semibold mb-4">
                "Logs"
                {move || {
                    let (component, command) = applied.get();
                    log_filter_summary(&component, &command).map(|summary| view! {
                        <span class="text-sm font-normal text-gray-500">" — " {summary}</span>
                    })
                }}
            </h2>

            <form on:submit=apply aria-label="Filter logs" class="flex flex-wrap items-end gap-3 mb-4">
                <div>
                    <label for="log-component" class="block text-sm font-medium text-gray-700 mb-1">
                        "Component"
                    </label>
                    <select
                        id="log-component"
                        on:change=move |ev| set_draft_component.set(event_target_value(&ev))
                        class="border border-gray-300 rounded px-3 py-2 text-sm"
                    >
                        <option value="" prop:selected=move || draft_component.get().is_empty()>
                            "All"
                        </option>
                        {LOG_COMPONENTS.iter().map(|name| view! {
                            <option value=*name prop:selected=move || draft_component.get() == *name>
                                {*name}
                            </option>
                        }).collect_view()}
                    </select>
                </div>
                <div>
                    <label for="log-command" class="block text-sm font-medium text-gray-700 mb-1">
                        "Command"
                    </label>
                    <input
                        id="log-command"
                        type="text"
                        placeholder="e.g. deploy"
                        autocomplete="off"
                        prop:value=move || draft_command.get()
                        on:input=move |ev| set_draft_command.set(event_target_value(&ev))
                        class="border border-gray-300 rounded px-3 py-2 text-sm"
                    />
                </div>
                <button
                    type="submit"
                    class="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded hover:bg-blue-700"
                >
                    "Apply"
                </button>
                <button
                    type="button"
                    on:click=clear
                    class="px-4 py-2 text-sm font-medium text-gray-700 border border-gray-300 rounded hover:bg-gray-50"
                >
                    "Clear"
                </button>
            </form>

            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || match data.get() {
                None => view! { <LoadingSpinner /> }.into_any(),
                Some(resp) if resp.items.is_empty() => {
                    let (component, command) = applied.get();
                    let text = if log_filter_summary(&component, &command).is_some() {
                        "No log events match these filters."
                    } else {
                        "No log events yet."
                    };
                    view! { <p class="text-sm text-gray-500">{text}</p> }.into_any()
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
