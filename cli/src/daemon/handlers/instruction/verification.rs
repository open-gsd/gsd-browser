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

    let strong = signals.iter().any(|signal| {
        matches!(
            signal.get("kind").and_then(|value| value.as_str()),
            Some("url_changed")
                | Some("title_changed")
                | Some("body_text_changed")
                | Some("interactive_count_changed")
                | Some("requested_value_visible_or_set")
                | Some("requested_option_visible_or_set")
                | Some("requested_checked_state_present")
        )
    });
    let status = if strong {
        "observed"
    } else if signals.is_empty() {
        "not_checked"
    } else {
        "executed_unverified"
    };

    json!({
        "status": status,
        "instruction": instruction,
        "expectedKind": analysis.kind.as_str(),
        "signals": signals,
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
    }
}
