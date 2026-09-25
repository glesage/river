//! One toast at a time, centred at the top of the UI.
//!
//! [`show_toast`] and [`show_error_toast`] put a message on screen, replacing
//! whatever toast was already showing: back-to-back actions refresh it rather
//! than queue. A normal toast disappears after [`TOAST_DURATION_MS`]; an error
//! toast stays until the user closes it or runs its action.
//!
//! [`ToastHost`], mounted once in `App` after every modal, draws it. It sits
//! above every modal and the room header, and never covers the composer or
//! the loading dots docked above it.

use dioxus::prelude::*;
use dioxus_free_icons::{
    icons::fa_solid_icons::{FaTriangleExclamation, FaXmark},
    Icon,
};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// How long a normal toast stays on screen.
pub(crate) const TOAST_DURATION_MS: u64 = 5_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ToastKind {
    Info,
    Error,
}

/// A button on the toast. `run` is called from the click handler, so it must
/// defer any global signal write itself (see
/// `.claude/rules/dioxus-signal-safety.md`); the toast closes afterwards.
#[derive(Clone)]
pub struct ToastAction {
    label: &'static str,
    run: Rc<dyn Fn()>,
}

impl ToastAction {
    pub fn new(label: &'static str, run: impl Fn() + 'static) -> Self {
        Self {
            label,
            run: Rc::new(run),
        }
    }
}

#[derive(Clone)]
struct Toast {
    /// Identity, so a timer or a click can tell whether the toast it was
    /// armed for is still the one showing.
    id: u64,
    message: String,
    kind: ToastKind,
    action: Option<ToastAction>,
}

/// The toast on screen, if any. Written only inside `crate::util::defer`.
static TOAST: GlobalSignal<Option<Toast>> = Global::new(|| None);

/// Monotonic, so two toasts shown in the same millisecond never share an id
/// and one's timer can't close the other.
static NEXT_TOAST_ID: AtomicU64 = AtomicU64::new(1);

fn next_toast_id() -> u64 {
    NEXT_TOAST_ID.fetch_add(1, Ordering::Relaxed)
}

/// Show `message` for [`TOAST_DURATION_MS`], with an optional action.
pub fn show_toast(message: impl Into<String>, action: Option<ToastAction>) {
    show(
        ToastKind::Info,
        message.into(),
        action,
        Some(TOAST_DURATION_MS),
    );
}

/// Show an error that stays until the user closes it or runs its action.
pub fn show_error_toast(message: impl Into<String>, action: Option<ToastAction>) {
    show(ToastKind::Error, message.into(), action, None);
}

fn show(kind: ToastKind, message: String, action: Option<ToastAction>, lifetime_ms: Option<u64>) {
    let id = next_toast_id();
    let toast = Toast {
        id,
        message,
        kind,
        action,
    };
    crate::util::defer(move || {
        *TOAST.write() = Some(toast);
    });
    if let Some(ms) = lifetime_ms {
        crate::util::safe_spawn_local(async move {
            crate::util::sleep(crate::util::millis(ms)).await;
            close(id);
        });
    }
}

/// Close toast `id`, if it is still the one showing: a newer toast, or one
/// already closed, is left alone.
fn close(id: u64) {
    crate::util::defer(move || {
        let showing = TOAST.peek().as_ref().map(|t| t.id);
        if is_current(showing, id) {
            *TOAST.write() = None;
        }
    });
}

fn is_current(showing: Option<u64>, id: u64) -> bool {
    showing == Some(id)
}

/// Draws the toast. Mounted once in `App`, after every modal, and never
/// unmounted.
#[component]
pub fn ToastHost() -> Element {
    // `read()`, not `try_read()`: every write is a single `set` inside
    // `defer`, so no borrow outlives a statement and a render cannot meet one
    // (the same reasoning as `network_activity_indicator::visible_activity`).
    let toast = TOAST.read().clone();
    let status = toast
        .as_ref()
        .map(|t| t.message.clone())
        .unwrap_or_default();

    rsx! {
        // Always mounted: a live region only announces changes to content that
        // was already in the DOM, so it must not come and go with the toast.
        span {
            class: "sr-only",
            role: "status",
            "aria-live": "polite",
            "data-testid": "toast-status",
            "{status}"
        }
        // Keyed by id, so each new toast remounts and replays its entrance.
        for t in toast.into_iter() {
            ToastCard { key: "{t.id}", toast: t }
        }
    }
}

#[component]
fn ToastCard(toast: Toast) -> Element {
    let id = toast.id;
    let is_error = toast.kind == ToastKind::Error;
    let card_class = if is_error {
        "river-toast bg-panel text-text border border-red-500/60 rounded-lg shadow-lg px-4 py-2 text-sm"
    } else {
        "river-toast bg-panel text-text border border-border rounded-lg shadow-lg px-4 py-2 text-sm"
    };
    rsx! {
        div {
            class: card_class,
            "data-testid": "toast",
            "data-kind": if is_error { "error" } else { "info" },
            if is_error {
                span { class: "text-red-500 flex-shrink-0",
                    Icon { width: 14, height: 14, icon: FaTriangleExclamation }
                }
            }
            span { class: "min-w-0 break-words", "{toast.message}" }
            if let Some(action) = toast.action.clone() {
                button {
                    class: "text-accent hover:underline font-medium flex-shrink-0",
                    "data-testid": "toast-action",
                    onclick: move |_| {
                        (action.run)();
                        close(id);
                    },
                    "{action.label}"
                }
            }
            button {
                class: "text-text-muted hover:text-text px-1 flex-shrink-0",
                "data-testid": "toast-dismiss",
                "aria-label": "Dismiss",
                onclick: move |_| close(id),
                Icon { width: 10, height: 10, icon: FaXmark }
            }
        }
    }
}

impl PartialEq for Toast {
    /// By id: a toast's content never changes after it is shown.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::source_scan::{production_only, strip_line_comments};

    #[test]
    fn toast_ids_are_distinct() {
        let a = next_toast_id();
        let b = next_toast_id();
        assert_ne!(a, b, "two toasts shown back to back must not share an id");
    }

    /// A timer or click armed for one toast must not close a newer one.
    #[test]
    fn only_the_current_toast_is_closed() {
        assert!(is_current(Some(7), 7));
        assert!(!is_current(Some(8), 7), "a newer toast is left alone");
        assert!(!is_current(None, 7), "an already-closed toast stays closed");
    }

    #[test]
    fn toasts_last_five_seconds() {
        assert_eq!(TOAST_DURATION_MS, 5_000);
    }

    /// The host must come after every modal in `App`, so its `z-index` never
    /// has to fight DOM order, and there must be exactly one.
    #[test]
    fn the_host_mounts_once_after_every_modal() {
        let app = strip_line_comments(production_only(include_str!("app.rs")));
        let host = app.find("ToastHost {}").expect("App must mount ToastHost");
        assert_eq!(app.matches("ToastHost {}").count(), 1);
        for modal in [
            "EditRoomModal {}",
            "NotificationModal {}",
            "MemberInfoModal {}",
            "CreateRoomModal {}",
            "DmThreadModal {}",
            "InviteViaDmPickerModal {}",
            "ReceiveInvitationModal {",
        ] {
            let at = app
                .find(modal)
                .unwrap_or_else(|| panic!("{modal} is no longer mounted in App; move the pin"));
            assert!(at < host, "ToastHost must come after {modal}");
        }
    }

    /// The card sits at the top, above every modal (z-50).
    #[test]
    fn the_toast_sits_at_the_top_above_the_modals() {
        let css = include_str!("../../assets/main.css");
        let rule = &css[css.find(".river-toast {").expect(".river-toast rule")..];
        let rule = &rule[..rule.find('}').unwrap()];
        assert!(rule.contains("position: fixed;"), "{rule}");
        assert!(
            rule.contains("top: max(1rem, env(safe-area-inset-top));"),
            "{rule}"
        );
        assert!(
            !rule.contains("bottom:"),
            "the toast belongs at the top: {rule}"
        );
        assert!(rule.contains("z-index: 60;"), "{rule}");
    }
}
