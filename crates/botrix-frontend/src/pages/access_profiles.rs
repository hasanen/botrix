//! Access profiles page: reusable room/user allowlists for command triggers.

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::triggers::{AccessProfileDto, AccessProfileInput};

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::modal::ConfirmModal;
use crate::triggers_form::parse_allowlist;

const INPUT_CLASS: &str = "w-full border border-gray-300 rounded px-3 py-2 text-sm";
const LABEL_CLASS: &str = "block text-sm font-medium text-gray-700 mb-1";
const BUTTON_CLASS: &str = "px-3 py-1 border border-gray-300 rounded hover:bg-gray-50 text-sm";

/// Editor state: `id` is `None` when creating a new profile.
#[derive(Debug, Clone, Default)]
struct ProfileForm {
    id: Option<i64>,
    name: String,
    allowed_rooms: String,
    allowed_users: String,
}

impl ProfileForm {
    fn from_dto(dto: &AccessProfileDto) -> Self {
        Self {
            id: Some(dto.id),
            name: dto.name.clone(),
            allowed_rooms: dto.allowed_rooms.join("\n"),
            allowed_users: dto.allowed_users.join("\n"),
        }
    }

    fn to_input(&self) -> AccessProfileInput {
        AccessProfileInput {
            name: self.name.trim().to_string(),
            allowed_rooms: parse_allowlist(&self.allowed_rooms),
            allowed_users: parse_allowlist(&self.allowed_users),
        }
    }
}

/// Access profiles page component.
#[component]
pub fn AccessProfilesPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <ProfilesCard />
        </AuthLayout>
    }
}

#[component]
fn ProfilesCard() -> impl IntoView {
    let (items, set_items) = signal(Vec::<AccessProfileDto>::new());
    let (loaded, set_loaded) = signal(false);
    let (error, set_error) = signal(Option::<String>::None);
    let (reload, set_reload) = signal(0_u32);
    let form = RwSignal::new(Option::<ProfileForm>::None);
    let (saving, set_saving) = signal(false);
    let (delete_target, set_delete_target) = signal(Option::<i64>::None);

    Effect::new(move |_| {
        reload.get();
        spawn_local(async move {
            match api::list_access_profiles().await {
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

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(current) = form.get() else {
            return;
        };
        let input = current.to_input();
        set_saving.set(true);
        spawn_local(async move {
            let result = match current.id {
                Some(id) => api::update_access_profile(id, &input).await,
                None => api::create_access_profile(&input).await,
            };
            set_saving.set(false);
            match result {
                Ok(_) => {
                    set_error.set(None);
                    form.set(None);
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

    let confirm_delete = move |()| {
        let Some(id) = delete_target.get() else {
            return;
        };
        spawn_local(async move {
            let result = api::delete_access_profile(id).await;
            set_delete_target.set(None);
            match result {
                Ok(()) => {
                    set_error.set(None);
                    // Close the editor if it was editing the deleted profile.
                    if form.with(|f| f.as_ref().and_then(|f| f.id) == Some(id)) {
                        form.set(None);
                    }
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

    view! {
        <section class="bg-white rounded-lg shadow p-6 space-y-6">
            <div class="flex items-center justify-between">
                <h2 class="text-xl font-semibold">"Access profiles"</h2>
                <button
                    type="button"
                    class=BUTTON_CLASS
                    on:click=move |_| form.set(Some(ProfileForm::default()))
                >
                    "New profile"
                </button>
            </div>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}

            {move || if !loaded.get() {
                view! { <LoadingSpinner /> }.into_any()
            } else if items.with(Vec::is_empty) {
                view! { <p class="text-sm text-gray-500">"No access profiles yet."</p> }.into_any()
            } else {
                view! {
                    <table class="w-full text-sm text-left">
                        <thead>
                            <tr class="text-gray-500 border-b border-gray-200">
                                <th class="py-2 pr-4">"Name"</th>
                                <th class="py-2 pr-4">"Rooms"</th>
                                <th class="py-2 pr-4">"Users"</th>
                                <th class="py-2">"Actions"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {items.get().into_iter().map(|p| {
                                let edit_dto = p.clone();
                                let id = p.id;
                                let edit_label = format!("Edit {}", p.name);
                                let delete_label = format!("Delete {}", p.name);
                                view! {
                                    <tr class="border-b border-gray-200">
                                        <td class="py-2 pr-4">{p.name}</td>
                                        <td class="py-2 pr-4">{p.allowed_rooms.len()}</td>
                                        <td class="py-2 pr-4">{p.allowed_users.len()}</td>
                                        <td class="py-2 flex gap-2">
                                            <button
                                                type="button"
                                                class=BUTTON_CLASS
                                                aria-label=edit_label
                                                on:click=move |_| form.set(Some(ProfileForm::from_dto(&edit_dto)))
                                            >
                                                "Edit"
                                            </button>
                                            <button
                                                type="button"
                                                class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50 text-sm text-red-700"
                                                aria-label=delete_label
                                                on:click=move |_| set_delete_target.set(Some(id))
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

            {move || form.get().map(|current| {
                let heading = if current.id.is_some() { "Edit profile" } else { "New profile" };
                view! {
                    <form on:submit=on_submit class="space-y-4" aria-labelledby="profile-form-heading">
                        <h3 id="profile-form-heading" class="text-lg font-semibold">{heading}</h3>
                        <div>
                            <label for="profile-name" class=LABEL_CLASS>"Name"</label>
                            <input
                                id="profile-name"
                                type="text"
                                required
                                class=INPUT_CLASS
                                prop:value=current.name.clone()
                                on:input=move |ev| form.update(|f| {
                                    if let Some(f) = f { f.name = event_target_value(&ev); }
                                })
                            />
                        </div>
                        <div>
                            <label for="profile-rooms" class=LABEL_CLASS>"Allowed rooms (one per line)"</label>
                            <textarea
                                id="profile-rooms"
                                rows="4"
                                class=INPUT_CLASS
                                aria-describedby="profile-help"
                                prop:value=current.allowed_rooms.clone()
                                on:input=move |ev| form.update(|f| {
                                    if let Some(f) = f { f.allowed_rooms = event_target_value(&ev); }
                                })
                            ></textarea>
                        </div>
                        <div>
                            <label for="profile-users" class=LABEL_CLASS>"Allowed users (one per line)"</label>
                            <textarea
                                id="profile-users"
                                rows="4"
                                class=INPUT_CLASS
                                aria-describedby="profile-help"
                                prop:value=current.allowed_users.clone()
                                on:input=move |ev| form.update(|f| {
                                    if let Some(f) = f { f.allowed_users = event_target_value(&ev); }
                                })
                            ></textarea>
                        </div>
                        <p id="profile-help" class="text-xs text-gray-500">
                            "Empty lists mean unrestricted."
                        </p>
                        <div class="flex gap-2">
                            <button
                                type="submit"
                                class="bg-blue-600 text-white rounded px-3 py-2 text-sm font-medium disabled:opacity-50"
                                disabled=move || saving.get()
                            >
                                "Save"
                            </button>
                            <button type="button" class=BUTTON_CLASS on:click=move |_| form.set(None)>
                                "Cancel"
                            </button>
                        </div>
                    </form>
                }
            })}

            <ConfirmModal
                open=Signal::derive(move || delete_target.get().is_some())
                title="Delete access profile".to_string()
                message="Triggers that reference this profile will fall back to their inline room and user lists. This cannot be undone.".to_string()
                confirm_label="Delete".to_string()
                on_confirm=Callback::new(confirm_delete)
                on_cancel=Callback::new(move |()| set_delete_target.set(None))
            />
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_round_trips_lists_one_per_line() {
        let dto = AccessProfileDto {
            id: 4,
            name: "ops".into(),
            allowed_rooms: vec!["!a:x".into(), "!b:x".into()],
            allowed_users: vec!["@u:x".into()],
        };
        let form = ProfileForm::from_dto(&dto);
        assert_eq!(form.id, Some(4));
        assert_eq!(form.allowed_rooms, "!a:x\n!b:x");

        let input = form.to_input();
        assert_eq!(input.name, "ops");
        assert_eq!(input.allowed_rooms, dto.allowed_rooms);
        assert_eq!(input.allowed_users, dto.allowed_users);
    }

    #[test]
    fn to_input_trims_name_and_parses_lists() {
        let form = ProfileForm {
            id: None,
            name: "  team ".into(),
            allowed_rooms: " !a:x ,\n\n!a:x".into(),
            allowed_users: String::new(),
        };
        let input = form.to_input();
        assert_eq!(input.name, "team");
        assert_eq!(input.allowed_rooms, ["!a:x"]);
        assert!(input.allowed_users.is_empty());
    }
}
