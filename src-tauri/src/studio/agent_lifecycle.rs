//! Pure lifecycle decisions for the durable Studio Director planning loop.
//!
//! Keeping thresholds and transitions here makes the expensive model/filesystem runner a thin
//! executor. Every restart and reviewer-exhaustion rule is testable without a model runtime.

use super::{StudioError, MAX_MOVIE_AGENT_SESSIONS};

const MAX_CONSECUTIVE_TOOL_FREE_TURNS: u32 = 3;
const MAX_INDEPENDENT_REVIEW_ROUNDS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnDecision {
    Continue,
    RestartSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewDecision {
    Repair,
    Exhausted,
}

#[derive(Debug)]
pub(super) struct AgentLifecycle {
    session: u32,
    absolute_step: u32,
    independent_review_round: u32,
    no_tool_streak: u32,
}

impl AgentLifecycle {
    pub(super) fn new() -> Self {
        Self {
            session: 1,
            absolute_step: 0,
            independent_review_round: 0,
            no_tool_streak: 0,
        }
    }

    pub(super) fn ensure_session_budget(&self) -> Result<(), StudioError> {
        if self.session > MAX_MOVIE_AGENT_SESSIONS {
            return Err(StudioError::Planning(format!(
                "The Studio Director did not submit a valid movie after {MAX_MOVIE_AGENT_SESSIONS} context sessions; the durable workspace is intact for a later retry"
            )));
        }
        Ok(())
    }

    pub(super) fn begin_step(&mut self) -> u32 {
        self.absolute_step = self.absolute_step.saturating_add(1);
        self.absolute_step
    }

    pub(super) fn record_model_turn(&mut self, used_workspace_tool: bool) -> TurnDecision {
        if used_workspace_tool {
            self.no_tool_streak = 0;
            return TurnDecision::Continue;
        }
        self.no_tool_streak = self.no_tool_streak.saturating_add(1);
        if self.no_tool_streak >= MAX_CONSECUTIVE_TOOL_FREE_TURNS {
            TurnDecision::RestartSession
        } else {
            TurnDecision::Continue
        }
    }

    pub(super) fn restart_session(&mut self) {
        self.session = self.session.saturating_add(1);
        self.no_tool_streak = 0;
    }

    pub(super) fn record_review_rejection(&mut self) -> ReviewDecision {
        self.independent_review_round = self.independent_review_round.saturating_add(1);
        if self.independent_review_round >= MAX_INDEPENDENT_REVIEW_ROUNDS {
            ReviewDecision::Exhausted
        } else {
            ReviewDecision::Repair
        }
    }

    pub(super) fn position(&self) -> (u32, u32) {
        (self.session, self.absolute_step)
    }

    pub(super) fn session(&self) -> u32 {
        self.session
    }

    pub(super) fn absolute_step(&self) -> u32 {
        self.absolute_step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn third_consecutive_tool_free_turn_restarts_in_a_fresh_context() {
        let mut lifecycle = AgentLifecycle::new();
        assert_eq!(lifecycle.record_model_turn(false), TurnDecision::Continue);
        assert_eq!(lifecycle.record_model_turn(false), TurnDecision::Continue);
        assert_eq!(
            lifecycle.record_model_turn(false),
            TurnDecision::RestartSession
        );
        lifecycle.restart_session();
        assert_eq!(lifecycle.position(), (2, 0));
        assert_eq!(lifecycle.record_model_turn(false), TurnDecision::Continue);
    }

    #[test]
    fn workspace_use_resets_the_tool_free_streak() {
        let mut lifecycle = AgentLifecycle::new();
        lifecycle.record_model_turn(false);
        lifecycle.record_model_turn(false);
        assert_eq!(lifecycle.record_model_turn(true), TurnDecision::Continue);
        assert_eq!(lifecycle.record_model_turn(false), TurnDecision::Continue);
        assert_eq!(lifecycle.record_model_turn(false), TurnDecision::Continue);
    }

    #[test]
    fn absolute_steps_survive_context_session_rollover() {
        let mut lifecycle = AgentLifecycle::new();
        assert_eq!(lifecycle.begin_step(), 1);
        lifecycle.restart_session();
        assert_eq!(lifecycle.begin_step(), 2);
        assert_eq!(lifecycle.position(), (2, 2));
    }

    #[test]
    fn third_independent_rejection_exhausts_the_repair_budget() {
        let mut lifecycle = AgentLifecycle::new();
        assert_eq!(lifecycle.record_review_rejection(), ReviewDecision::Repair);
        assert_eq!(lifecycle.record_review_rejection(), ReviewDecision::Repair);
        assert_eq!(
            lifecycle.record_review_rejection(),
            ReviewDecision::Exhausted
        );
    }

    #[test]
    fn session_budget_rejects_only_after_the_configured_number_of_sessions() {
        let mut lifecycle = AgentLifecycle::new();
        for _ in 0..MAX_MOVIE_AGENT_SESSIONS {
            lifecycle.ensure_session_budget().unwrap();
            lifecycle.restart_session();
        }
        assert!(lifecycle.ensure_session_budget().is_err());
    }
}
