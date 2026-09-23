use codex_app_server_client::InProcessServerEvent;
use codex_app_server_protocol::ThreadGoal;
use codex_app_server_protocol::ThreadGoalStatus;

#[derive(Debug, Default)]
pub(crate) struct ExecGoalState {
    state: GoalState,
    pending_activation: Option<ThreadGoal>,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum GoalState {
    #[default]
    Disabled,
    Active,
    Terminal,
}

impl ExecGoalState {
    pub(crate) fn activate(goal: ThreadGoal) -> Self {
        eprintln!(
            "Goal activation acknowledged: thread={} status={:?} updated_at={}",
            goal.thread_id, goal.status, goal.updated_at
        );
        Self {
            state: GoalState::Active,
            pending_activation: Some(goal),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.state == GoalState::Active
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.state != GoalState::Disabled
    }

    pub(crate) fn activation_pending(&self) -> bool {
        self.pending_activation.is_some()
    }

    pub(crate) fn check_event(&self, event: Option<&InProcessServerEvent>) -> anyhow::Result<()> {
        match event {
            None if self.is_active() => {
                anyhow::bail!("Goal event stream closed before goal completion");
            }
            Some(InProcessServerEvent::Lagged { skipped }) if self.activation_pending() => {
                anyhow::bail!("Goal activation unconfirmed after losing {skipped} events");
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn update_from_goal(&mut self, goal: &ThreadGoal) {
        if let Some(expected) = &self.pending_activation {
            // Resume can queue old snapshots before goal/set reactivates the goal.
            // The server orders this exact snapshot before activation effects;
            // use it as a fence. Second-precision timestamps alone are insufficient.
            if goal != expected {
                eprintln!(
                    "Ignoring pre-activation goal snapshot: thread={} status={:?} updated_at={}",
                    goal.thread_id, goal.status, goal.updated_at
                );
                return;
            }
            self.pending_activation = None;
            eprintln!("Goal activation observed: thread={}", goal.thread_id);
        }
        eprintln!(
            "Goal state notification: thread={} status={:?} updated_at={}",
            goal.thread_id, goal.status, goal.updated_at
        );
        self.state = match goal.status {
            ThreadGoalStatus::Active => GoalState::Active,
            ThreadGoalStatus::Paused
            | ThreadGoalStatus::Blocked
            | ThreadGoalStatus::UsageLimited
            | ThreadGoalStatus::BudgetLimited
            | ThreadGoalStatus::Complete => GoalState::Terminal,
        };
    }

    pub(crate) fn clear(&mut self) {
        if self.activation_pending() {
            // Ignore queued "no goal" resume snapshots; real clears follow activation.
            eprintln!("Ignoring pre-activation goal-cleared snapshot");
            return;
        }
        eprintln!("Goal cleared after activation");
        self.state = GoalState::Terminal;
    }
}

#[cfg(test)]
#[path = "goal_state_tests.rs"]
mod tests;
