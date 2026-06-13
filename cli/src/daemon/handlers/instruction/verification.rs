use super::model::{InstructionAnalysis, InstructionKind};
use serde_json::{json, Value};

pub(super) fn verify_action_effect(
    instruction: &str,
    analysis: &InstructionAnalysis,
    plan: &Value,
    before_model: &Value,
    after_model: &Value,
    dispatch_result: &Value,
) -> Value {
    let mut signals = Vec::new();
    let before_summary = before_model.get("summary").unwrap_or(&Value::Null);
    let after_summary = after_model.get("summary").unwrap_or(&Value::Null);

    if let Some(capability) = plan.get("capability") {
        if capability
            .get("name")
            .and_then(|value| value.as_str())
            .is_some()
        {
            signals.push(json!({
                "kind": "planned_capability",
                "name": capability.get("name").and_then(|value| value.as_str()),
                "category": capability.get("category").and_then(|value| value.as_str()),
                "expectedEffect": capability.get("expectedEffect").and_then(|value| value.as_str()),
                "strategy": capability.get("strategy").and_then(|value| value.as_str()),
            }));
        }
    }

    if string_field_changed(before_summary, after_summary, "url") {
        signals.push(json!({"kind": "url_changed"}));
    }
    if string_field_changed(before_summary, after_summary, "title") {
        signals.push(json!({"kind": "title_changed"}));
    }
    if number_field_changed(before_summary, after_summary, "bodyTextLength") {
        signals.push(json!({"kind": "body_text_changed"}));
    }
    if number_field_changed(before_summary, after_summary, "interactiveElements") {
        signals.push(json!({"kind": "interactive_count_changed"}));
    }
    if matches!(
        plan.get("action").and_then(|value| value.as_str()),
        Some("navigate") | Some("back") | Some("forward") | Some("reload")
    ) && dispatch_result
        .get("url")
        .and_then(|value| value.as_str())
        .is_some()
    {
        signals.push(json!({
            "kind": "navigation_completed",
            "action": plan.get("action").and_then(|value| value.as_str()),
            "url": dispatch_result.get("url").and_then(|value| value.as_str()),
            "title": dispatch_result.get("title").and_then(|value| value.as_str()),
        }));
    }
    if plan.get("action").and_then(|value| value.as_str()) == Some("sequence") {
        let step_count = plan
            .get("steps")
            .and_then(|value| value.as_array())
            .map(|steps| steps.len())
            .unwrap_or(0);
        let result_count = dispatch_result
            .get("steps")
            .and_then(|value| value.as_array())
            .map(|steps| steps.len())
            .unwrap_or(0);
        signals.push(json!({
            "kind": "sequence_executed",
            "plannedSteps": step_count,
            "resultSteps": result_count,
            "complete": step_count == result_count,
        }));
    } else {
        signals.push(json!({
            "kind": "action_executed",
            "action": plan.get("action").and_then(|value| value.as_str()).unwrap_or("unknown"),
        }));
    }
    if let Some(value_result) = first_successful_value_result(dispatch_result) {
        signals.push(json!({
            "kind": "value_control_set",
            "controlKind": value_result.get("kind").and_then(|value| value.as_str()),
            "expected": value_result.get("expected").and_then(|value| value.as_str()),
            "actual": value_result.get("actual").and_then(|value| value.as_str()),
            "method": value_result.get("method").and_then(|value| value.as_str()),
        }));
    }
    if let Some(derived_value) = dispatch_result.get("derivedValue") {
        if derived_value
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            && derived_value
                .get("value")
                .and_then(|value| value.as_str())
                .is_some()
        {
            signals.push(json!({
                "kind": "derived_value_entered",
                "mode": derived_value.get("mode").and_then(|value| value.as_str()),
                "value": derived_value.get("value").and_then(|value| value.as_str()),
                "target": derived_value.get("target").and_then(|value| value.as_str()),
                "submitted": derived_value.get("submitted").and_then(|value| value.as_bool()),
            }));
        }
    }
    if let Some(generated_value) = dispatch_result.get("generatedValue") {
        if generated_value
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            && generated_value.get("value").is_some()
        {
            signals.push(json!({
                "kind": "generated_value_submitted",
                "mode": generated_value.get("mode").and_then(|value| value.as_str()),
                "value": generated_value.get("value").cloned().unwrap_or(Value::Null),
                "attempts": generated_value.get("attempts").and_then(|value| value.as_u64()),
                "generator": generated_value.get("generator").and_then(|value| value.as_str()),
                "output": generated_value.get("output").and_then(|value| value.as_str()),
                "target": generated_value.get("target").and_then(|value| value.as_str()),
                "submitted": generated_value.get("submitted").and_then(|value| value.as_bool()),
            }));
        }
    }
    if let Some(conditional_action) = dispatch_result.get("conditionalAction") {
        if conditional_action
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            && conditional_action.get("action").is_some()
        {
            signals.push(json!({
                "kind": "conditional_action_completed",
                "mode": conditional_action.get("mode").and_then(|value| value.as_str()),
                "value": conditional_action.get("value").cloned().unwrap_or(Value::Null),
                "source": conditional_action.get("source").and_then(|value| value.as_str()),
                "action": conditional_action.get("action").and_then(|value| value.as_str()),
                "elapsedMs": conditional_action.get("elapsedMs").and_then(|value| value.as_u64()),
            }));
        }
    }
    if let Some(command_action) = dispatch_result.get("commandSurfaceAction") {
        if command_action
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            && command_action.get("command").is_some()
        {
            signals.push(json!({
                "kind": "command_surface_completed",
                "mode": command_action.get("mode").and_then(|value| value.as_str()),
                "command": command_action.get("command").and_then(|value| value.as_str()),
                "targetItem": command_action.get("targetItem").and_then(|value| value.as_str()),
                "input": command_action.get("input").and_then(|value| value.as_str()),
                "container": command_action.get("container").and_then(|value| value.as_str()),
            }));
        }
    }
    if let Some(form) = successful_result_object(dispatch_result, "formWorkflow") {
        signals.push(json!({
            "kind": "form_workflow_completed",
            "mode": form.get("mode").and_then(|value| value.as_str()),
            "filledCount": form.get("filled").and_then(|value| value.as_array()).map(|items| items.len()),
            "submitted": form.get("submitted").map(|value| !value.is_null()),
        }));
    }
    if plan.get("action").and_then(|value| value.as_str()) == Some("date_picker")
        && dispatch_result
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        signals.push(json!({
            "kind": "calendar_date_selected",
            "mode": dispatch_result.get("mode").and_then(|value| value.as_str()),
            "date": dispatch_result.get("date").and_then(|value| value.as_str()),
        }));
    }
    if let Some(selection) = successful_result_object(dispatch_result, "selection") {
        signals.push(json!({
            "kind": "text_selected",
            "selector": selection.get("selector").and_then(|value| value.as_str()),
            "requestedText": selection.get("requestedText").and_then(|value| value.as_str()),
            "selectedLength": selection.get("selected").and_then(|value| value.as_str()).map(|text| text.len()),
            "mode": selection.get("mode").and_then(|value| value.as_str()),
        }));
    }
    if let Some(format) = successful_result_object(dispatch_result, "formatText") {
        signals.push(json!({
            "kind": "text_formatted",
            "selector": format.get("selector").and_then(|value| value.as_str()),
            "style": format.get("style").and_then(|value| value.as_str()),
            "mode": format.get("mode").and_then(|value| value.as_str()),
        }));
    }
    if let Some(slider) = successful_result_object(dispatch_result, "slider") {
        signals.push(json!({
            "kind": "slider_value_set",
            "selector": slider.get("selector").and_then(|value| value.as_str()),
            "value": slider.get("value").cloned().unwrap_or(Value::Null),
            "mode": slider.get("mode").and_then(|value| value.as_str()),
        }));
    }
    if let Some(scoped_workflow) = successful_result_object(dispatch_result, "scopedWorkflow") {
        signals.push(json!({
            "kind": "scoped_workflow_completed",
            "mode": scoped_workflow.get("mode").and_then(|value| value.as_str()),
            "action": scoped_workflow.get("action").and_then(|value| value.as_str()),
            "target": scoped_workflow.get("target").cloned().unwrap_or(Value::Null),
        }));
    }
    if let Some(ordered) = successful_result_object(dispatch_result, "orderedValues") {
        signals.push(json!({
            "kind": "ordered_targets_clicked",
            "mode": ordered.get("mode").and_then(|value| value.as_str()),
            "clickedCount": ordered.get("clicked").and_then(|value| value.as_array()).map(|items| items.len()),
        }));
    }
    if let Some(menu) = successful_result_object(dispatch_result, "menuPath") {
        signals.push(json!({
            "kind": "menu_path_selected",
            "mode": menu.get("mode").and_then(|value| value.as_str()),
            "path": menu.get("path").cloned().unwrap_or(Value::Null),
        }));
    }
    if let Some(scoped_click) = successful_result_object(dispatch_result, "scopedMenuClick") {
        signals.push(json!({
            "kind": "scoped_target_clicked",
            "mode": scoped_click.get("mode").and_then(|value| value.as_str()),
            "target": scoped_click.get("target").cloned().unwrap_or(Value::Null),
        }));
    }
    if let Some(scroll_text) = successful_result_object(dispatch_result, "scrollTextExtract") {
        signals.push(json!({
            "kind": "scroll_text_value_used",
            "mode": scroll_text.get("mode").and_then(|value| value.as_str()),
            "value": scroll_text.get("value").cloned().unwrap_or(Value::Null),
        }));
    }
    if let Some(draw_path) = dispatch_result.get("drawPath") {
        signals.push(json!({
            "kind": "pointer_path_drawn",
            "points": draw_path.get("points").and_then(|value| value.as_u64()),
            "button": draw_path.get("button").and_then(|value| value.as_str()),
        }));
    }
    if let Some(oriented) = dispatch_result.get("orientedVisual") {
        signals.push(json!({
            "kind": "visual_object_clicked",
            "targetText": oriented.get("targetText").and_then(|value| value.as_str()),
            "attempts": oriented.get("attempts").and_then(|value| value.as_u64()),
        }));
    }
    if let Some(record) = successful_result_object(dispatch_result, "recordPropertyClick") {
        signals.push(json!({
            "kind": "record_property_clicked",
            "mode": record.get("mode").and_then(|value| value.as_str()),
            "record": record.get("record").cloned().unwrap_or(Value::Null),
            "property": record.get("property").and_then(|value| value.as_str()),
        }));
    }
    if let Some(tree) = successful_result_object(dispatch_result, "treeSearchClick") {
        signals.push(json!({
            "kind": "tree_target_clicked",
            "mode": tree.get("mode").and_then(|value| value.as_str()),
            "target": tree.get("target").cloned().unwrap_or(Value::Null),
        }));
    }
    if let Some(visual) = successful_result_object(dispatch_result, "visualFeedbackSearch") {
        signals.push(json!({
            "kind": "pointer_feedback_target_clicked",
            "mode": visual.get("mode").and_then(|value| value.as_str()),
            "target": visual.get("target").cloned().unwrap_or(Value::Null),
        }));
    }
    if let Some(discover) = successful_result_object(dispatch_result, "discoverClick") {
        signals.push(json!({
            "kind": "target_revealed_or_clicked",
            "mode": discover.get("mode").and_then(|value| value.as_str()),
            "target": discover.get("target").cloned().unwrap_or(Value::Null),
        }));
    }
    if let Some(autocomplete) = first_successful_autocomplete_result(dispatch_result) {
        signals.push(json!({
            "kind": "autocomplete_option_selected",
            "selected": autocomplete.get("selected").and_then(|value| value.as_str()),
            "inputValue": autocomplete.get("inputValue").and_then(|value| value.as_str()),
            "mode": autocomplete.get("mode").and_then(|value| value.as_str()),
        }));
    }
    if let Some(selection) = first_successful_selection_result(dispatch_result) {
        signals.push(json!({
            "kind": "option_selection_set",
            "mode": selection.get("mode").and_then(|value| value.as_str()),
            "matched": selection.get("matched").cloned().unwrap_or(Value::Null),
            "actual": selection.get("actual").cloned().unwrap_or(Value::Null),
        }));
    }
    if let Some(checked) = first_successful_checked_result(dispatch_result) {
        signals.push(json!({
            "kind": "checked_state_set",
            "mode": checked.get("mode").and_then(|value| value.as_str()),
            "desired": checked.get("desired").and_then(|value| value.as_bool()),
            "actual": checked.get("actual").and_then(|value| value.as_bool()),
            "changed": checked.get("changed").and_then(|value| value.as_bool()),
        }));
    }
    if let Some(uploaded) = first_successful_upload_result(dispatch_result) {
        signals.push(json!({
            "kind": "file_uploaded",
            "selector": uploaded.get("selector").and_then(|value| value.as_str()),
            "fileCount": uploaded.get("files").and_then(|value| value.as_array()).map(|files| files.len()),
        }));
    }
    if let Some(key) = dispatch_result
        .get("pressed")
        .and_then(|value| value.as_str())
    {
        signals.push(json!({
            "kind": "key_pressed",
            "key": key,
        }));
    }
    if dispatch_result
        .get("met")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        signals.push(json!({
            "kind": "wait_condition_met",
            "condition": dispatch_result.get("condition").and_then(|value| value.as_str()),
            "value": dispatch_result.get("value").and_then(|value| value.as_str()),
            "elapsedMs": dispatch_result.get("elapsed_ms").and_then(|value| value.as_u64()),
        }));
    }
    if matches!(
        plan.get("action").and_then(|value| value.as_str()),
        Some("set_viewport")
    ) && dispatch_result
        .get("width")
        .and_then(|value| value.as_i64())
        .is_some()
        && dispatch_result
            .get("height")
            .and_then(|value| value.as_i64())
            .is_some()
    {
        signals.push(json!({
            "kind": "viewport_set",
            "width": dispatch_result.get("width").and_then(|value| value.as_i64()),
            "height": dispatch_result.get("height").and_then(|value| value.as_i64()),
            "preset": dispatch_result.get("preset").and_then(|value| value.as_str()),
        }));
    }
    if matches!(
        plan.get("action").and_then(|value| value.as_str()),
        Some("emulate_device")
    ) && dispatch_result
        .get("device")
        .and_then(|value| value.as_str())
        .is_some()
    {
        signals.push(json!({
            "kind": "device_emulated",
            "device": dispatch_result.get("device").and_then(|value| value.as_str()),
            "width": dispatch_result.get("width").and_then(|value| value.as_i64()),
            "height": dispatch_result.get("height").and_then(|value| value.as_i64()),
            "mobile": dispatch_result.get("mobile").and_then(|value| value.as_bool()),
        }));
    }
    if matches!(
        plan.get("action").and_then(|value| value.as_str()),
        Some("read_text")
    ) && dispatch_result
        .get("text")
        .and_then(|value| value.as_str())
        .filter(|text| !text.trim().is_empty())
        .is_some()
    {
        signals.push(json!({
            "kind": "text_read",
            "selector": dispatch_result.get("selector").and_then(|value| value.as_str()),
            "length": dispatch_result.get("length").and_then(|value| value.as_u64()),
            "truncated": dispatch_result.get("truncated").and_then(|value| value.as_bool()),
        }));
    }
    if matches!(
        plan.get("action").and_then(|value| value.as_str()),
        Some("analyze_form")
    ) && dispatch_result
        .get("fieldCount")
        .and_then(|value| value.as_u64())
        .is_some()
    {
        signals.push(json!({
            "kind": "form_analyzed",
            "formSelector": dispatch_result.get("formSelector").and_then(|value| value.as_str()),
            "fieldCount": dispatch_result.get("fieldCount").and_then(|value| value.as_u64()),
            "submitButtonCount": dispatch_result.get("submitButtons").and_then(|value| value.as_array()).map(|buttons| buttons.len()),
        }));
    }
    if matches!(
        plan.get("action").and_then(|value| value.as_str()),
        Some("accessibility_tree")
    ) && dispatch_result
        .get("tree")
        .and_then(|value| value.as_str())
        .filter(|tree| !tree.trim().is_empty())
        .is_some()
    {
        signals.push(json!({
            "kind": "accessibility_tree_read",
            "nodeCount": dispatch_result.get("nodeCount").and_then(|value| value.as_u64()),
            "truncated": dispatch_result.get("truncated").and_then(|value| value.as_bool()),
        }));
    }
    if matches!(
        plan.get("action").and_then(|value| value.as_str()),
        Some("find")
    ) && dispatch_result
        .get("count")
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
        > 0
    {
        signals.push(json!({
            "kind": "elements_found",
            "count": dispatch_result.get("count").and_then(|value| value.as_u64()),
            "truncated": dispatch_result.get("truncated").and_then(|value| value.as_bool()),
        }));
    }
    if dispatch_result
        .get("verified")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        signals.push(json!({
            "kind": "assertion_verified",
            "summary": dispatch_result.get("summary").and_then(|value| value.as_str()),
            "checks": dispatch_result.get("checks").cloned().unwrap_or(Value::Null),
        }));
    }
    if dispatch_result
        .get("data")
        .and_then(|value| value.as_str())
        .is_some()
        && dispatch_result
            .get("byteLength")
            .and_then(|value| value.as_u64())
            .unwrap_or(0)
            > 0
    {
        signals.push(json!({
            "kind": "screenshot_captured",
            "scope": dispatch_result.get("scope").and_then(|value| value.as_str()),
            "mimeType": dispatch_result.get("mimeType").and_then(|value| value.as_str()),
            "width": dispatch_result.get("width").and_then(|value| value.as_u64()),
            "height": dispatch_result.get("height").and_then(|value| value.as_u64()),
            "byteLength": dispatch_result.get("byteLength").and_then(|value| value.as_u64()),
        }));
    }
    if dispatch_result.get("hovered").is_some() {
        signals.push(json!({
            "kind": "element_hovered",
            "selector": dispatch_result
                .get("hovered")
                .and_then(|hovered| hovered.get("selector"))
                .and_then(|value| value.as_str()),
        }));
    }
    if let Some(grid) = first_successful_checkbox_grid_result(dispatch_result) {
        signals.push(json!({
            "kind": "checkbox_grid_set",
            "mode": grid.get("mode").and_then(|value| value.as_str()),
            "target": grid.get("target").and_then(|value| value.as_str()),
            "rows": grid.get("rows").and_then(|value| value.as_i64()),
            "cols": grid.get("cols").and_then(|value| value.as_i64()),
            "checkedCount": grid.get("checkedCount").and_then(|value| value.as_i64()),
        }));
    }
    if plan.get("action").and_then(|value| value.as_str()) == Some("focus") {
        if let Some(focus) = dispatch_result
            .get("state")
            .and_then(|state| state.get("focus"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
        {
            signals.push(json!({"kind": "focus_set", "focus": focus}));
        }
    }

    match analysis.kind {
        InstructionKind::Fill => {
            if let Some(value) = &analysis.value {
                if model_contains_value(after_model, value) {
                    signals.push(json!({"kind": "requested_value_visible_or_set", "value": value}));
                }
            }
        }
        InstructionKind::SetChecked => {
            if let Some(checked) = analysis.checked {
                if model_contains_checked(after_model, checked) {
                    signals.push(
                        json!({"kind": "requested_checked_state_present", "checked": checked}),
                    );
                }
            }
        }
        InstructionKind::SelectOption => {
            if let Some(value) = &analysis.value {
                if model_contains_value(after_model, value) {
                    signals
                        .push(json!({"kind": "requested_option_visible_or_set", "value": value}));
                }
            }
        }
        _ => {}
    }

    let strong = signals.iter().any(signal_is_strong);
    let effect = capability_effect_summary(plan, &signals).unwrap_or(Value::Null);
    let effect_observed = effect.get("observed").and_then(|value| value.as_bool());
    let effect_checkable = effect
        .get("checkable")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let effect_status = match effect_observed {
        Some(true) => "observed",
        Some(false) => "not_observed",
        None => "not_applicable",
    };
    let verified = if effect_checkable {
        effect_observed.unwrap_or(false)
    } else if effect.is_object() {
        false
    } else {
        strong
    };
    let status = match (effect_checkable, effect_observed) {
        (true, Some(true)) => "observed",
        (true, Some(false)) => "not_observed",
        _ if verified => "observed",
        _ if signals.is_empty() => "not_checked",
        _ => "executed_unverified",
    };

    json!({
        "status": status,
        "verified": verified,
        "effectStatus": effect_status,
        "instruction": instruction,
        "expectedKind": analysis.kind.as_str(),
        "effect": effect,
        "signals": signals,
    })
}

fn capability_effect_summary(plan: &Value, signals: &[Value]) -> Option<Value> {
    let capability = plan.get("capability")?;
    let expected_effect = capability
        .get("expectedEffect")
        .and_then(|value| value.as_str())?;
    let matching_signal_kinds = effect_signal_kinds(expected_effect);
    if matching_signal_kinds.is_empty() {
        return Some(json!({
            "expectedEffect": expected_effect,
            "capability": capability.get("name").and_then(|value| value.as_str()),
            "category": capability.get("category").and_then(|value| value.as_str()),
            "checkable": false,
            "observed": Value::Null,
            "matchingSignals": [],
        }));
    }
    let matched_signals = signals
        .iter()
        .filter_map(|signal| signal.get("kind").and_then(|value| value.as_str()))
        .filter(|kind| {
            matching_signal_kinds
                .iter()
                .any(|expected| expected == kind)
        })
        .collect::<Vec<_>>();
    let observed = signals.iter().any(|signal| {
        signal
            .get("kind")
            .and_then(|value| value.as_str())
            .map(is_strong_signal_kind)
            .unwrap_or(false)
            && signal
                .get("kind")
                .and_then(|value| value.as_str())
                .map(|kind| {
                    matching_signal_kinds
                        .iter()
                        .any(|expected| expected == &kind)
                })
                .unwrap_or(false)
    });

    Some(json!({
        "expectedEffect": expected_effect,
        "capability": capability.get("name").and_then(|value| value.as_str()),
        "category": capability.get("category").and_then(|value| value.as_str()),
        "checkable": true,
        "observed": observed,
        "matchingSignals": matched_signals,
    }))
}

fn effect_signal_kinds(expected_effect: &str) -> &'static [&'static str] {
    match expected_effect {
        "form_fields_filled" => &[
            "form_workflow_completed",
            "value_control_set",
            "option_selection_set",
            "requested_value_visible_or_set",
            "requested_option_visible_or_set",
        ],
        "form_workflow_completed" | "compound_form_sequence_completed" => &[
            "form_workflow_completed",
            "value_control_set",
            "option_selection_set",
            "checked_state_set",
            "requested_value_visible_or_set",
            "requested_option_visible_or_set",
            "body_text_changed",
            "interactive_count_changed",
        ],
        "table_values_filled" => &[
            "value_control_set",
            "requested_value_visible_or_set",
            "body_text_changed",
        ],
        "scoped_field_filled" => &[
            "value_control_set",
            "option_selection_set",
            "requested_value_visible_or_set",
            "requested_option_visible_or_set",
        ],
        "scoped_field_edited" => &["value_control_set", "requested_value_visible_or_set"],
        "file_uploaded" => &["file_uploaded"],
        "slider_value_set" | "slider_values_set" | "numeric_value_set" => {
            &["slider_value_set", "value_control_set"]
        }
        "autocomplete_option_selected" => &["autocomplete_option_selected"],
        "slider_and_checkbox_values_set" => &["value_control_set", "checked_state_set"],
        "ordinal_checked_control_set" => &["checked_state_set"],
        "scoped_checked_control_set" => &["checked_state_set"],
        "grouped_choice_control_set" => &["checked_state_set", "option_selection_set"],
        "list_options_selected" | "menu_path_selected" => &[
            "menu_path_selected",
            "option_selection_set",
            "body_text_changed",
            "interactive_count_changed",
        ],
        "checkbox_pattern_rendered" => &["checkbox_grid_set", "checked_state_set"],
        "color_picker_value_set" => &[
            "value_control_set",
            "checked_state_set",
            "requested_value_visible_or_set",
            "body_text_changed",
        ],
        "visual_color_targets_selected" => &[
            "checked_state_set",
            "body_text_changed",
            "interactive_count_changed",
        ],
        "text_copied_to_target" => &[
            "value_control_set",
            "requested_value_visible_or_set",
            "body_text_changed",
        ],
        "item_action_completed"
        | "item_quantities_selected"
        | "pointer_feedback_target_clicked"
        | "record_property_clicked"
        | "row_classification_completed"
        | "tree_target_clicked"
        | "target_revealed_or_clicked"
        | "visual_object_clicked"
        | "ranked_target_clicked"
        | "coordinate_target_clicked"
        | "scoped_target_clicked"
        | "ordinal_target_clicked"
        | "ordered_targets_clicked"
        | "ordered_numeric_targets_clicked"
        | "scroll_fill_and_submit_completed" => &[
            "scoped_workflow_completed",
            "ordered_targets_clicked",
            "scoped_target_clicked",
            "record_property_clicked",
            "tree_target_clicked",
            "target_revealed_or_clicked",
            "pointer_feedback_target_clicked",
            "visual_object_clicked",
            "body_text_changed",
            "interactive_count_changed",
            "value_control_set",
            "checked_state_set",
            "requested_value_visible_or_set",
        ],
        "text_selected" => &["text_selected"],
        "element_resized" => &["pointer_path_drawn", "body_text_changed"],
        "calendar_date_selected" => &[
            "calendar_date_selected",
            "value_control_set",
            "requested_value_visible_or_set",
            "body_text_changed",
        ],
        "scoped_multi_action_completed" => &[
            "scoped_workflow_completed",
            "scoped_target_clicked",
            "form_workflow_completed",
            "value_control_set",
            "option_selection_set",
            "checked_state_set",
            "requested_value_visible_or_set",
            "requested_option_visible_or_set",
            "requested_checked_state_present",
            "body_text_changed",
            "interactive_count_changed",
        ],
        "visible_text_entered" => &["value_control_set", "requested_value_visible_or_set"],
        "table_cell_value_used" => &[
            "value_control_set",
            "requested_value_visible_or_set",
            "body_text_changed",
        ],
        "visible_count_value_used" => &["value_control_set", "requested_value_visible_or_set"],
        "derived_value_or_action_completed" => &[
            "derived_value_entered",
            "value_control_set",
            "body_text_changed",
            "interactive_count_changed",
        ],
        "generated_value_submitted" => &[
            "generated_value_submitted",
            "value_control_set",
            "body_text_changed",
            "interactive_count_changed",
        ],
        "conditional_action_completed" => &[
            "conditional_action_completed",
            "body_text_changed",
            "interactive_count_changed",
        ],
        "command_surface_completed" => &["command_surface_completed", "body_text_changed"],
        "visual_geometry_value_selected" => &["body_text_changed", "interactive_count_changed"],
        "scroll_text_value_used" => &[
            "scroll_text_value_used",
            "value_control_set",
            "requested_value_visible_or_set",
        ],
        _ => &[],
    }
}

fn signal_is_strong(signal: &Value) -> bool {
    signal
        .get("kind")
        .and_then(|value| value.as_str())
        .map(is_strong_signal_kind)
        .unwrap_or(false)
}

fn is_strong_signal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "url_changed"
            | "title_changed"
            | "body_text_changed"
            | "interactive_count_changed"
            | "navigation_completed"
            | "value_control_set"
            | "autocomplete_option_selected"
            | "option_selection_set"
            | "checked_state_set"
            | "file_uploaded"
            | "key_pressed"
            | "wait_condition_met"
            | "viewport_set"
            | "device_emulated"
            | "text_read"
            | "form_analyzed"
            | "accessibility_tree_read"
            | "elements_found"
            | "assertion_verified"
            | "screenshot_captured"
            | "element_hovered"
            | "checkbox_grid_set"
            | "focus_set"
            | "form_workflow_completed"
            | "calendar_date_selected"
            | "text_selected"
            | "text_formatted"
            | "slider_value_set"
            | "scoped_workflow_completed"
            | "ordered_targets_clicked"
            | "menu_path_selected"
            | "scoped_target_clicked"
            | "scroll_text_value_used"
            | "pointer_path_drawn"
            | "visual_object_clicked"
            | "record_property_clicked"
            | "tree_target_clicked"
            | "pointer_feedback_target_clicked"
            | "target_revealed_or_clicked"
            | "derived_value_entered"
            | "generated_value_submitted"
            | "conditional_action_completed"
            | "command_surface_completed"
            | "requested_value_visible_or_set"
            | "requested_option_visible_or_set"
            | "requested_checked_state_present"
    )
}

fn successful_result_object<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
    if let Some(candidate) = value.get(field) {
        if candidate
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    value
        .get("steps")
        .and_then(|value| value.as_array())
        .and_then(|steps| {
            steps
                .iter()
                .find_map(|step| successful_result_object(step, field))
        })
}

fn string_field_changed(before: &Value, after: &Value, field: &str) -> bool {
    let before = before.get(field).and_then(|value| value.as_str());
    let after = after.get(field).and_then(|value| value.as_str());
    before != after
}

fn number_field_changed(before: &Value, after: &Value, field: &str) -> bool {
    let before = before.get(field).and_then(|value| value.as_i64());
    let after = after.get(field).and_then(|value| value.as_i64());
    before != after
}

fn model_contains_value(model: &Value, expected: &str) -> bool {
    let expected = expected.to_lowercase();
    model
        .get("elements")
        .and_then(|value| value.as_array())
        .map(|elements| {
            elements.iter().any(|element| {
                ["value", "text", "normalizedText"].iter().any(|field| {
                    element
                        .get(field)
                        .and_then(|value| value.as_str())
                        .map(|text| text.to_lowercase().contains(&expected))
                        .unwrap_or(false)
                })
            })
        })
        .unwrap_or(false)
}

fn model_contains_checked(model: &Value, expected: bool) -> bool {
    model
        .get("elements")
        .and_then(|value| value.as_array())
        .map(|elements| {
            elements.iter().any(|element| {
                element
                    .get("checked")
                    .map(|value| match value {
                        Value::Bool(value) => *value == expected,
                        Value::String(value) => value == &expected.to_string(),
                        _ => false,
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn first_successful_value_result(value: &Value) -> Option<&Value> {
    if value.get("ok").and_then(|ok| ok.as_bool()).unwrap_or(false)
        && value.get("actual").is_some()
        && value.get("expected").is_some()
    {
        return Some(value);
    }
    if let Some(candidate) = value.get("valueResult") {
        if candidate
            .get("ok")
            .and_then(|ok| ok.as_bool())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    if let Some(candidate) = value.get("typed").and_then(|typed| typed.get("fill")) {
        if candidate
            .get("ok")
            .and_then(|ok| ok.as_bool())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    if let Some(candidate) = value
        .get("typed")
        .and_then(|typed| typed.get("valueResult"))
    {
        if candidate
            .get("ok")
            .and_then(|ok| ok.as_bool())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    if let Some(steps) = value.get("steps").and_then(|steps| steps.as_array()) {
        for step in steps {
            if let Some(candidate) = first_successful_value_result(step) {
                return Some(candidate);
            }
        }
    }
    None
}

fn first_successful_autocomplete_result(value: &Value) -> Option<&Value> {
    if let Some(candidate) = value.get("autocomplete") {
        if candidate
            .get("ok")
            .and_then(|ok| ok.as_bool())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    if let Some(steps) = value.get("steps").and_then(|steps| steps.as_array()) {
        for step in steps {
            if let Some(candidate) = first_successful_autocomplete_result(step) {
                return Some(candidate);
            }
        }
    }
    None
}

fn first_successful_selection_result(value: &Value) -> Option<&Value> {
    if let Some(selection) = value.get("selected") {
        if selection.get("mode").is_some() || selection.get("actual").is_some() {
            return Some(selection);
        }
    }
    if let Some(steps) = value.get("steps").and_then(|steps| steps.as_array()) {
        for step in steps {
            if let Some(candidate) = first_successful_selection_result(step) {
                return Some(candidate);
            }
        }
    }
    None
}

fn first_successful_checked_result(value: &Value) -> Option<&Value> {
    if let Some(checked) = value
        .get("checked")
        .and_then(|checked| checked.get("result"))
    {
        if checked
            .get("ok")
            .and_then(|ok| ok.as_bool())
            .unwrap_or(false)
        {
            return Some(checked);
        }
    }
    if let Some(candidate) = value.get("checkedResult") {
        if candidate
            .get("ok")
            .and_then(|ok| ok.as_bool())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    if let Some(steps) = value.get("steps").and_then(|steps| steps.as_array()) {
        for step in steps {
            if let Some(candidate) = first_successful_checked_result(step) {
                return Some(candidate);
            }
        }
    }
    None
}

fn first_successful_upload_result(value: &Value) -> Option<&Value> {
    if let Some(uploaded) = value.get("uploaded") {
        if uploaded
            .get("files")
            .and_then(|files| files.as_array())
            .map(|files| !files.is_empty())
            .unwrap_or(false)
        {
            return Some(uploaded);
        }
    }
    if let Some(steps) = value.get("steps").and_then(|steps| steps.as_array()) {
        for step in steps {
            if let Some(candidate) = first_successful_upload_result(step) {
                return Some(candidate);
            }
        }
    }
    None
}

fn first_successful_checkbox_grid_result(value: &Value) -> Option<&Value> {
    if let Some(grid) = value.get("checkboxGrid") {
        if grid.get("ok").and_then(|ok| ok.as_bool()).unwrap_or(false) {
            return Some(grid);
        }
    }
    if let Some(steps) = value.get("steps").and_then(|steps| steps.as_array()) {
        for step in steps {
            if let Some(candidate) = first_successful_checkbox_grid_result(step) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::handlers::instruction::model::{InstructionAnalysis, InstructionKind};

    #[test]
    fn verification_observes_requested_fill_value() {
        let analysis = InstructionAnalysis {
            kind: InstructionKind::Fill,
            value: Some("Alice".to_string()),
            target_hint: Some("name".to_string()),
            secondary_hint: None,
            checked: None,
            direction: None,
        };
        let before = json!({"summary": {"url": "https://example.test", "bodyTextLength": 10}, "elements": []});
        let after = json!({
            "summary": {"url": "https://example.test", "bodyTextLength": 10},
            "elements": [{"value": "Alice", "text": "", "normalizedText": "alice"}],
        });
        let verification = verify_action_effect(
            "enter Alice into name",
            &analysis,
            &json!({"action": "type"}),
            &before,
            &after,
            &json!({}),
        );
        assert_eq!(verification["status"], "observed");
        assert_eq!(verification["verified"], true);
        assert_eq!(verification["effectStatus"], "not_applicable");
    }

    #[test]
    fn verification_observes_typed_value_control_result() {
        let analysis = InstructionAnalysis {
            kind: InstructionKind::Fill,
            value: Some("blue".to_string()),
            target_hint: Some("color".to_string()),
            secondary_hint: None,
            checked: None,
            direction: None,
        };
        let before =
            json!({"summary": {"url": "about:blank", "bodyTextLength": 0}, "elements": []});
        let after = json!({
            "summary": {"url": "about:blank", "bodyTextLength": 0},
            "elements": [{"value": "#0000ff", "text": "", "normalizedText": ""}],
        });
        let verification = verify_action_effect(
            "set color to blue",
            &analysis,
            &json!({"action": "type"}),
            &before,
            &after,
            &json!({
                "typed": {
                    "valueResult": {
                        "ok": true,
                        "kind": "color",
                        "expected": "#0000ff",
                        "actual": "#0000ff",
                        "method": "typed-direct"
                    }
                }
            }),
        );
        assert_eq!(verification["status"], "observed");
        assert_eq!(verification["verified"], true);
        assert_eq!(verification["effectStatus"], "not_applicable");
        assert!(verification["signals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|signal| signal["kind"] == "value_control_set"));
    }

    #[test]
    fn verification_distinguishes_strong_change_from_expected_capability_effect() {
        let analysis = InstructionAnalysis {
            kind: InstructionKind::Fill,
            value: Some("Alice".to_string()),
            target_hint: Some("name".to_string()),
            secondary_hint: None,
            checked: None,
            direction: None,
        };
        let before =
            json!({"summary": {"url": "about:blank", "bodyTextLength": 10}, "elements": []});
        let after =
            json!({"summary": {"url": "about:blank", "bodyTextLength": 12}, "elements": []});
        let verification = verify_action_effect(
            "enter Alice into name",
            &analysis,
            &json!({
                "action": "type",
                "capability": {
                    "name": "generic-form-fill",
                    "expectedEffect": "form_fields_filled",
                    "category": "form_control"
                }
            }),
            &before,
            &after,
            &json!({}),
        );

        assert_eq!(verification["status"], "not_observed");
        assert_eq!(verification["verified"], false);
        assert_eq!(verification["effectStatus"], "not_observed");
        assert_eq!(verification["effect"]["checkable"], true);
        assert_eq!(verification["effect"]["observed"], false);
    }

    #[test]
    fn verification_does_not_overclaim_unknown_capability_effects() {
        let analysis = InstructionAnalysis {
            kind: InstructionKind::Click,
            value: None,
            target_hint: Some("action".to_string()),
            secondary_hint: None,
            checked: None,
            direction: None,
        };
        let before =
            json!({"summary": {"url": "about:blank", "bodyTextLength": 10}, "elements": []});
        let after =
            json!({"summary": {"url": "about:blank", "bodyTextLength": 12}, "elements": []});
        let verification = verify_action_effect(
            "click the action control",
            &analysis,
            &json!({
                "action": "click",
                "capability": {
                    "name": "future-generic-capability",
                    "expectedEffect": "future_effect_not_yet_mapped",
                    "category": "generic"
                }
            }),
            &before,
            &after,
            &json!({}),
        );

        assert_eq!(verification["status"], "executed_unverified");
        assert_eq!(verification["verified"], false);
        assert_eq!(verification["effectStatus"], "not_applicable");
        assert_eq!(verification["effect"]["checkable"], false);
        assert_eq!(verification["effect"]["observed"], Value::Null);
    }
}
