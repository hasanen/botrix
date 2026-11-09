//! Create / edit page for a command trigger.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};
use leptos_router::NavigateOptions;

use botrix_common::api::triggers::AccessProfileDto;

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::triggers_form::{HeaderForm, SecretForm, TriggerForm};

/// HTTP methods a trigger may use.
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Method preselected for a new trigger.
const DEFAULT_METHOD: &str = "POST";

/// Where to go after a successful save.
const LIST_PATH: &str = "/triggers";

const INPUT_CLASS: &str = "w-full border border-gray-300 rounded px-3 py-2 text-sm";
const LABEL_CLASS: &str = "block text-sm font-medium text-gray-700 mb-1";
const BUTTON_CLASS: &str = "px-3 py-1 border border-gray-300 rounded hover:bg-gray-50 text-sm";

/// Defaults for a new trigger: enabled, `POST`.
fn new_trigger_form() -> TriggerForm {
    TriggerForm {
        enabled: true,
        method: DEFAULT_METHOD.to_owned(),
        ..TriggerForm::default()
    }
}

/// Page for `/triggers/new`.
#[component]
pub fn TriggerNewPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <TriggerEditor id=None initial=new_trigger_form() />
        </AuthLayout>
    }
}

/// Page for `/triggers/:id`.
#[component]
pub fn TriggerEditPage() -> impl IntoView {
    let params = use_params_map();
    let (loaded, set_loaded) = signal(Option::<(i64, TriggerForm)>::None);
    let (error, set_error) = signal(Option::<String>::None);

    Effect::new(move |_| {
        let raw = params.get().get("id");
        let Some(id) = raw.as_deref().and_then(|s| s.parse::<i64>().ok()) else {
            set_error.set(Some("Invalid trigger id.".to_owned()));
            return;
        };
        set_loaded.set(None);
        spawn_local(async move {
            match api::get_trigger(id).await {
                Ok(dto) => {
                    set_error.set(None);
                    set_loaded.set(Some((id, TriggerForm::from_dto(&dto))));
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
        <AuthLayout>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || match loaded.get() {
                Some((id, form)) => view! { <TriggerEditor id=Some(id) initial=form /> }.into_any(),
                None if error.get().is_some() => ().into_any(),
                None => view! { <LoadingSpinner /> }.into_any(),
            }}
        </AuthLayout>
    }
}

/// A labelled single-line input.
#[component]
fn TextField(
    id: String,
    label: String,
    #[prop(into)] value: Signal<String>,
    on_input: Callback<String>,
    #[prop(optional)] sr_only_label: bool,
    #[prop(optional)] required: bool,
    #[prop(optional)] readonly: bool,
    #[prop(default = "text")] kind: &'static str,
    #[prop(optional)] autocomplete: Option<&'static str>,
    #[prop(optional)] placeholder: Option<&'static str>,
) -> impl IntoView {
    let label_class = if sr_only_label {
        "sr-only"
    } else {
        LABEL_CLASS
    };
    view! {
        <div class="w-full">
            <label for=id.clone() class=label_class>{label}</label>
            <input
                id=id
                type=kind
                class=INPUT_CLASS
                autocomplete=autocomplete
                placeholder=placeholder
                required=required
                readonly=readonly
                prop:value=move || value.get()
                on:input=move |ev| on_input.run(event_target_value(&ev))
            />
        </div>
    }
}

#[component]
fn TriggerEditor(id: Option<i64>, initial: TriggerForm) -> impl IntoView {
    let existing_secrets = initial.secrets.len();
    let form = RwSignal::new(initial);
    let (profiles, set_profiles) = signal(Vec::<AccessProfileDto>::new());
    let (error, set_error) = signal(Option::<String>::None);
    let (saving, set_saving) = signal(false);
    let navigate = use_navigate();

    Effect::new(move |_| {
        spawn_local(async move {
            match api::list_access_profiles().await {
                Ok(list) => set_profiles.set(list),
                Err(e) => {
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    });

    let header_count = Memo::new(move |_| form.with(|f| f.headers.len()));
    let secret_count = Memo::new(move |_| form.with(|f| f.secrets.len()));

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        set_error.set(None);
        set_saving.set(true);
        let input = form.with_untracked(TriggerForm::to_input);
        let navigate = navigate.clone();
        spawn_local(async move {
            let result = match id {
                Some(id) => api::update_trigger(id, &input).await,
                None => api::create_trigger(&input).await,
            };
            match result {
                Ok(_) => navigate(LIST_PATH, NavigateOptions::default()),
                Err(e) => {
                    set_saving.set(false);
                    if !redirect_if_unauthorized(&e) {
                        set_error.set(Some(e.user_message()));
                    }
                }
            }
        });
    };

    let heading = if id.is_some() {
        "Edit trigger"
    } else {
        "New trigger"
    };

    view! {
        <section class="bg-white rounded-lg shadow p-6" aria-labelledby="trigger-heading">
            <h2 id="trigger-heading" class="text-xl font-semibold mb-4">{heading}</h2>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            <form on:submit=on_submit class="space-y-6 max-w-lg">
                <div class="space-y-4">
                    <TextField
                        id="trigger-command".to_owned()
                        label="Command".to_owned()
                        value=Signal::derive(move || form.with(|f| f.command.clone()))
                        on_input=Callback::new(move |v| form.update(|f| f.command = v))
                        required=true
                    />
                    <TextField
                        id="trigger-description".to_owned()
                        label="Description".to_owned()
                        value=Signal::derive(move || form.with(|f| f.description.clone()))
                        on_input=Callback::new(move |v| form.update(|f| f.description = v))
                    />
                    <div class="flex items-center gap-2">
                        <input
                            id="trigger-enabled"
                            type="checkbox"
                            prop:checked=move || form.with(|f| f.enabled)
                            on:change=move |ev| {
                                form.update(|f| f.enabled = event_target_checked(&ev))
                            }
                        />
                        <label for="trigger-enabled" class="text-sm font-medium text-gray-700">
                            "Enabled"
                        </label>
                    </div>
                </div>

                <fieldset class="space-y-4">
                    <legend class="text-lg font-semibold mb-2">"Webhook"</legend>
                    <div>
                        <label for="trigger-method" class=LABEL_CLASS>"Method"</label>
                        <select
                            id="trigger-method"
                            class=INPUT_CLASS
                            on:change=move |ev| form.update(|f| f.method = event_target_value(&ev))
                        >
                            {METHODS.iter().map(|m| {
                                let m = *m;
                                view! {
                                    <option
                                        value=m
                                        selected=move || form.with(|f| f.method == m)
                                    >
                                        {m}
                                    </option>
                                }
                            }).collect_view()}
                        </select>
                    </div>
                    <TextField
                        id="trigger-url".to_owned()
                        label="URL".to_owned()
                        kind="url"
                        value=Signal::derive(move || form.with(|f| f.url.clone()))
                        on_input=Callback::new(move |v| form.update(|f| f.url = v))
                        required=true
                    />
                    <div>
                        <label for="trigger-template" class=LABEL_CLASS>"Response template"</label>
                        <textarea
                            id="trigger-template"
                            rows="4"
                            class=INPUT_CLASS
                            aria-describedby="trigger-template-help"
                            prop:value=move || form.with(|f| f.response_template.clone())
                            on:input=move |ev| {
                                form.update(|f| f.response_template = event_target_value(&ev))
                            }
                        ></textarea>
                        <p id="trigger-template-help" class="text-xs text-gray-500 mt-2">
                            "Optional. Leave empty to use the default reply."
                        </p>
                    </div>
                </fieldset>

                <fieldset class="space-y-3">
                    <legend class="text-lg font-semibold mb-2">"Headers"</legend>
                    {move || (0..header_count.get()).map(|i| {
                        view! {
                            <div class="flex items-end gap-2">
                                <TextField
                                    id=format!("header-name-{i}")
                                    label=format!("Header {} name", i + 1)
                                    value=Signal::derive(move || form.with(|f| {
                                        f.headers.get(i).map(|h| h.name.clone()).unwrap_or_default()
                                    }))
                                    on_input=Callback::new(move |v| form.update(|f| {
                                        if let Some(h) = f.headers.get_mut(i) { h.name = v; }
                                    }))
                                />
                                <TextField
                                    id=format!("header-value-{i}")
                                    label=format!("Header {} value", i + 1)
                                    value=Signal::derive(move || form.with(|f| {
                                        f.headers.get(i).map(|h| h.value.clone()).unwrap_or_default()
                                    }))
                                    on_input=Callback::new(move |v| form.update(|f| {
                                        if let Some(h) = f.headers.get_mut(i) { h.value = v; }
                                    }))
                                />
                                <button
                                    type="button"
                                    class=BUTTON_CLASS
                                    aria-label=format!("Remove header {}", i + 1)
                                    on:click=move |_| form.update(|f| {
                                        if i < f.headers.len() { f.headers.remove(i); }
                                    })
                                >
                                    "Remove"
                                </button>
                            </div>
                        }
                    }).collect_view()}
                    <button
                        type="button"
                        class=BUTTON_CLASS
                        on:click=move |_| form.update(|f| f.headers.push(HeaderForm::default()))
                    >
                        "Add header"
                    </button>
                    <p class="text-xs text-gray-500">
                        "Header values may reference a secret as {{secret:NAME}}."
                    </p>
                </fieldset>

                <fieldset class="space-y-3">
                    <legend class="text-lg font-semibold mb-2">"Secrets"</legend>
                    <p class="text-xs text-gray-500">
                        "Values are write-only and never shown. For a secret that is set, an empty value keeps it unchanged."
                    </p>
                    {move || (0..secret_count.get()).map(|i| {
                        let is_existing = i < existing_secrets;
                        let is_set = move || form.with(|f| f.secrets.get(i).is_some_and(|s| s.is_set));
                        view! {
                            <div class="flex items-end gap-2">
                                <TextField
                                    id=format!("secret-name-{i}")
                                    label=format!("Secret {} name", i + 1)
                                    readonly=is_existing
                                    value=Signal::derive(move || form.with(|f| {
                                        f.secrets.get(i).map(|s| s.name.clone()).unwrap_or_default()
                                    }))
                                    on_input=Callback::new(move |v| form.update(|f| {
                                        if let Some(s) = f.secrets.get_mut(i) { s.name = v; }
                                    }))
                                />
                                <span class="text-sm text-gray-600 nowrap" data-testid="secret-status">
                                    {move || if is_existing && is_set() { "set" } else { "not set" }}
                                </span>
                                <TextField
                                    id=format!("secret-value-{i}")
                                    label=format!("Secret {} value", i + 1)
                                    kind="password"
                                    autocomplete="new-password"
                                    placeholder=if is_existing { "Leave empty to keep" } else { "" }
                                    value=Signal::derive(move || form.with(|f| {
                                        f.secrets.get(i).map(|s| s.new_value.clone()).unwrap_or_default()
                                    }))
                                    on_input=Callback::new(move |v| form.update(|f| {
                                        if let Some(s) = f.secrets.get_mut(i) { s.new_value = v; }
                                    }))
                                />
                                {if is_existing {
                                    view! {
                                        <div class="flex items-center gap-2">
                                            <input
                                                id=format!("secret-clear-{i}")
                                                type="checkbox"
                                                prop:checked=move || form.with(|f| {
                                                    f.secrets.get(i).is_some_and(|s| s.clear)
                                                })
                                                on:change=move |ev| form.update(|f| {
                                                    if let Some(s) = f.secrets.get_mut(i) {
                                                        s.clear = event_target_checked(&ev);
                                                    }
                                                })
                                            />
                                            <label for=format!("secret-clear-{i}") class="text-sm text-gray-700">
                                                "Clear"
                                            </label>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <button
                                            type="button"
                                            class=BUTTON_CLASS
                                            aria-label=format!("Remove secret {}", i + 1)
                                            on:click=move |_| form.update(|f| {
                                                if i < f.secrets.len() { f.secrets.remove(i); }
                                            })
                                        >
                                            "Remove"
                                        </button>
                                    }.into_any()
                                }}
                            </div>
                        }
                    }).collect_view()}
                    <button
                        type="button"
                        class=BUTTON_CLASS
                        on:click=move |_| form.update(|f| f.secrets.push(SecretForm::default()))
                    >
                        "Add secret"
                    </button>
                </fieldset>

                <fieldset class="space-y-4">
                    <legend class="text-lg font-semibold mb-2">"Access"</legend>
                    <div>
                        <label for="trigger-profile" class=LABEL_CLASS>"Access profile"</label>
                        <select
                            id="trigger-profile"
                            class=INPUT_CLASS
                            on:change=move |ev| {
                                let value = event_target_value(&ev);
                                form.update(|f| f.access_profile_id = value.parse::<i64>().ok());
                            }
                        >
                            <option value="" selected=move || form.with(|f| f.access_profile_id.is_none())>
                                "None"
                            </option>
                            {move || profiles.get().into_iter().map(|p| {
                                let pid = p.id;
                                view! {
                                    <option
                                        value=pid.to_string()
                                        selected=move || form.with(|f| f.access_profile_id == Some(pid))
                                    >
                                        {p.name}
                                    </option>
                                }
                            }).collect_view()}
                        </select>
                    </div>
                    <div>
                        <label for="trigger-rooms" class=LABEL_CLASS>"Allowed rooms (one per line)"</label>
                        <textarea
                            id="trigger-rooms"
                            rows="3"
                            class=INPUT_CLASS
                            aria-describedby="trigger-access-help"
                            prop:value=move || form.with(|f| f.allowed_rooms.clone())
                            on:input=move |ev| form.update(|f| f.allowed_rooms = event_target_value(&ev))
                        ></textarea>
                    </div>
                    <div>
                        <label for="trigger-users" class=LABEL_CLASS>"Allowed users (one per line)"</label>
                        <textarea
                            id="trigger-users"
                            rows="3"
                            class=INPUT_CLASS
                            aria-describedby="trigger-access-help"
                            prop:value=move || form.with(|f| f.allowed_users.clone())
                            on:input=move |ev| form.update(|f| f.allowed_users = event_target_value(&ev))
                        ></textarea>
                    </div>
                    <p id="trigger-access-help" class="text-xs text-gray-500">
                        "Empty lists mean unrestricted; inline lists override the profile."
                    </p>
                </fieldset>

                <div class="flex gap-2">
                    <button
                        type="submit"
                        class="bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium disabled:opacity-50"
                        disabled=move || saving.get()
                    >
                        "Save"
                    </button>
                    <A href=LIST_PATH attr:class=BUTTON_CLASS>"Cancel"</A>
                </div>
            </form>
        </section>
    }
}
