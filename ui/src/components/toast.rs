//! New toasts replace the current toast rather than queue.

use dioxus::prelude::*;
use dioxus_free_icons::{
    icons::fa_solid_icons::{FaTriangleExclamation, FaXmark},
    Icon,
};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};


const TOAST_DURATION_MS: u64 = 5_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ToastKind {
    Info,
    Error,
}

/// `run` executes in a click handler and must defer its own global signal writes.
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

    id: u64,
    message: String,
    kind: ToastKind,
    action: Option<ToastAction>,
}

// Written only inside `crate::util::defer`.
static TOAST: GlobalSignal<Option<Toast>> = Global::new(|| None);

// Unlike timestamps, unique even within one millisecond; stale timers cannot close a new toast.
static NEXT_TOAST_ID: AtomicU64 = AtomicU64::new(1);

fn next_toast_id() -> u64 {
    NEXT_TOAST_ID.fetch_add(1, Ordering::Relaxed)
}

/// Show `message` for [`TOAST_DURATION_MS`], with an optional action.
pub fn show_toast(message: impl Into<String>, action: Option<ToastAction>) {
    show(ToastKind::Info, message.into(), action);
}

/// Show an error that stays until the user closes it or runs its action.
pub fn show_error_toast(message: impl Into<String>, action: Option<ToastAction>) {
    show(ToastKind::Error, message.into(), action);
}

fn show(kind: ToastKind, message: String, action: Option<ToastAction>) {
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
    if kind == ToastKind::Info {
        crate::util::safe_spawn_local(async move {
            crate::util::sleep(crate::util::millis(TOAST_DURATION_MS)).await;
            close(id);
        });
    }
}


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

/// Keep mounted once in `App`, after the modals.
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
        // Keep the live region mounted so screen readers announce content changes.
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
    // Whole utility names, so Tailwind's source scan finds both.
    let border_class = if is_error {
        "border-red-500/60"
    } else {
        "border-border"
    };
    rsx! {
        div {
            class: "river-toast bg-panel text-text border {border_class} rounded-lg shadow-lg px-4 py-2 text-sm",
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


    #[test]
    fn the_toast_sits_at_the_top_above_the_modals() {
        let css = include_str!("../../assets/main.css");
        let rule = crate::util::source_scan::css_rule_body(css, ".river-toast");
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
