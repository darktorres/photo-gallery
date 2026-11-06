//! Traces of the pauses (S-18/S-12.18), in virtual time: each case is (now_ms, arm or fire) in order.

use pg_client::controller::deadlines::{Deadlines, Timer};

#[test]
fn a_rearm_moves_the_deadline_and_one_fire_comes_once() {
    let mut d = Deadlines::default();
    d.arm(Timer::ApplyQuery, 0);
    d.arm(Timer::ApplyQuery, 200);
    assert_eq!(d.next_deadline(), Some(500));
    assert_eq!(d.handle_timeout(299), vec![]);
    assert_eq!(d.handle_timeout(500), vec![Timer::ApplyQuery]);
    assert_eq!(d.handle_timeout(10_000), vec![]);
    assert!(!d.pending());
    assert_eq!(d.next_deadline(), None);
}

#[test]
fn three_edits_within_the_pause_give_one_list_at_the_deadline() {
    let mut d = Deadlines::default();
    d.arm(Timer::ApplyQuery, 0);
    d.arm(Timer::ApplyQuery, 100);
    d.arm(Timer::ApplyQuery, 250);
    assert_eq!(d.next_deadline(), Some(550));
    assert_eq!(d.handle_timeout(550), vec![Timer::ApplyQuery]);
}

#[test]
fn due_pauses_fire_in_the_order_they_were_due() {
    let mut d = Deadlines::default();
    d.arm(Timer::SaveSession, 0);
    d.arm(Timer::Layout, 0);
    d.arm(Timer::Size, 10);
    assert_eq!(d.next_deadline(), Some(50));
    assert_eq!(d.handle_timeout(2000), vec![Timer::Layout, Timer::Size, Timer::SaveSession]);
}

#[test]
fn a_late_timeout_fires_what_is_due_and_keeps_the_rest() {
    let mut d = Deadlines::default();
    d.arm(Timer::Layout, 0);
    d.arm(Timer::ApplyQuery, 0);
    assert_eq!(d.handle_timeout(100), vec![Timer::Layout]);
    d.arm(Timer::SaveSession, 100);
    assert_eq!(d.next_deadline(), Some(300));
    assert_eq!(d.handle_timeout(300), vec![Timer::ApplyQuery]);
    assert_eq!(d.next_deadline(), Some(1100));
}

#[test]
fn the_shutdown_flush_drains_every_pending_pause() {
    let mut d = Deadlines::default();
    d.arm(Timer::SaveSession, 0);
    d.arm(Timer::Size, 0);
    assert_eq!(d.drain(), vec![Timer::Size, Timer::SaveSession]);
    assert!(!d.pending());
}
