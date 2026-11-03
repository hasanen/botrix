//! Colored status pill component.

use leptos::prelude::*;

/// Visual status variants for the badge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadgeStatus {
    /// Green — everything is working.
    Connected,
    /// Yellow — transitional or warning state.
    Connecting,
    /// Red — error or failure.
    Error,
    /// Gray — no active session.
    Disconnected,
}

/// A colored pill that displays a status label.
#[component]
pub fn StatusBadge(status: BadgeStatus, label: String) -> impl IntoView {
    let css = match status {
        BadgeStatus::Connected => "bg-green-100 text-green-800",
        BadgeStatus::Connecting => "bg-yellow-100 text-yellow-800",
        BadgeStatus::Error => "bg-red-100 text-red-800",
        BadgeStatus::Disconnected => "bg-gray-100 text-gray-600",
    };

    view! {
        <span class=format!("inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium {css}")>
            {label}
        </span>
    }
}
