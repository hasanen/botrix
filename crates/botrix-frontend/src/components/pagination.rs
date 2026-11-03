//! Prev/next page controls component.

use leptos::prelude::*;

/// Simple previous / next pagination controls.
#[component]
pub fn Pagination(
    /// Current page number (1-indexed).
    page: u32,
    /// Total number of items.
    total: i64,
    /// Items per page.
    per_page: u32,
    /// Callback fired with the new page number.
    on_page_change: Callback<u32>,
) -> impl IntoView {
    let total_pages = move || ((total as f64) / (per_page as f64)).ceil() as u32;
    let has_prev = move || page > 1;
    let has_next = move || page < total_pages();

    view! {
        <div class="flex items-center justify-between text-sm text-gray-600 mt-4">
            <span>
                "Page " {page} " of " {total_pages}
            </span>
            <div class="flex gap-2">
                <button
                    type="button"
                    disabled=move || !has_prev()
                    on:click=move |_| on_page_change.run(page.saturating_sub(1))
                    class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50 disabled:opacity-40 disabled:cursor-not-allowed"
                >
                    "Previous"
                </button>
                <button
                    type="button"
                    disabled=move || !has_next()
                    on:click=move |_| on_page_change.run(page + 1)
                    class="px-3 py-1 border border-gray-300 rounded hover:bg-gray-50 disabled:opacity-40 disabled:cursor-not-allowed"
                >
                    "Next"
                </button>
            </div>
        </div>
    }
}
