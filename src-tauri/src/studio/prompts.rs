use super::MovieSettings;
use crate::prompt_catalog::{self, PromptId};

pub(super) fn movie_agent_system() -> String {
    prompt_catalog::text(PromptId::MovieAgentSystem)
}

pub(super) fn initial_instruction() -> String {
    prompt_catalog::text(PromptId::MovieInitial)
}

pub(super) fn resume_instruction() -> String {
    prompt_catalog::text(PromptId::MovieResume)
}

pub(super) fn continue_with_tools() -> String {
    prompt_catalog::text(PromptId::MovieContinue)
}
pub(super) fn first_clean_check() -> String {
    prompt_catalog::text(PromptId::MovieFirstCheck)
}
pub(super) fn second_clean_check() -> String {
    prompt_catalog::text(PromptId::MovieSecondCheck)
}
pub(super) fn submit_blocked() -> String {
    prompt_catalog::text(PromptId::MovieSubmitBlocked)
}

pub(super) fn response_checkpoint(error: &str) -> String {
    prompt_catalog::render(PromptId::MovieResponseCheckpoint, &[("error", error)])
}

pub(super) fn producer_direction(text: &str) -> String {
    prompt_catalog::render(PromptId::MovieProducerDirection, &[("direction", text)])
}

pub(super) fn independent_reviewer_system() -> String {
    prompt_catalog::text(PromptId::MovieReviewerSystem)
}

pub(super) fn generation_agent_system() -> String {
    prompt_catalog::text(PromptId::MovieGenerationAgentSystem)
}

pub(super) fn generation_frame_analyst_system() -> String {
    prompt_catalog::text(PromptId::MovieGenerationFrameAnalystSystem)
}

pub(super) fn generation_reviewer_system() -> String {
    prompt_catalog::text(PromptId::MovieGenerationReviewerSystem)
}

pub(super) fn generation_initial() -> String {
    prompt_catalog::text(PromptId::MovieGenerationInitial)
}

pub(super) fn generation_resume() -> String {
    prompt_catalog::text(PromptId::MovieGenerationResume)
}

pub(super) fn generation_continue() -> String {
    prompt_catalog::text(PromptId::MovieGenerationContinue)
}

pub(super) fn generation_review_rejected(review: &str) -> String {
    prompt_catalog::render(
        PromptId::MovieGenerationReviewRejected,
        &[("review", review)],
    )
}

pub(super) fn prompt_catalog(settings: &MovieSettings) -> Vec<super::planning::PromptDocument> {
    vec![
        super::planning::PromptDocument::new(
            "system",
            "Studio Director system prompt",
            "system",
            movie_agent_system(),
        ),
        super::planning::PromptDocument::new(
            "initial-instruction",
            "New planning session instruction",
            "instruction",
            initial_instruction(),
        ),
        super::planning::PromptDocument::new(
            "resume-instruction",
            "Checkpoint resume instruction",
            "instruction",
            resume_instruction(),
        ),
        super::planning::PromptDocument::new(
            "continue-with-tools",
            "Tool-use recovery instruction",
            "instruction",
            continue_with_tools(),
        ),
        super::planning::PromptDocument::new(
            "producer-direction-template",
            "Live producer direction wrapper",
            "instruction",
            producer_direction("{producer direction}"),
        ),
        super::planning::PromptDocument::new(
            "response-checkpoint-template",
            "Model response recovery instruction",
            "instruction",
            response_checkpoint("{local model error}"),
        ),
        super::planning::PromptDocument::new(
            "first-clean-check",
            "First clean native-check response",
            "lint",
            first_clean_check(),
        ),
        super::planning::PromptDocument::new(
            "second-clean-check",
            "Second clean native-check response",
            "lint",
            second_clean_check(),
        ),
        super::planning::PromptDocument::new(
            "submit-blocked",
            "Submit lint response",
            "lint",
            submit_blocked(),
        ),
        super::planning::PromptDocument::new(
            "native-lint-policy",
            "Native planning and lint limits",
            "lint",
            format!(
                "Maximum scenes: {}. Native canvas: {}x{}. Each H3 prompt must contain 120-450 words. A draft must pass native schema, producer-intent, reference-placement, continuity, prompt-quality, and full-review checks twice without intervening edits before submit is accepted.",
                settings.max_clips, settings.width, settings.height
            ),
        ),
        super::planning::PromptDocument::new(
            "independent-reviewer-system",
            "Independent whole-film reviewer prompt",
            "system",
            independent_reviewer_system(),
        ),
        super::planning::PromptDocument::new(
            "generation-frame-analyst-system",
            "Exact endpoint frame analyst prompt",
            "system",
            generation_frame_analyst_system(),
        ),
        super::planning::PromptDocument::new(
            "generation-agent-system",
            "Generative Director system prompt",
            "system",
            generation_agent_system(),
        ),
        super::planning::PromptDocument::new(
            "generation-reviewer-system",
            "Fresh-context generative reviewer prompt",
            "system",
            generation_reviewer_system(),
        ),
        super::planning::PromptDocument::new(
            "generation-initial",
            "Generative edit initial instruction",
            "instruction",
            generation_initial(),
        ),
        super::planning::PromptDocument::new(
            "generation-resume",
            "Generative edit resume instruction",
            "instruction",
            generation_resume(),
        ),
        super::planning::PromptDocument::new(
            "generation-continue",
            "Generative edit tool-use recovery",
            "instruction",
            generation_continue(),
        ),
        super::planning::PromptDocument::new(
            "generation-review-rejected",
            "Generative edit review-repair wrapper",
            "instruction",
            generation_review_rejected("{review JSON}"),
        ),
    ]
}
