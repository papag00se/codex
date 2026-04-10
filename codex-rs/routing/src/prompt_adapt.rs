//! Prompt adaptation per model tier.
//!
//! Weaker models need more explicit prompts with chain-of-thought
//! scaffolding. Stronger models work better with concise prompts.
//! This module adapts prompts based on the target model tier.

use crate::classifier::RouteTarget;

/// Adapt a task prompt for the target model tier.
///
/// - Local models: add explicit step-by-step scaffolding
/// - Cloud fast/mini: moderate scaffolding
/// - Cloud reasoner/coder: concise, trust the model
pub fn adapt_prompt(prompt: &str, target: RouteTarget) -> String {
    match target {
        RouteTarget::LightReasoner | RouteTarget::LightCoder => {
            format!(
                "{prompt}\n\n\
                 Think through this step by step:\n\
                 1. Understand what is being asked\n\
                 2. Consider the key facts\n\
                 3. Give a clear, direct answer"
            )
        }
        RouteTarget::CloudFast => {
            // Moderate scaffolding — model is capable but not frontier
            format!("{prompt}\n\nBe concise and direct.")
        }
        RouteTarget::CloudMini | RouteTarget::CloudReasoner | RouteTarget::CloudCoder => {
            // No scaffolding — frontier models work better without hand-holding
            prompt.to_string()
        }
    }
}

/// Adapt a planning prompt for the target model tier.
pub fn adapt_planning_prompt(goal: &str, target: RouteTarget) -> String {
    match target {
        RouteTarget::LightReasoner | RouteTarget::LightCoder => {
            format!(
                "Break this goal into specific, actionable subtasks.\n\
                 Each task should be one clear action that can be done independently.\n\
                 Return JSON only.\n\n\
                 Goal: {goal}"
            )
        }
        _ => {
            // Frontier models understand the goal format without extra scaffolding
            goal.to_string()
        }
    }
}

/// Adapt an evaluation prompt for the target model tier.
pub fn adapt_evaluation_prompt(task_desc: &str, output: &str, target: RouteTarget) -> String {
    let truncated_output = if output.len() > 2000 { &output[..2000] } else { output };

    match target {
        RouteTarget::LightReasoner | RouteTarget::LightCoder => {
            format!(
                "Was this task completed successfully?\n\n\
                 Task: {task_desc}\n\n\
                 Output: {truncated_output}\n\n\
                 Answer YES if the task was done correctly, NO if it was not.\n\
                 Start your answer with YES or NO, then explain briefly."
            )
        }
        _ => {
            format!(
                "Evaluate: is this task complete?\n\
                 Task: {task_desc}\n\
                 Output: {truncated_output}\n\
                 Respond yes or no with brief reason."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_gets_scaffolding() {
        let adapted = adapt_prompt("What is X?", RouteTarget::LightReasoner);
        assert!(adapted.contains("step by step"));
        assert!(adapted.contains("What is X?"));
    }

    #[test]
    fn test_cloud_coder_no_scaffolding() {
        let adapted = adapt_prompt("What is X?", RouteTarget::CloudCoder);
        assert_eq!(adapted, "What is X?");
    }

    #[test]
    fn test_cloud_fast_concise() {
        let adapted = adapt_prompt("What is X?", RouteTarget::CloudFast);
        assert!(adapted.contains("concise"));
    }

    #[test]
    fn test_evaluation_local() {
        let adapted = adapt_evaluation_prompt("Fix bug", "Done", RouteTarget::LightReasoner);
        assert!(adapted.contains("YES"));
        assert!(adapted.contains("NO"));
    }
}
