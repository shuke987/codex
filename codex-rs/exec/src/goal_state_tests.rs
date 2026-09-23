use super::*;
use pretty_assertions::assert_eq;

fn active_goal() -> ThreadGoal {
    ThreadGoal {
        thread_id: "review-thread".to_string(),
        objective: "continue this review".to_string(),
        status: ThreadGoalStatus::Active,
        token_budget: None,
        tokens_used: 100,
        time_used_seconds: 20,
        created_at: 1,
        updated_at: 2,
    }
}

#[test]
fn old_terminal_snapshots_cannot_stop_reactivation() {
    for status in [
        ThreadGoalStatus::Blocked,
        ThreadGoalStatus::Paused,
        ThreadGoalStatus::UsageLimited,
        ThreadGoalStatus::BudgetLimited,
        ThreadGoalStatus::Complete,
    ] {
        let active = active_goal();
        let mut state = ExecGoalState::activate(active.clone());
        let mut old = active.clone();
        old.status = status;
        // The timestamps deliberately match: compare the actual snapshot.
        state.update_from_goal(&old);
        assert_eq!(
            (state.is_active(), state.activation_pending()),
            (true, true)
        );
        state.update_from_goal(&active);
        assert_eq!(
            (state.is_active(), state.activation_pending()),
            (true, false)
        );
        // The same stopped status is authoritative after the activation fence.
        state.update_from_goal(&old);
        assert_eq!(
            (state.is_active(), state.activation_pending()),
            (false, false)
        );
    }
}

#[test]
fn unrelated_active_snapshot_does_not_open_activation_fence() {
    let active = active_goal();
    let mut state = ExecGoalState::activate(active.clone());
    let mut old_active = active.clone();
    old_active.objective = "previous objective".to_string();
    state.update_from_goal(&old_active);
    let mut old_blocked = old_active;
    old_blocked.status = ThreadGoalStatus::Blocked;
    state.update_from_goal(&old_blocked);
    assert_eq!(
        (state.is_active(), state.activation_pending()),
        (true, true)
    );
    state.update_from_goal(&active);
    assert_eq!(
        (state.is_active(), state.activation_pending()),
        (true, false)
    );
}

#[test]
fn only_pre_activation_clear_is_ignored() {
    let active = active_goal();
    let mut state = ExecGoalState::activate(active.clone());
    state.clear();
    assert_eq!(
        (state.is_active(), state.activation_pending()),
        (true, true)
    );
    state.update_from_goal(&active);
    state.clear();
    assert_eq!(
        (state.is_active(), state.activation_pending()),
        (false, false)
    );
}

#[test]
fn lost_activation_notification_fails_instead_of_waiting_for_a_terminal_snapshot() {
    let active = active_goal();
    let mut state = ExecGoalState::activate(active.clone());
    let lost = InProcessServerEvent::Lagged { skipped: 1 };
    assert!(state.check_event(Some(&lost)).is_err());
    let mut terminal = active.clone();
    terminal.status = ThreadGoalStatus::Complete;
    state.update_from_goal(&terminal);
    assert!(state.check_event(Some(&lost)).is_err());

    state.update_from_goal(&active);
    // Keep the existing policy for best-effort event loss after activation.
    assert!(state.check_event(Some(&lost)).is_ok());
}

#[test]
fn closed_event_stream_fails_until_the_goal_has_stopped() {
    let active = active_goal();
    let mut state = ExecGoalState::activate(active.clone());
    assert!(state.check_event(None).is_err());
    state.update_from_goal(&active);
    assert!(state.check_event(None).is_err());
    let mut terminal = active;
    terminal.status = ThreadGoalStatus::Complete;
    state.update_from_goal(&terminal);
    assert!(state.check_event(None).is_ok());
    assert!(ExecGoalState::default().check_event(None).is_ok());
}
