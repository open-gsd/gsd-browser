//! Natural-language instruction planning for generic browser actions.
//!
//! This module intentionally avoids task- or site-specific names. It translates
//! short user instructions into existing primitive handlers by combining verb
//! classification with live DOM affordances.

mod action_runtime;
mod model;
mod page_model;
mod parser;
mod planner;
mod planner_js;
mod verification;
mod workflow_runtime;

use crate::daemon::capture::capture_compact_page_state;
use crate::daemon::handlers;
use crate::daemon::logs::DaemonLogs;
use crate::daemon::state::DaemonState;
use action_runtime::{
    handle_autocomplete_select, handle_click_ordered_values, handle_command_surface_action,
    handle_conditional_value_action, handle_derive_and_act, handle_discover_click,
    handle_draw_path, handle_feedback_loop_value, handle_focus_element, handle_format_text,
    handle_generate_constrained_value, handle_orient_visual, handle_read_text,
    handle_record_property_click, handle_scoped_menu_click, handle_scroll_element,
    handle_scroll_text_extract, handle_select_menu_path, handle_select_text,
    handle_set_checkbox_grid, handle_set_slider, handle_tree_search_click,
    handle_visual_feedback_search,
};
use chromiumoxide::{Browser, Page};
use model::InstructionKind;
use page_model::capture_instruction_page_model;
use parser::{analyze_instruction, build_intent};
use planner::build_plan;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;
use verification::verify_action_effect;
use workflow_runtime::{handle_date_picker, handle_form_workflow, handle_scoped_item_workflow};

const PLAN_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MAX_STEPS: usize = 8;

/// Handle `act_instruction`.
///
/// Params: { instruction: string, dry_run?: bool, scope?: string,
/// min_confidence?: number, max_steps?: number }
pub async fn handle_act_instruction(
    _page: &Page,
    logs: &DaemonLogs,
    state: &DaemonState,
    browser: &Arc<Mutex<Browser>>,
    params: &Value,
) -> Result<Value, String> {
    let instruction = params
        .get("instruction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: instruction".to_string())?;
    let dry_run = params
        .get("dry_run")
        .or_else(|| params.get("dryRun"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let scope = params.get("scope").and_then(|v| v.as_str());
    let min_confidence = params
        .get("min_confidence")
        .or_else(|| params.get("minConfidence"))
        .and_then(|v| v.as_f64());
    let max_steps = params
        .get("max_steps")
        .or_else(|| params.get("maxSteps"))
        .and_then(|v| v.as_u64())
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_STEPS);

    let analysis = analyze_instruction(instruction);
    let intent = build_intent(instruction, &analysis);
    if analysis.kind == InstructionKind::Unknown {
        return Err(format!(
            "act_instruction: could not infer a generic browser action from instruction: {instruction}"
        ));
    }

    let mut active_page = state
        .pages
        .lock()
        .map_err(|_| "act_instruction: page registry lock poisoned".to_string())?
        .active_page()
        .ok_or_else(|| "act_instruction: no active page in registry".to_string())?;

    let before_model = match capture_instruction_page_model_with_retry(
        active_page.as_ref(),
        state,
        browser,
        scope,
    )
    .await?
    {
        (model, Some(refreshed_page)) => {
            active_page = refreshed_page;
            model
        }
        (model, None) => model,
    };
    let mut plan = build_plan(active_page.as_ref(), instruction, &analysis, &intent, scope).await?;
    annotate_plan_context(&mut plan, &before_model);
    let step_count = plan_step_count(&plan);
    if step_count > max_steps {
        return Ok(json!({
            "instruction": instruction,
            "analysis": analysis.to_json(),
            "intent": intent.to_json(),
            "plan": plan,
            "pageModel": before_model,
            "blocked": true,
            "blockReason": "max_steps_exceeded",
            "message": format!("act_instruction planned {step_count} steps, above max_steps={max_steps}; rerun with dry_run, a narrower scope, or a higher max_steps if this is intended"),
            "dryRun": dry_run,
        }));
    }
    if let Some(min_confidence) = min_confidence {
        let confidence = plan_confidence(&plan);
        if confidence < min_confidence {
            return Ok(json!({
                "instruction": instruction,
                "analysis": analysis.to_json(),
                "intent": intent.to_json(),
                "plan": plan,
                "pageModel": before_model,
                "blocked": true,
                "blockReason": "confidence_below_threshold",
                "message": format!("act_instruction confidence {confidence:.3} is below min_confidence={min_confidence:.3}; inspect the plan or lower the threshold to execute"),
                "dryRun": dry_run,
            }));
        }
    }
    if dry_run {
        return Ok(json!({
            "instruction": instruction,
            "analysis": analysis.to_json(),
            "intent": intent.to_json(),
            "plan": plan,
            "pageModel": before_model,
            "dryRun": true,
        }));
    }

    let (executed_plan, result, fallback_attempts) =
        execute_planned_action_with_fallbacks(active_page.as_ref(), logs, state, &plan).await?;
    plan = executed_plan;
    let after_model = match capture_instruction_page_model_with_retry(
        active_page.as_ref(),
        state,
        browser,
        scope,
    )
    .await?
    {
        (model, Some(refreshed_page)) => {
            active_page = refreshed_page;
            model
        }
        (model, None) => model,
    };
    let verification = verify_action_effect(
        instruction,
        &analysis,
        &plan,
        &before_model,
        &after_model,
        &result,
    );

    let mut response = json!({
        "instruction": instruction,
        "analysis": analysis.to_json(),
        "intent": intent.to_json(),
        "plan": plan,
        "result": result,
        "verification": verification,
        "pageModel": {
            "before": before_model,
            "after": after_model,
        },
        "state": capture_compact_page_state(active_page.as_ref(), false).await,
    });
    if let Some(fallback_attempts) = fallback_attempts {
        if let Some(object) = response.as_object_mut() {
            object.insert(
                "execution".to_string(),
                json!({ "fallbackAttempts": fallback_attempts }),
            );
        }
    }
    Ok(response)
}

async fn capture_instruction_page_model_with_retry(
    page: &Page,
    state: &DaemonState,
    browser: &Arc<Mutex<Browser>>,
    scope: Option<&str>,
) -> Result<(Value, Option<Arc<Page>>), String> {
    let mut last_error = None;
    let mut refreshed_page: Option<Arc<Page>> = None;
    for attempt in 0..3 {
        let current_page: &Page = match refreshed_page.as_ref() {
            Some(page) => page.as_ref(),
            None => page,
        };
        match capture_instruction_page_model(current_page, scope).await {
            Ok(model) => return Ok((model, refreshed_page)),
            Err(error) if is_transient_cdp_session_error(&error) && attempt < 2 => {
                last_error = Some(error);
                if let Ok(page) =
                    super::pages::refresh_active_page_from_browser(browser, state).await
                {
                    refreshed_page = Some(page);
                }
                sleep(Duration::from_millis(120 * (attempt + 1) as u64)).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "page model capture failed".to_string()))
}

fn is_transient_cdp_session_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("session with given id not found")
        || lower.contains("target closed")
        || lower.contains("target detached")
        || lower.contains("session closed")
        || lower.contains("receiver is gone")
}

fn plan_step_count(plan: &Value) -> usize {
    if plan.get("action").and_then(|v| v.as_str()) == Some("sequence") {
        plan.get("steps")
            .and_then(|v| v.as_array())
            .map(|steps| steps.len())
            .unwrap_or(0)
    } else {
        1
    }
}

fn plan_confidence(plan: &Value) -> f64 {
    plan.get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

fn annotate_plan_context(plan: &mut Value, page_model: &Value) {
    let summary = page_model
        .get("summary")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let element_count = page_model
        .get("elements")
        .and_then(|value| value.as_array())
        .map(|elements| elements.len())
        .unwrap_or(0);
    if let Some(object) = plan.as_object_mut() {
        object.insert(
            "planner".to_string(),
            json!({
                "pageModelVersion": page_model.get("version").and_then(|value| value.as_i64()).unwrap_or(1),
                "pageModelSummary": summary,
                "candidateCount": element_count,
                "strategy": "intent-page-model-candidate-score-v1",
            }),
        );
    }
}

async fn dispatch_planned_action(
    page: &Page,
    logs: &DaemonLogs,
    state: &DaemonState,
    plan: &Value,
) -> Result<Value, String> {
    let action = plan
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "act_instruction: planner returned no action".to_string())?;
    let params = plan.get("params").cloned().unwrap_or_else(|| json!({}));
    match action {
        "click" => handlers::interaction::handle_click(page, state, &params).await,
        "focus" => handle_focus_element(page, state, &params).await,
        "hover" => handlers::interaction::handle_hover(page, state, &params).await,
        "type" => handlers::interaction::handle_type_text(page, state, &params).await,
        "select_option" => handlers::interaction::handle_select_option(page, state, &params).await,
        "set_checked" => handlers::interaction::handle_set_checked(page, state, &params).await,
        "upload_file" => handlers::interaction::handle_upload_file(page, state, &params).await,
        "press" => handlers::interaction::handle_press(page, state, &params).await,
        "set_checkbox_grid" => handle_set_checkbox_grid(page, &params).await,
        "set_slider" => handle_set_slider(page, &params).await,
        "autocomplete_select" => handle_autocomplete_select(page, state, &params).await,
        "scoped_item_workflow" => handle_scoped_item_workflow(page, &params).await,
        "scoped_menu_click" => handle_scoped_menu_click(page, &params).await,
        "form_workflow" => handle_form_workflow(page, &params).await,
        "date_picker" => handle_date_picker(page, &params).await,
        "derive_and_act" => handle_derive_and_act(page, &params).await,
        "generate_constrained_value" => handle_generate_constrained_value(page, &params).await,
        "feedback_loop_value" => handle_feedback_loop_value(page, &params).await,
        "conditional_value_action" => handle_conditional_value_action(page, &params).await,
        "command_surface_action" => handle_command_surface_action(page, &params).await,
        "discover_click" => handle_discover_click(page, state, &params).await,
        "visual_feedback_search" => handle_visual_feedback_search(page, &params).await,
        "tree_search_click" => handle_tree_search_click(page, &params).await,
        "record_property_click" => handle_record_property_click(page, &params).await,
        "click_ordered_values" => handle_click_ordered_values(page, &params).await,
        "select_menu_path" => handle_select_menu_path(page, &params).await,
        "select_text" => handle_select_text(page, &params).await,
        "format_text" => handle_format_text(page, &params).await,
        "scroll_element" => handle_scroll_element(page, &params).await,
        "scroll_text_extract" => handle_scroll_text_extract(page, &params).await,
        "orient_visual" => handle_orient_visual(page, &params).await,
        "draw_path" => handle_draw_path(page, &params).await,
        "drag" => handlers::interaction::handle_drag(page, state, &params).await,
        "scroll" => handlers::interaction::handle_scroll(page, state, &params).await,
        "set_viewport" => handlers::interaction::handle_set_viewport(page, &params).await,
        "emulate_device" => handlers::device::handle_emulate_device(page, &params).await,
        "read_text" => handle_read_text(page, &params).await,
        "analyze_form" => handlers::forms::handle_analyze_form(page, &params).await,
        "accessibility_tree" => handlers::inspect::handle_accessibility_tree(page, &params).await,
        "find" => handlers::inspect::handle_find(page, state, &params).await,
        "navigate" => handlers::navigate::handle_navigate(page, &params, state).await,
        "back" => handlers::navigate::handle_back(page, state).await,
        "forward" => handlers::navigate::handle_forward(page, state).await,
        "reload" => handlers::navigate::handle_reload(page, state).await,
        "screenshot" => handlers::screenshot::handle_screenshot(page, &params).await,
        "assert" => {
            let result = handlers::assert_cmd::handle_assert(page, logs, state, &params).await?;
            if !result
                .get("verified")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                let summary = result
                    .get("summary")
                    .and_then(|value| value.as_str())
                    .unwrap_or("assertion failed");
                return Err(format!("act_instruction: assertion failed: {summary}"));
            }
            Ok(result)
        }
        "wait_for" => {
            let result = handlers::wait::handle_wait_for(page, logs, state, &params).await?;
            if !result
                .get("met")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                let condition = result
                    .get("condition")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                let value = result
                    .get("value")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                return Err(format!(
                    "act_instruction: wait_for condition '{condition}' was not met for '{value}'"
                ));
            }
            Ok(result)
        }
        other => Err(format!(
            "act_instruction: unsupported planned action: {other}"
        )),
    }
}

async fn execute_plan_without_fallbacks(
    page: &Page,
    logs: &DaemonLogs,
    state: &DaemonState,
    plan: &Value,
) -> Result<Value, String> {
    let action = plan
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "act_instruction: planner returned no action".to_string())?;
    if action == "sequence" {
        let steps = plan
            .get("steps")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "act_instruction: sequence plan has no steps".to_string())?;
        let mut results = Vec::with_capacity(steps.len());
        for step in steps {
            results.push(dispatch_planned_action(page, logs, state, step).await?);
        }
        Ok(json!({ "steps": results }))
    } else {
        dispatch_planned_action(page, logs, state, plan).await
    }
}

async fn execute_planned_action_with_fallbacks(
    page: &Page,
    logs: &DaemonLogs,
    state: &DaemonState,
    plan: &Value,
) -> Result<(Value, Value, Option<Value>), String> {
    let primary_action = plan
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "act_instruction: planner returned no action".to_string())?;
    match execute_plan_without_fallbacks(page, logs, state, plan).await {
        Ok(result) => Ok((plan.clone(), result, None)),
        Err(primary_error) => {
            if primary_action == "sequence" {
                return Err(primary_error);
            }
            let alternates = match plan
                .get("alternatePlans")
                .and_then(|value| value.as_array())
            {
                Some(alternates) if !alternates.is_empty() => alternates,
                _ => return Err(primary_error),
            };
            let mut attempts = vec![json!({
                "action": primary_action,
                "capability": plan.get("capability").cloned().unwrap_or(Value::Null),
                "ok": false,
                "error": primary_error,
            })];
            for alternate in alternates.iter().take(3) {
                let alternate_action = alternate
                    .get("action")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                match execute_plan_without_fallbacks(page, logs, state, alternate).await {
                    Ok(result) => {
                        let mut executed_plan = alternate.clone();
                        if let Some(object) = executed_plan.as_object_mut() {
                            object.insert(
                                "fallback".to_string(),
                                json!({
                                    "used": true,
                                    "primaryAction": primary_action,
                                    "attemptCount": attempts.len() + 1,
                                }),
                            );
                        }
                        attempts.push(json!({
                            "action": alternate_action,
                            "capability": alternate.get("capability").cloned().unwrap_or(Value::Null),
                            "ok": true,
                        }));
                        return Ok((executed_plan, result, Some(json!(attempts))));
                    }
                    Err(error) => attempts.push(json!({
                        "action": alternate_action,
                        "capability": alternate.get("capability").cloned().unwrap_or(Value::Null),
                        "ok": false,
                        "error": error,
                    })),
                }
            }
            let errors = attempts
                .iter()
                .filter_map(|attempt| {
                    let action = attempt.get("action").and_then(|value| value.as_str())?;
                    let error = attempt.get("error").and_then(|value| value.as_str())?;
                    Some(format!("{action}: {error}"))
                })
                .collect::<Vec<_>>()
                .join("; ");
            Err(format!(
                "act_instruction: primary plan and alternate plans failed: {errors}"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_generic_actions() {
        assert_eq!(
            analyze_instruction("click the Continue button").kind,
            InstructionKind::Click
        );
        assert_eq!(
            analyze_instruction(
                "Reserve the cheapest service option from: Origin Alpha to: Destination Beta on 07/09/2026."
            )
            .kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction(
                "Find the cheapest service option from: Origin Alpha to: Destination Beta on 07/09/2026."
            )
            .kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction(
                "Show me the cheapest service option from: Origin Alpha to: Destination Beta on 07/09/2026."
            )
            .kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction(
                "Can you show me the cheapest service option from: Origin Alpha to: Destination Beta on 07/09/2026?"
            )
            .kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction(
                "I need the cheapest service option from: Origin Alpha to: Destination Beta on 07/09/2026."
            )
            .kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction(
                "I need the cheapest service option between Origin Alpha and Destination Beta for July 9, 2026."
            )
            .kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction("Create a 90 mins event named \"Gym\", between 12PM and 4PM.").kind,
            InstructionKind::Fill
        );
        let wait_visible = analyze_instruction("Wait until Results appears");
        assert_eq!(wait_visible.kind, InstructionKind::Wait);
        assert_eq!(wait_visible.direction.as_deref(), Some("visible"));
        assert_eq!(wait_visible.value.as_deref(), Some("Results"));
        let wait_hidden = analyze_instruction("Wait for Loading to disappear");
        assert_eq!(wait_hidden.kind, InstructionKind::Wait);
        assert_eq!(wait_hidden.direction.as_deref(), Some("hidden"));
        assert_eq!(wait_hidden.value.as_deref(), Some("Loading"));
        let navigate = analyze_instruction("Open about:blank#agent-target");
        assert_eq!(navigate.kind, InstructionKind::Navigate);
        assert_eq!(navigate.direction.as_deref(), Some("url"));
        assert_eq!(navigate.value.as_deref(), Some("about:blank#agent-target"));
        let localhost = analyze_instruction("Go to localhost:3000");
        assert_eq!(localhost.kind, InstructionKind::Navigate);
        assert_eq!(localhost.value.as_deref(), Some("http://localhost:3000"));
        let back = analyze_instruction("Go back");
        assert_eq!(back.kind, InstructionKind::Navigate);
        assert_eq!(back.direction.as_deref(), Some("back"));
        let forward = analyze_instruction("Go forward");
        assert_eq!(forward.kind, InstructionKind::Navigate);
        assert_eq!(forward.direction.as_deref(), Some("forward"));
        let reload = analyze_instruction("Reload the page");
        assert_eq!(reload.kind, InstructionKind::Navigate);
        assert_eq!(reload.direction.as_deref(), Some("reload"));
        let viewport_preset = analyze_instruction("Set viewport to mobile");
        assert_eq!(viewport_preset.kind, InstructionKind::SetViewport);
        assert_eq!(viewport_preset.direction.as_deref(), Some("preset"));
        assert_eq!(viewport_preset.value.as_deref(), Some("mobile"));
        let viewport_dimensions = analyze_instruction("Resize browser to 390x844");
        assert_eq!(viewport_dimensions.kind, InstructionKind::SetViewport);
        assert_eq!(viewport_dimensions.direction.as_deref(), Some("dimensions"));
        assert_eq!(viewport_dimensions.value.as_deref(), Some("390x844"));
        let emulate = analyze_instruction("Emulate iPhone 15");
        assert_eq!(emulate.kind, InstructionKind::EmulateDevice);
        assert_eq!(emulate.direction.as_deref(), Some("device"));
        assert_eq!(emulate.value.as_deref(), Some("iPhone 15"));
        let read_page = analyze_instruction("Read the page");
        assert_eq!(read_page.kind, InstructionKind::ReadText);
        assert_eq!(read_page.direction.as_deref(), Some("visible_text"));
        assert_eq!(read_page.target_hint, None);
        let read_selector = analyze_instruction("Extract text from #status-panel");
        assert_eq!(read_selector.kind, InstructionKind::ReadText);
        assert_eq!(read_selector.target_hint.as_deref(), Some("#status-panel"));
        let read_semantic = analyze_instruction("Read the Profile card");
        assert_eq!(read_semantic.kind, InstructionKind::ReadText);
        assert_eq!(read_semantic.target_hint.as_deref(), Some("Profile card"));
        let analyze_form = analyze_instruction("Analyze the Shipping form");
        assert_eq!(analyze_form.kind, InstructionKind::AnalyzeForm);
        assert_eq!(analyze_form.direction.as_deref(), Some("fields"));
        assert_eq!(analyze_form.target_hint.as_deref(), Some("Shipping"));
        let analyze_selector = analyze_instruction("Inspect form fields in #shipping");
        assert_eq!(analyze_selector.kind, InstructionKind::AnalyzeForm);
        assert_eq!(analyze_selector.target_hint.as_deref(), Some("#shipping"));
        let tree_page = analyze_instruction("Show accessibility tree");
        assert_eq!(tree_page.kind, InstructionKind::AccessibilityTree);
        assert_eq!(tree_page.direction.as_deref(), Some("tree"));
        assert_eq!(tree_page.target_hint, None);
        let tree_selector = analyze_instruction("Show accessibility tree for #nav");
        assert_eq!(tree_selector.kind, InstructionKind::AccessibilityTree);
        assert_eq!(tree_selector.target_hint.as_deref(), Some("#nav"));
        let tree_semantic = analyze_instruction("List roles in the Profile card");
        assert_eq!(tree_semantic.kind, InstructionKind::AccessibilityTree);
        assert_eq!(tree_semantic.target_hint.as_deref(), Some("Profile card"));
        let find_button = analyze_instruction("Find buttons named Save profile");
        assert_eq!(find_button.kind, InstructionKind::FindElements);
        assert_eq!(find_button.direction.as_deref(), Some("role_text"));
        assert_eq!(find_button.value.as_deref(), Some("Save profile"));
        let find_selector = analyze_instruction("Locate #profile");
        assert_eq!(find_selector.kind, InstructionKind::FindElements);
        assert_eq!(find_selector.direction.as_deref(), Some("selector"));
        assert_eq!(find_selector.value.as_deref(), Some("#profile"));
        let find_text = analyze_instruction("Search for Account status");
        assert_eq!(find_text.kind, InstructionKind::FindElements);
        assert_eq!(find_text.direction.as_deref(), Some("text"));
        assert_eq!(find_text.value.as_deref(), Some("Account status"));
        let find_then_reply =
            analyze_instruction("Find the message by Carlynne and reply with \"Ornare commodo\".");
        assert_ne!(find_then_reply.kind, InstructionKind::FindElements);
        let find_then_toggle =
            analyze_instruction("Find the message by Carlynne and turn on alerts.");
        assert_ne!(find_then_toggle.kind, InstructionKind::FindElements);
        assert_eq!(
            analyze_instruction("Forward Anitra's e-mail to Loralyn.").kind,
            InstructionKind::Click
        );
        assert_eq!(
            analyze_instruction("Please forward the information from Margaretta to Roslyn.").kind,
            InstructionKind::Click
        );
        assert_eq!(
            analyze_instruction("Send the email from Chrysa to Misty.").kind,
            InstructionKind::Click
        );
        assert_eq!(
            analyze_instruction(
                "Respond \"Vitae mattis dictum. Ut.\" to the email sent by Chrysa."
            )
            .kind,
            InstructionKind::Click
        );
        assert_eq!(
            analyze_instruction(
                "Write \"Launch update approved\" into the Message body editor and press Send."
            )
            .kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction("Enter an item that starts with \"Com\" and ends with \"os\".")
                .kind,
            InstructionKind::Fill
        );
        let assert_visible = analyze_instruction("Verify that Success message is visible");
        assert_eq!(assert_visible.kind, InstructionKind::Assert);
        assert_eq!(assert_visible.direction.as_deref(), Some("visible"));
        assert_eq!(assert_visible.value.as_deref(), Some("Success message"));
        let assert_hidden = analyze_instruction("Check that #spinner is hidden");
        assert_eq!(assert_hidden.kind, InstructionKind::Assert);
        assert_eq!(assert_hidden.direction.as_deref(), Some("hidden"));
        assert_eq!(assert_hidden.value.as_deref(), Some("#spinner"));
        let assert_url = analyze_instruction("Expect URL contains #done");
        assert_eq!(assert_url.kind, InstructionKind::Assert);
        assert_eq!(assert_url.direction.as_deref(), Some("url_contains"));
        assert_eq!(assert_url.value.as_deref(), Some("#done"));
        let assert_logs = analyze_instruction("Ensure no console errors");
        assert_eq!(assert_logs.kind, InstructionKind::Assert);
        assert_eq!(assert_logs.direction.as_deref(), Some("no_console_errors"));
        let assert_value = analyze_instruction("Verify that Email value equals alice@example.com");
        assert_eq!(assert_value.kind, InstructionKind::Assert);
        assert_eq!(assert_value.direction.as_deref(), Some("value_equals"));
        assert_eq!(assert_value.target_hint.as_deref(), Some("Email"));
        assert_eq!(assert_value.value.as_deref(), Some("alice@example.com"));
        let assert_checked = analyze_instruction("Ensure newsletter checkbox is checked");
        assert_eq!(assert_checked.kind, InstructionKind::Assert);
        assert_eq!(assert_checked.direction.as_deref(), Some("checked"));
        assert_eq!(
            assert_checked.target_hint.as_deref(),
            Some("newsletter checkbox")
        );
        let assert_unchecked = analyze_instruction("Expect beta access checkbox is not checked");
        assert_eq!(assert_unchecked.kind, InstructionKind::Assert);
        assert_eq!(assert_unchecked.direction.as_deref(), Some("unchecked"));
        assert_eq!(
            assert_unchecked.target_hint.as_deref(),
            Some("beta access checkbox")
        );
        let screenshot = analyze_instruction("Take a full page screenshot");
        assert_eq!(screenshot.kind, InstructionKind::Screenshot);
        assert_eq!(screenshot.direction.as_deref(), Some("full_page"));
        let element_screenshot = analyze_instruction("Capture a screenshot of the profile card");
        assert_eq!(element_screenshot.kind, InstructionKind::Screenshot);
        assert_eq!(
            element_screenshot.target_hint.as_deref(),
            Some("the profile card")
        );
        let right_click = analyze_instruction("Right click the Actions button");
        assert_eq!(right_click.kind, InstructionKind::Click);
        assert_eq!(right_click.value.as_deref(), Some("right_click"));
        assert_eq!(
            right_click.target_hint.as_deref(),
            Some("the Actions button")
        );
        let double_click = analyze_instruction("Double-click the Preview card");
        assert_eq!(double_click.kind, InstructionKind::Click);
        assert_eq!(double_click.value.as_deref(), Some("double_click"));
        assert_eq!(
            double_click.target_hint.as_deref(),
            Some("the Preview card")
        );
        assert_eq!(
            analyze_instruction("Focus into the textbox").kind,
            InstructionKind::Focus
        );
        assert_eq!(
            analyze_instruction("enter 'alice@example.com' into email").kind,
            InstructionKind::Fill
        );
        assert_eq!(
            analyze_instruction("choose California from the State dropdown").kind,
            InstructionKind::SelectOption
        );
        let sort_option = analyze_instruction("choose Date from Sort order");
        assert_eq!(sort_option.kind, InstructionKind::SelectOption);
        assert_eq!(sort_option.value.as_deref(), Some("Date"));
        assert_eq!(sort_option.target_hint.as_deref(), Some("Sort order"));
        assert_eq!(
            analyze_instruction("uncheck newsletter checkbox").checked,
            Some(false)
        );
        assert_eq!(
            analyze_instruction("turn on Email notifications").checked,
            Some(true)
        );
        assert_eq!(
            analyze_instruction("disable Compact mode").checked,
            Some(false)
        );
        let checked = analyze_instruction("check Red, Green, and Blue");
        assert_eq!(checked.kind, InstructionKind::SetChecked);
        assert_eq!(checked.target_hint.as_deref(), Some("Red, Green, and Blue"));
        let upload = analyze_instruction("Upload \"/tmp/resume.txt\" to the Resume field");
        assert_eq!(upload.kind, InstructionKind::UploadFile);
        assert_eq!(upload.value.as_deref(), Some("/tmp/resume.txt"));
        let press_key = analyze_instruction("Press Enter.");
        assert_eq!(press_key.kind, InstructionKind::PressKey);
        assert_eq!(press_key.value.as_deref(), Some("Enter"));
        assert_eq!(
            analyze_instruction("Press Submit.").kind,
            InstructionKind::Click
        );
        let hover = analyze_instruction("Hover over the Account menu");
        assert_eq!(hover.kind, InstructionKind::Hover);
        assert_eq!(hover.target_hint.as_deref(), Some("the Account menu"));
        let clear = analyze_instruction("Clear the search field");
        assert_eq!(clear.kind, InstructionKind::ClearField);
        assert_eq!(clear.target_hint.as_deref(), Some("the search field"));
        let append = analyze_instruction("Append beta to the Notes field");
        assert_eq!(append.kind, InstructionKind::AppendField);
        assert_eq!(append.value.as_deref(), Some("beta"));
        assert_eq!(
            analyze_instruction("drag card A to Done").kind,
            InstructionKind::Drag
        );
        assert_eq!(
            analyze_instruction(
                "Drag the smaller box so that it is completely inside the larger box."
            )
            .kind,
            InstructionKind::Drag
        );
        let directional_drag = analyze_instruction("Drag the Token right.");
        assert_eq!(directional_drag.kind, InstructionKind::Drag);
        assert_eq!(
            directional_drag.target_hint.as_deref(),
            Some("the Token right")
        );
        let line_draw = analyze_instruction("Draw a horizontal line on the drawing surface.");
        assert_eq!(line_draw.kind, InstructionKind::Drag);
        assert_eq!(line_draw.value.as_deref(), Some("line"));
        let create_line =
            analyze_instruction("Create a line that bisects the angle evenly in two.");
        assert_eq!(create_line.kind, InstructionKind::Drag);
        assert_eq!(create_line.value.as_deref(), Some("line"));
        let circle_draw =
            analyze_instruction("Draw a circle centered around the marked point, then submit.");
        assert_eq!(circle_draw.kind, InstructionKind::Drag);
        assert_eq!(circle_draw.value.as_deref(), Some("circle"));
        assert_eq!(
            analyze_instruction("scroll up").direction.as_deref(),
            Some("up")
        );
    }

    #[test]
    fn classifies_checkbox_grid_pattern_rendering_before_fill_fallback() {
        let analysis =
            analyze_instruction("Draw the number \"2\" in the checkboxes and press Submit.");
        assert_eq!(analysis.kind, InstructionKind::RenderPattern);
        assert_eq!(analysis.value.as_deref(), Some("2"));
        assert_eq!(analysis.target_hint.as_deref(), Some("checkbox grid"));
    }

    #[test]
    fn classifies_coordinate_grid_clicks_as_clicks() {
        let analysis = analyze_instruction("Click on the grid coordinate (-1,0).");
        assert_eq!(analysis.kind, InstructionKind::Click);
    }

    #[test]
    fn classifies_visual_find_clicks_as_click_plans() {
        let analysis =
            analyze_instruction("Find and click on the center of the circle, then press Submit.");
        assert_eq!(analysis.kind, InstructionKind::Click);
        assert_eq!(
            analysis.target_hint.as_deref(),
            Some("the center of the circle")
        );
    }

    #[test]
    fn extracts_follow_up_icon_click_targets() {
        let intent = build_intent(
            "Click the \"Menu\" button, and then find and click on the item with the \"ui-icon-play\" icon.",
            &analyze_instruction(
                "Click the \"Menu\" button, and then find and click on the item with the \"ui-icon-play\" icon.",
            ),
        );

        assert_eq!(intent.ordered_click_hints, vec!["Menu", "ui-icon-play"]);
        assert_eq!(intent.follow_up_click_hint.as_deref(), Some("ui-icon-play"));
    }

    #[test]
    fn quoted_text_becomes_fill_value() {
        let analysis = analyze_instruction("type \"hello world\" into the message field");
        assert_eq!(analysis.kind, InstructionKind::Fill);
        assert_eq!(analysis.value.as_deref(), Some("hello world"));
        assert_eq!(analysis.target_hint.as_deref(), Some("the message field"));
    }

    #[test]
    fn unquoted_value_target_pairs_are_split() {
        let fill = analyze_instruction("enter Alice into email");
        assert_eq!(fill.kind, InstructionKind::Fill);
        assert_eq!(fill.value.as_deref(), Some("Alice"));
        assert_eq!(fill.target_hint.as_deref(), Some("email"));

        let fill_target_with_value = analyze_instruction("Fill Public contact with Ada Lovelace");
        assert_eq!(fill_target_with_value.kind, InstructionKind::Fill);
        assert_eq!(
            fill_target_with_value.value.as_deref(),
            Some("Ada Lovelace")
        );
        assert_eq!(
            fill_target_with_value.target_hint.as_deref(),
            Some("Public contact")
        );

        let scoped_set =
            analyze_instruction("Set the Quantity field in the row containing Alice to 3.");
        assert_eq!(scoped_set.kind, InstructionKind::Fill);
        assert_eq!(scoped_set.value.as_deref(), Some("3"));
        assert_eq!(
            scoped_set.target_hint.as_deref(),
            Some("the Quantity field in the row containing Alice")
        );

        let plain_numeric_set = analyze_instruction("Set the Rating to 6 and click Apply.");
        assert_eq!(plain_numeric_set.kind, InstructionKind::Fill);
        assert_eq!(plain_numeric_set.value.as_deref(), Some("6"));
        assert_eq!(plain_numeric_set.target_hint.as_deref(), Some("the Rating"));

        let select = analyze_instruction("choose California from the State dropdown");
        assert_eq!(select.kind, InstructionKind::SelectOption);
        assert_eq!(select.value.as_deref(), Some("California"));
        assert_eq!(select.target_hint.as_deref(), Some("the State dropdown"));

        let shorthand =
            analyze_instruction("In the Support panel, Plan Enterprise, Routing Manual.");
        assert_eq!(shorthand.kind, InstructionKind::Fill);
        assert_eq!(
            shorthand.target_hint.as_deref(),
            Some("scoped multi-action")
        );

        let key_value_shorthand =
            analyze_instruction("In the Support panel, Plan: Enterprise; Routing=Manual.");
        assert_eq!(key_value_shorthand.kind, InstructionKind::Fill);
        assert_eq!(
            key_value_shorthand.target_hint.as_deref(),
            Some("scoped multi-action")
        );

        let record_first = analyze_instruction("For Alice, Quantity 4, Status Approved, Save.");
        assert_eq!(record_first.kind, InstructionKind::Fill);
        assert_eq!(
            record_first.target_hint.as_deref(),
            Some("scoped multi-action")
        );

        let row_first = analyze_instruction("Bob row: Quantity: 5; Status=Approved; Save.");
        assert_eq!(row_first.kind, InstructionKind::Fill);
        assert_eq!(
            row_first.target_hint.as_deref(),
            Some("scoped multi-action")
        );

        let leading_section =
            analyze_instruction("Support: Title Escalated; Status Approved; Save.");
        assert_eq!(leading_section.kind, InstructionKind::Fill);
        assert_eq!(
            leading_section.target_hint.as_deref(),
            Some("scoped multi-action")
        );

        let bulleted_section =
            analyze_instruction("Support:\n- Title: Escalated\n- Status: Approved\n- Save");
        assert_eq!(bulleted_section.kind, InstructionKind::Fill);
        assert_eq!(
            bulleted_section.target_hint.as_deref(),
            Some("scoped multi-action")
        );

        let single_field_then_completion =
            analyze_instruction("Support: Resume document: /tmp/support.pdf; Save.");
        assert_eq!(single_field_then_completion.kind, InstructionKind::Fill);
        assert_eq!(
            single_field_then_completion.target_hint.as_deref(),
            Some("scoped multi-action")
        );
    }

    #[test]
    fn value_controls_are_not_treated_as_dropdown_options() {
        let slider = analyze_instruction("Select -94 with the slider and hit Submit");
        assert_eq!(slider.kind, InstructionKind::Fill);
        assert_eq!(slider.value.as_deref(), Some("-94"));
        assert_eq!(slider.target_hint.as_deref(), Some("the slider"));

        let spinner = analyze_instruction("Use the spinner to select -6");
        assert_eq!(spinner.kind, InstructionKind::Fill);
        assert_eq!(spinner.value.as_deref(), Some("-6"));
        assert_eq!(spinner.target_hint.as_deref(), Some("the spinner"));

        let date = analyze_instruction("set date to 2026-06-04");
        assert_eq!(date.kind, InstructionKind::Fill);
        assert_eq!(date.value.as_deref(), Some("2026-06-04"));
        assert_eq!(date.target_hint.as_deref(), Some("date"));

        let time = analyze_instruction("set time to 3:15 pm");
        assert_eq!(time.kind, InstructionKind::Fill);
        assert_eq!(time.value.as_deref(), Some("3:15 pm"));
        assert_eq!(time.target_hint.as_deref(), Some("time"));

        let color = analyze_instruction("set color to blue");
        assert_eq!(color.kind, InstructionKind::Fill);
        assert_eq!(color.value.as_deref(), Some("blue"));
        assert_eq!(color.target_hint.as_deref(), Some("color"));

        let ordered_click = analyze_instruction("Click on the numbers in ascending order.");
        assert_eq!(ordered_click.kind, InstructionKind::Click);
        assert!(
            build_intent("Click on the numbers in ascending order.", &ordered_click)
                .wants_ordered_values
        );

        let colored_box = analyze_instruction("Click on the olive colored box.");
        assert_eq!(colored_box.kind, InstructionKind::Click);
        assert_eq!(
            colored_box.target_hint.as_deref(),
            Some("the olive colored box")
        );
    }

    #[test]
    fn sequence_helpers_report_step_count_and_confidence() {
        let plan = json!({
            "action": "sequence",
            "confidence": 0.42,
            "steps": [
                {"action": "type", "confidence": 0.7},
                {"action": "click", "confidence": 0.9}
            ]
        });
        assert_eq!(plan_step_count(&plan), 2);
        assert_eq!(plan_confidence(&plan), 0.42);
    }

    #[test]
    fn non_sequence_plan_counts_as_one_step() {
        let plan = json!({
            "action": "click",
            "confidence": 0.8,
            "params": {"selector": "#submit"}
        });
        assert_eq!(plan_step_count(&plan), 1);
        assert_eq!(plan_confidence(&plan), 0.8);
    }

    #[test]
    fn intent_extracts_ordered_click_clauses() {
        let analysis = analyze_instruction("Click button ONE, then click button TWO.");
        let intent = build_intent("Click button ONE, then click button TWO.", &analysis);
        assert_eq!(intent.action_verbs, vec!["click"]);
        assert_eq!(intent.ordered_click_hints, vec!["ONE", "TWO"]);
        assert_eq!(intent.follow_up_click_hint.as_deref(), Some("TWO"));
        assert!(!intent.wants_ordered_values);
    }

    #[test]
    fn intent_extracts_ordered_numeric_targets() {
        let analysis = analyze_instruction("Click on the numbers in ascending order.");
        let intent = build_intent("Click on the numbers in ascending order.", &analysis);
        assert_eq!(intent.order.as_deref(), Some("ascending"));
        assert_eq!(
            intent.ordered_click_hints,
            vec!["numbers in ascending order"]
        );
        assert!(intent.wants_numeric_targets);
        assert!(intent.wants_ordered_values);
        assert_eq!(
            intent
                .to_json()
                .get("order")
                .and_then(|value| value.as_str()),
            Some("ascending")
        );
    }

    #[test]
    fn close_by_clicking_is_click_intent() {
        let analysis = analyze_instruction("Close the dialog box by clicking the \"x\".");
        assert_eq!(analysis.kind, InstructionKind::Click);
        assert_eq!(analysis.target_hint.as_deref(), Some("x"));

        let intent = build_intent("Close the dialog box by clicking the \"x\".", &analysis);
        assert_eq!(intent.action_verbs, vec!["click"]);
        assert_eq!(intent.ordered_click_hints, vec!["x"]);
    }

    #[test]
    fn click_link_hint_ignores_control_words() {
        let analysis = analyze_instruction("Click on the link \"maecenas\".");
        assert_eq!(analysis.kind, InstructionKind::Click);
        assert_eq!(analysis.target_hint.as_deref(), Some("maecenas"));

        let intent = build_intent("Click on the link \"maecenas\".", &analysis);
        assert_eq!(intent.ordered_click_hints, vec!["maecenas"]);
    }

    #[test]
    fn intent_extracts_hierarchical_menu_path() {
        let analysis = analyze_instruction("Select Alice>Bob>Carol");
        assert_eq!(analysis.kind, InstructionKind::SelectOption);
        assert_eq!(analysis.value.as_deref(), Some("Alice>Bob>Carol"));

        let intent = build_intent("Select Alice>Bob>Carol", &analysis);
        assert_eq!(intent.menu_path, vec!["Alice", "Bob", "Carol"]);
        assert_eq!(
            intent
                .to_json()
                .get("menuPath")
                .and_then(|value| value.as_array())
                .map(|items| items.len()),
            Some(3)
        );
    }
}
