//! Command triggers page.
//!
//! Triggers are defined in the config file, so this page is read-only apart
//! from secret values, which can be set or cleared per declared secret. It is
//! laid out like the webhook endpoints card.

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::triggers::TriggerInfo;

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::modal::ConfirmModal;
use crate::components::status_badge::StatusBadge;
use crate::util::{access_lines, secret_badge, secret_value_error};

/// A `(command, secret name)` pair a modal is acting on.
type SecretTarget = (String, String);

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
    let (items, set_items) = signal(Vec::<TriggerInfo>::new());
    let (loaded, set_loaded) = signal(false);
    let (error, set_error) = signal(Option::<String>::None);
    let (set_target, set_set_target) = signal(Option::<SecretTarget>::None);
    let (clear_target, set_clear_target) = signal(Option::<SecretTarget>::None);
    let (secret_value, set_secret_value) = signal(String::new());
    let (modal_error, set_modal_error) = signal(Option::<String>::None);
    let (saving, set_saving) = signal(false);
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

    // Closes the set-value dialog and forgets what was typed.
    let close_set = move || {
        set_set_target.set(None);
        set_secret_value.set(String::new());
        set_modal_error.set(None);
    };

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some((command, name)) = set_target.get() else {
            return;
        };
        let value = secret_value.get();
        if let Some(message) = secret_value_error(&value) {
            set_modal_error.set(Some(message.to_owned()));
            return;
        }
        set_modal_error.set(None);
        set_saving.set(true);
        spawn_local(async move {
            let result = api::set_trigger_secret(&command, &name, &value).await;
            set_saving.set(false);
            match result {
                Ok(_) => {
                    close_set();
                    set_reload.update(|n| *n += 1);
                }
                Err(e) => {
                    if !redirect_if_unauthorized(&e) {
                        set_modal_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    };

    let confirm_clear = move |()| {
        let Some((command, name)) = clear_target.get() else {
            return;
        };
        spawn_local(async move {
            let result = api::clear_trigger_secret(&command, &name).await;
            set_clear_target.set(None);
            match result {
                Ok(_) => set_reload.update(|n| *n += 1),
                Err(e) => fail(e),
            }
        });
    };

    view! {
        <section class="bg-white rounded-lg shadow p-6">
            <h2 class="text-xl font-semibold mb-1">"Command triggers"</h2>
            <p class="text-sm text-gray-600 mb-4">
                "Triggers are defined in the config file. Only secret values can be changed here."
            </p>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || if !loaded.get() {
                view! { <LoadingSpinner /> }.into_any()
            } else if items.get().is_empty() {
                view! {
                    <p class="text-sm text-gray-500">
                        "No command triggers are defined. Add them under "
                        <code>"triggers:"</code> " in the config file."
                    </p>
                }.into_any()
            } else {
                view! {
                    <div class="overflow-x-auto">
                        <table class="w-full text-sm text-left">
                            <caption class="sr-only">"Command triggers"</caption>
                            <thead>
                                <tr class="text-gray-500 border-b border-gray-200">
                                    <th scope="col" class="py-2 pr-4">"Command"</th>
                                    <th scope="col" class="py-2 pr-4">"Method"</th>
                                    <th scope="col" class="py-2 pr-4">"URL"</th>
                                    <th scope="col" class="py-2 pr-4">"Enabled"</th>
                                    <th scope="col" class="py-2 pr-4">"Access"</th>
                                    <th scope="col" class="py-2">"Secrets"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {items.get().into_iter().map(|t| {
                                    let access = access_lines(
                                        &t.allowed_rooms,
                                        &t.allowed_users,
                                        t.access_profile.as_deref(),
                                    );
                                    let headers = t.header_names.join(", ");
                                    let command = t.command.clone();
                                    view! {
                                        <tr class="border-b border-gray-200 align-top">
                                            <td class="py-2 pr-4">
                                                <code>{t.command.clone()}</code>
                                                {(!t.description.is_empty()).then(|| view! {
                                                    <div class="text-xs text-gray-500">{t.description.clone()}</div>
                                                })}
                                            </td>
                                            <td class="py-2 pr-4">{t.method.clone()}</td>
                                            <td class="py-2 pr-4">
                                                <code class="text-xs break-all">{t.url.clone()}</code>
                                                {(!headers.is_empty()).then(|| view! {
                                                    <div class="text-xs text-gray-500">"Headers: " {headers.clone()}</div>
                                                })}
                                            </td>
                                            <td class="py-2 pr-4">{if t.enabled { "Yes" } else { "No" }}</td>
                                            <td class="py-2 pr-4 text-xs">
                                                {access.into_iter().map(|line| view! {
                                                    <div class="break-all">{line}</div>
                                                }).collect_view()}
                                            </td>
                                            <td class="py-2">
                                                {if t.secrets.is_empty() {
                                                    view! { <span class="text-gray-500">"None"</span> }.into_any()
                                                } else {
                                                    t.secrets.into_iter().map(|s| {
                                                        let (badge, label) = secret_badge(s.is_set);
                                                        let set_label = format!("Set value of secret {} for {}", s.name, command);
                                                        let clear_label = format!("Clear secret {} of {}", s.name, command);
                                                        let for_set = (command.clone(), s.name.clone());
                                                        let for_clear = (command.clone(), s.name.clone());
                                                        view! {
                                                            <div class="flex flex-wrap items-center gap-2 mb-1">
                                                                <code class="text-xs">{s.name.clone()}</code>
                                                                <StatusBadge status=badge label=label.to_string() />
                                                                <button
                                                                    type="button"
                                                                    aria-label=set_label
                                                                    on:click=move |_| set_set_target.set(Some(for_set.clone()))
                                                                    class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50"
                                                                >
                                                                    "Set value"
                                                                </button>
                                                                {s.is_set.then(|| view! {
                                                                    <button
                                                                        type="button"
                                                                        aria-label=clear_label
                                                                        on:click=move |_| set_clear_target.set(Some(for_clear.clone()))
                                                                        class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50 text-red-700"
                                                                    >
                                                                        "Clear"
                                                                    </button>
                                                                })}
                                                            </div>
                                                        }
                                                    }).collect_view().into_any()
                                                }}
                                            </td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                }.into_any()
            }}

            <Show when=move || set_target.get().is_some()>
                <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
                    <form
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="secret-modal-title"
                        on:submit=submit
                        class="bg-white rounded-lg shadow-xl max-w-md w-full mx-4 p-6"
                    >
                        <h2 id="secret-modal-title" class="text-lg font-semibold text-gray-900 mb-2">
                            {move || set_target.get().map(|(command, name)| {
                                format!("Set secret {name} for {command}")
                            })}
                        </h2>
                        <p class="text-sm text-gray-600 mb-4">
                            "The value is stored encrypted and is never shown again."
                        </p>
                        <label for="secret-value" class="block text-sm font-medium text-gray-700 mb-1">
                            "Secret value"
                        </label>
                        <input
                            id="secret-value"
                            type="password"
                            autocomplete="new-password"
                            class="w-full border border-gray-300 rounded px-3 py-2 text-sm mb-2"
                            prop:value=move || secret_value.get()
                            on:input=move |ev| set_secret_value.set(event_target_value(&ev))
                        />
                        {move || modal_error.get().map(|msg| view! {
                            <p role="alert" class="text-sm text-red-700 mb-2">{msg}</p>
                        })}
                        <div class="flex justify-end gap-3 mt-4">
                            <button
                                type="button"
                                on:click=move |_| close_set()
                                class="px-4 py-2 text-sm font-medium text-gray-700 border border-gray-300 rounded hover:bg-gray-50"
                            >
                                "Cancel"
                            </button>
                            <button
                                type="submit"
                                prop:disabled=move || saving.get()
                                class="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded hover:bg-blue-700 disabled:opacity-50"
                            >
                                "Save"
                            </button>
                        </div>
                    </form>
                </div>
            </Show>

            <ConfirmModal
                open=Signal::derive(move || clear_target.get().is_some())
                title="Clear secret".to_string()
                message="The stored value is removed and commands that use it will fail until a new value is set.".to_string()
                confirm_label="Clear".to_string()
                on_confirm=Callback::new(confirm_clear)
                on_cancel=Callback::new(move |()| set_clear_target.set(None))
            />
        </section>
    }
}
