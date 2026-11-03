//! Reusable confirmation dialog component.

use leptos::prelude::*;

/// A modal dialog that asks the user to confirm a destructive action.
#[component]
pub fn ConfirmModal(
    /// Whether the modal is currently visible.
    #[prop(into)]
    open: Signal<bool>,
    /// Title displayed at the top of the modal.
    title: String,
    /// Descriptive text explaining what will happen.
    message: String,
    /// Label for the confirm button.
    #[prop(default = "Confirm".to_string())]
    confirm_label: String,
    /// Callback fired when the user clicks confirm.
    on_confirm: Callback<()>,
    /// Callback fired when the user clicks cancel.
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <Show when=move || open.get()>
            <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
                <div class="bg-white rounded-lg shadow-xl max-w-md w-full mx-4 p-6">
                    <h2 class="text-lg font-semibold text-gray-900 mb-2">{title.clone()}</h2>
                    <p class="text-sm text-gray-600 mb-6">{message.clone()}</p>
                    <div class="flex justify-end gap-3">
                        <button
                            type="button"
                            on:click=move |_| on_cancel.run(())
                            class="px-4 py-2 text-sm font-medium text-gray-700 border border-gray-300 rounded hover:bg-gray-50"
                        >
                            "Cancel"
                        </button>
                        <button
                            type="button"
                            on:click=move |_| on_confirm.run(())
                            class="px-4 py-2 text-sm font-medium text-white bg-red-600 rounded hover:bg-red-700"
                        >
                            {confirm_label.clone()}
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    }
}
