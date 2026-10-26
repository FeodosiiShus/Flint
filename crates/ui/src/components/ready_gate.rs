use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Context, Styled, Task};

/// How long a surface waits for the content it loads before it is shown with what it has: past
/// this it would rather jump to its final form later than keep the user waiting.
pub const READY_GATE_DEADLINE: Duration = Duration::from_millis(300);

/// The deadline, which `FLINT_READY_GATE_DEADLINE_MS` replaces: a UI script that films a surface
/// blocks the main thread for every frame it captures, so the work the surface waits for takes
/// longer than it does for a user, and a film needs a longer deadline to show the gate at work.
fn ready_gate_deadline() -> Duration {
    static DEADLINE: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *DEADLINE.get_or_init(|| {
        std::env::var("FLINT_READY_GATE_DEADLINE_MS")
            .ok()
            .and_then(|milliseconds| milliseconds.parse().ok())
            .map_or(READY_GATE_DEADLINE, Duration::from_millis)
    })
}

/// Keeps a surface unseen until the content it loads after it opens is in place, so it appears in
/// its final form: a popup or dialog that opens in a placeholder and then jumps to its rows reads
/// as a flicker. The surface stays laid out and keeps focus, so it takes input while it waits.
///
/// A surface that loads anything after it is created owns one, tells it whether the load is done
/// with [`Self::reveal_when`], and hides itself with [`GatedSurface::hidden_until`]:
///
/// ```ignore
/// self.gate.reveal_when(self.is_loaded());
/// v_flex().popup_surface(cx).hidden_until(self.gate.is_revealed())
/// ```
pub struct ReadyGate {
    revealed: bool,
    deadline_passed: Rc<Cell<bool>>,
    _deadline: Task<()>,
}

impl ReadyGate {
    pub fn new<V: 'static>(cx: &mut Context<V>) -> Self {
        let deadline_passed = Rc::new(Cell::new(false));
        let deadline = cx.spawn({
            let deadline_passed = deadline_passed.clone();
            async move |view, cx| {
                cx.background_executor().timer(ready_gate_deadline()).await;
                deadline_passed.set(true);
                view.update(cx, |_, cx| cx.notify()).ok();
            }
        });
        Self {
            revealed: false,
            deadline_passed,
            _deadline: deadline,
        }
    }

    /// Reveals the surface once `ready`, for good: content that changes later does not hide it
    /// again.
    pub fn reveal_when(&mut self, ready: bool) {
        self.revealed |= ready;
    }

    pub fn is_revealed(&self) -> bool {
        self.revealed || self.deadline_passed.get()
    }
}

pub trait GatedSurface: Styled + Sized {
    /// Draws nothing of this element, glass included, until `revealed`.
    fn hidden_until(self, revealed: bool) -> Self {
        if revealed { self } else { self.opacity(0.) }
    }
}

impl<E: Styled> GatedSurface for E {}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, Entity, TestAppContext};

    struct Host {
        gate: ReadyGate,
    }

    fn host(cx: &mut TestAppContext) -> Entity<Host> {
        cx.new(|cx| Host {
            gate: ReadyGate::new(cx),
        })
    }

    #[gpui::test]
    fn a_surface_stays_unseen_until_its_content_is_ready(cx: &mut TestAppContext) {
        let host = host(cx);
        assert!(!host.read_with(cx, |host, _| host.gate.is_revealed()));

        host.update(cx, |host, _| host.gate.reveal_when(false));
        assert!(!host.read_with(cx, |host, _| host.gate.is_revealed()));

        host.update(cx, |host, _| host.gate.reveal_when(true));
        assert!(host.read_with(cx, |host, _| host.gate.is_revealed()));
    }

    #[gpui::test]
    fn content_that_changes_later_does_not_hide_a_revealed_surface_again(cx: &mut TestAppContext) {
        let host = host(cx);
        host.update(cx, |host, _| host.gate.reveal_when(true));
        host.update(cx, |host, _| host.gate.reveal_when(false));
        assert!(host.read_with(cx, |host, _| host.gate.is_revealed()));
    }

    #[gpui::test]
    fn a_surface_whose_content_never_arrives_is_shown_after_the_deadline(cx: &mut TestAppContext) {
        let host = host(cx);
        assert!(!host.read_with(cx, |host, _| host.gate.is_revealed()));

        cx.executor().advance_clock(READY_GATE_DEADLINE);
        assert!(host.read_with(cx, |host, _| host.gate.is_revealed()));
    }
}
