//! Dashboard page — Matrix connection summary.

use leptos::prelude::*;
use leptos::task::spawn_local;

use botrix_common::api::matrix::MatrixStatus;
use botrix_common::types::MatrixConnectionStatus;

use crate::api;
use crate::components::error_banner::ErrorBanner;
use crate::components::layout::{redirect_if_unauthorized, AuthLayout};
use crate::components::loading_spinner::LoadingSpinner;
use crate::components::status_badge::StatusBadge;
use crate::util::{connection_badge, format_unix};

/// Dashboard page component.
#[component]
pub fn DashboardPage() -> impl IntoView {
    view! {
        <AuthLayout>
            <DashboardContent />
        </AuthLayout>
    }
}

#[component]
fn DashboardContent() -> impl IntoView {
    let (status, set_status) = signal(Option::<MatrixStatus>::None);
    let (error, set_error) = signal(Option::<String>::None);

    Effect::new(move |_| {
        spawn_local(async move {
            match api::get::<MatrixStatus>("/api/v1/matrix/status").await {
                Ok(s) => set_status.set(Some(s)),
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
            <h2 class="text-xl font-semibold mb-4">"Dashboard"</h2>
            {move || error.get().map(|msg| view! { <ErrorBanner message=msg /> })}
            {move || match status.get() {
                None => view! { <LoadingSpinner /> }.into_any(),
                Some(s) => {
                    let (badge, label) = connection_badge(&s.status);
                    let is_connected = s.status == MatrixConnectionStatus::Connected;
                    view! {
                        <div class="space-y-3 text-sm">
                            <div class="flex items-center gap-2">
                                <span class="text-gray-600">"Matrix status:"</span>
                                <StatusBadge status=badge label=label.to_string() />
                            </div>
                            {is_connected.then(|| view! {
                                <div class="text-gray-600">
                                    "Account: " <span class="text-gray-900">
                                        {s.user_id.clone().unwrap_or_else(|| "—".to_owned())}
                                    </span>
                                </div>
                            })}
                            <div class="text-gray-600">
                                "Last connected: " <span class="text-gray-900">
                                    {s.connected_at.map(format_unix).unwrap_or_else(|| "never".to_owned())}
                                </span>
                            </div>
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}
