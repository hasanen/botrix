//! Root application component — defines the SPA router and top-level layout.

use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

use crate::pages::{
    dashboard::DashboardPage, login::LoginPage, logs::LogsPage, matrix::MatrixPage,
    settings::SettingsPage, setup::SetupPage, webhooks::WebhooksPage,
};

/// Root component for the Botrix SPA.
#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| view! { <p>"Page not found."</p> }>
                <Route path=path!("/setup") view=SetupPage />
                <Route path=path!("/login") view=LoginPage />
                <Route path=path!("/") view=DashboardPage />
                <Route path=path!("/matrix") view=MatrixPage />
                <Route path=path!("/webhooks") view=WebhooksPage />
                <Route path=path!("/logs") view=LogsPage />
                <Route path=path!("/settings") view=SettingsPage />
            </Routes>
        </Router>
    }
}
