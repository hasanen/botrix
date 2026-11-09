//! Command triggers list page.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

use botrix_common::api::triggers::{TriggerDto, TriggerTestResponse};

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::modal::ConfirmModal;

/// A finished dry-run, shown in the result dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TestOutcome {
    command: String,
    response: TriggerTestResponse,
}

/// Command triggers page component.
#[component]
pub fn TriggersPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <TriggersCard />
        </AuthLayout>
    }
}

#[component]
fn TriggersCard() -> impl IntoView {
    let (items, set_items) = signal(Vec::<TriggerDto>::new());
    let (loaded, set_loaded) = signal(false);
    let (error, set_error) = signal(Option::<String>::None);
    let (delete_target, set_delete_target) = signal(Option::<TriggerDto>::None);
    let (test_outcome, set_test_outcome) = signal(Option::<TestOutcome>::None);
    let (reload, set_reload) = signal(0_u32);

    let fail = move |e: api::ApiError| {
        if !redirect_if_unauthorized(&e) {
            set_error.set(Some(e.user_message()));
        }
    };

    Effect::new(move |_| {
        reload.get();
        spawn_local(async move {
            match api::list_triggers().await {
                Ok(list) => {
                    set_error.set(None);
                    set_items.set(list);
                    set_loaded.set(true);
                }
                Err(e) => fail(e),
            }
        });
    });

    let toggle = move |trigger: TriggerDto, enabled: bool| {
        spawn_local(async move {
            match api::set_trigger_enabled(&trigger, enabled).await {
                Ok(_) => set_reload.update(|n| *n += 1),
                Err(e) => {
                    // Re-render so the checkbox reflects the stored state.
                    set_reload.update(|n| *n += 1);
                    fail(e);
                }
            }
        });
    };

    let run_test = move |trigger: TriggerDto| {
        spawn_local(async move {
            match api::test_trigger(trigger.id, None).await {
                Ok(response) => set_test_outcome.set(Some(TestOutcome {
                    command: trigger.command,
                    response,
                })),
                Err(e) => fail(e),
            }
        });
    };

    let confirm_delete = move |()| {
        let Some(trigger) = delete_target.get() else {
            return;
        };
        spawn_local(async move {
            let result = api::delete_trigger(trigger.id).await;
            set_delete_target.set(None);
            match result {
                Ok(()) => set_reload.update(|n| *n += 1),
                Err(e) => fail(e),
            }
        });
    };

    view! {
        <section class="bg-white rounded-lg shadow p-6">
            <div class="flex items-center justify-between mb-4">
                <h2 class="text-xl font-semibold">"Command triggers"</h2>
                <A href="/triggers/new" attr:class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50">
                    "New trigger"
                </A>
            </div>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || if !loaded.get() {
                view! { <LoadingSpinner /> }.into_any()
            } else if items.get().is_empty() {
                view! { <p class="text-sm text-gray-500">"No command triggers yet."</p> }.into_any()
            } else {
                view! {
                    <table class="w-full text-sm text-left">
                        <caption class="sr-only">"Command triggers"</caption>
                        <thead>
                            <tr class="text-gray-500 border-b border-gray-200">
                                <th scope="col" class="py-2 pr-4">"Command"</th>
                                <th scope="col" class="py-2 pr-4">"Method"</th>
                                <th scope="col" class="py-2 pr-4">"URL"</th>
                                <th scope="col" class="py-2 pr-4">"Enabled"</th>
                                <th scope="col" class="py-2">"Actions"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {items.get().into_iter().map(|t| {
                                let edit_href = format!("/triggers/{}", t.id);
                                let edit_label = format!("Edit {}", t.command);
                                let test_label = format!("Test {}", t.command);
                                let delete_label = format!("Delete {}", t.command);
                                let toggle_label = format!("Enabled: {}", t.command);
                                let for_toggle = t.clone();
                                let for_test = t.clone();
                                let for_delete = t.clone();
                                view! {
                                    <tr class="border-b border-gray-200">
                                        <td class="py-2 pr-4"><code>{t.command.clone()}</code></td>
                                        <td class="py-2 pr-4">{t.method.clone()}</td>
                                        <td class="py-2 pr-4"><code class="text-xs">{t.url.clone()}</code></td>
                                        <td class="py-2 pr-4">
                                            <input
                                                type="checkbox"
                                                aria-label=toggle_label
                                                prop:checked=t.enabled
                                                on:change=move |ev| {
                                                    toggle(for_toggle.clone(), event_target_checked(&ev))
                                                }
                                            />
                                        </td>
                                        <td class="py-2 flex gap-2">
                                            <A
                                                href=edit_href
                                                attr:class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50"
                                                attr:aria-label=edit_label
                                            >
                                                "Edit"
                                            </A>
                                            <button
                                                type="button"
                                                aria-label=test_label
                                                on:click=move |_| run_test(for_test.clone())
                                                class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50"
                                            >
                                                "Test"
                                            </button>
                                            <button
                                                type="button"
                                                aria-label=delete_label
                                                on:click=move |_| set_delete_target.set(Some(for_delete.clone()))
                                                class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50 text-red-700"
                                            >
                                                "Delete"
                                            </button>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                }.into_any()
            }}

            {move || test_outcome.get().map(|outcome| {
                let status = outcome
                    .response
                    .status
                    .map_or_else(|| "no response".to_string(), |s| s.to_string());
                view! {
                    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
                        <div
                            role="dialog"
                            aria-modal="true"
                            aria-labelledby="trigger-test-title"
                            class="bg-white rounded-lg shadow-xl max-w-md w-full mx-4 p-6"
                        >
                            <h2 id="trigger-test-title" class="text-lg font-semibold text-gray-900 mb-2">
                                "Test result: " {outcome.command}
                            </h2>
                            <p class="text-sm text-gray-600 mb-2">
                                "Status: " <b>{status}</b>
                            </p>
                            <p class="text-sm text-gray-600 mb-2">"Rendered reply:"</p>
                            <pre class="text-xs mb-6" style="white-space: pre-wrap;">{outcome.response.reply}</pre>
                            <div class="flex justify-end gap-3">
                                <button
                                    type="button"
                                    on:click=move |_| set_test_outcome.set(None)
                                    class="px-4 py-2 text-sm font-medium text-gray-700 border border-gray-300 rounded hover:bg-gray-50"
                                >
                                    "Close"
                                </button>
                            </div>
                        </div>
                    </div>
                }
            })}

            <ConfirmModal
                open=Signal::derive(move || delete_target.get().is_some())
                title="Delete trigger".to_string()
                message="The trigger and its stored secrets will be removed. This cannot be undone.".to_string()
                confirm_label="Delete".to_string()
                on_confirm=Callback::new(confirm_delete)
                on_cancel=Callback::new(move |()| set_delete_target.set(None))
            />
        </section>
    }
}
