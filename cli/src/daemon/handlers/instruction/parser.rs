use super::model::{InstructionAnalysis, InstructionIntent, InstructionKind};

pub(super) fn analyze_instruction(instruction: &str) -> InstructionAnalysis {
    let lower = instruction.to_lowercase();
    let quoted = quoted_strings(instruction);
    let mut value = quoted.first().cloned();
    let mut target_hint = None;
    let mut secondary_hint = quoted.get(1).cloned();
    let mut checked = None;
    let mut direction = None;

    let kind = if starts_with_any(&lower, &["how many ", "count "]) || lower.contains(" how many ")
    {
        target_hint = trailing_hint(instruction, &["how many", "count"]);
        InstructionKind::Count
    } else if is_screenshot_instruction(&lower) {
        direction = if contains_any(
            &lower,
            &["full page", "full-page", "entire page", "whole page"],
        ) {
            Some("full_page".to_string())
        } else {
            Some("viewport".to_string())
        };
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| screenshot_target_hint(instruction));
        InstructionKind::Screenshot
    } else if is_assert_instruction(&lower) {
        if contains_any(&lower, &["no console errors", "without console errors"]) {
            direction = Some("no_console_errors".to_string());
        } else if contains_any(&lower, &["no failed requests", "without failed requests"]) {
            direction = Some("no_failed_requests".to_string());
        } else if contains_any(
            &lower,
            &["url contains", "url should contain", "url includes"],
        ) {
            direction = Some("url_contains".to_string());
            value = quoted.first().cloned().or_else(|| {
                text_after_markers(
                    instruction,
                    &["url contains", "url should contain", "url includes"],
                )
            });
        } else if contains_any(
            &lower,
            &["title contains", "title should contain", "title includes"],
        ) {
            direction = Some("title_contains".to_string());
            value = quoted.first().cloned().or_else(|| {
                text_after_markers(
                    instruction,
                    &["title contains", "title should contain", "title includes"],
                )
            });
        } else if let Some((assert_value, assert_target)) = assert_value_target(instruction) {
            direction = Some("value_equals".to_string());
            value = Some(assert_value);
            target_hint = Some(assert_target);
        } else if contains_any(
            &lower,
            &[
                " not checked",
                " unchecked",
                " unselected",
                " is off",
                " off",
            ],
        ) {
            direction = Some("unchecked".to_string());
            value = quoted
                .first()
                .cloned()
                .or_else(|| assert_target_hint(instruction));
            target_hint = value.clone();
        } else if contains_any(
            &lower,
            &[" checked", " selected", " is on", " enabled", " on"],
        ) {
            direction = Some("checked".to_string());
            value = quoted
                .first()
                .cloned()
                .or_else(|| assert_target_hint(instruction));
            target_hint = value.clone();
        } else if contains_any(
            &lower,
            &[
                " not visible",
                " hidden",
                " not shown",
                " absent",
                " not present",
                " disappears",
                " disappeared",
            ],
        ) {
            direction = Some("hidden".to_string());
            value = quoted
                .first()
                .cloned()
                .or_else(|| assert_target_hint(instruction));
            target_hint = value.clone();
        } else {
            direction = Some("visible".to_string());
            value = quoted
                .first()
                .cloned()
                .or_else(|| assert_target_hint(instruction));
            target_hint = value.clone();
        }
        InstructionKind::Assert
    } else if starts_with_any(
        &lower,
        &[
            "go back",
            "navigate back",
            "back to ",
            "back one page",
            "previous page",
        ],
    ) || contains_any(&lower, &[" browser back", " go back "])
    {
        direction = Some("back".to_string());
        InstructionKind::Navigate
    } else if starts_with_any(
        &lower,
        &[
            "go forward",
            "navigate forward",
            "forward to ",
            "forward one page",
            "next page in history",
        ],
    ) || contains_any(&lower, &[" browser forward", " go forward "])
    {
        direction = Some("forward".to_string());
        InstructionKind::Navigate
    } else if starts_with_any(&lower, &["reload", "refresh"])
        || contains_any(&lower, &[" reload page", " refresh page"])
    {
        direction = Some("reload".to_string());
        InstructionKind::Navigate
    } else if let Some(url) = instruction_url(instruction) {
        value = Some(url);
        direction = Some("url".to_string());
        InstructionKind::Navigate
    } else if let Some(device) = device_emulation_target(instruction) {
        value = Some(device);
        direction = Some("device".to_string());
        InstructionKind::EmulateDevice
    } else if is_viewport_instruction(&lower) {
        if let Some((width, height)) = viewport_dimensions(instruction) {
            value = Some(format!("{width}x{height}"));
            direction = Some("dimensions".to_string());
        } else if let Some(preset) = viewport_preset(&lower) {
            value = Some(preset.to_string());
            direction = Some("preset".to_string());
        }
        InstructionKind::SetViewport
    } else if is_read_text_instruction(&lower) {
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| read_text_target_hint(instruction));
        value = target_hint.clone();
        direction = Some("visible_text".to_string());
        InstructionKind::ReadText
    } else if is_analyze_form_instruction(&lower) {
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| form_analysis_target_hint(instruction));
        value = target_hint.clone();
        direction = Some("fields".to_string());
        InstructionKind::AnalyzeForm
    } else if is_accessibility_tree_instruction(&lower) {
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| accessibility_tree_target_hint(instruction));
        value = target_hint.clone();
        direction = Some("tree".to_string());
        InstructionKind::AccessibilityTree
    } else if is_feedback_value_search_instruction(&lower) {
        target_hint = field_hint(instruction).or_else(|| Some("answer".to_string()));
        InstructionKind::Fill
    } else if is_endpoint_form_workflow_instruction(&lower) {
        value = quoted.first().cloned();
        target_hint = Some("workflow".to_string());
        InstructionKind::Fill
    } else if is_find_elements_instruction(&lower) {
        let query = find_elements_query(instruction);
        value = query
            .text
            .clone()
            .or_else(|| query.selector.clone())
            .or_else(|| query.role.clone());
        target_hint = value.clone();
        direction = Some(query.direction());
        InstructionKind::FindElements
    } else if starts_with_any(&lower, &["wait ", "wait for ", "wait until "])
        || contains_any(&lower, &[" wait for ", " wait until "])
    {
        direction = if contains_any(
            &lower,
            &[
                " disappear",
                " disappears",
                " disappeared",
                " gone",
                " hidden",
                " not visible",
                " removed",
            ],
        ) {
            Some("hidden".to_string())
        } else if contains_any(
            &lower,
            &[
                "network idle",
                "network to be idle",
                "finish loading",
                "finished loading",
                "page load",
                "page loaded",
            ],
        ) {
            Some("network_idle".to_string())
        } else {
            Some("visible".to_string())
        };
        value = quoted
            .first()
            .cloned()
            .or_else(|| wait_target_hint(instruction));
        target_hint = value.clone();
        InstructionKind::Wait
    } else if starts_with_any(&lower, &["focus ", "focus into ", "focus on "])
        || lower.contains(" focus ")
    {
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["focus into", "focus on", "focus"]));
        InstructionKind::Focus
    } else if starts_with_any(
        &lower,
        &[
            "right click ",
            "right-click ",
            "context click ",
            "context-click ",
        ],
    ) || contains_any(
        &lower,
        &[
            " right click ",
            " right-click ",
            " context click ",
            " context-click ",
        ],
    ) {
        value = Some("right_click".to_string());
        target_hint = quoted.first().cloned().or_else(|| {
            trailing_hint(
                instruction,
                &[
                    "right click",
                    "right-click",
                    "context click",
                    "context-click",
                ],
            )
        });
        InstructionKind::Click
    } else if starts_with_any(&lower, &["double click ", "double-click "])
        || contains_any(&lower, &[" double click ", " double-click "])
    {
        value = Some("double_click".to_string());
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["double click", "double-click"]));
        InstructionKind::Click
    } else if starts_with_any(&lower, &["hover ", "hover over ", "hover on "])
        || contains_any(&lower, &[" hover over ", " hover on "])
    {
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["hover over", "hover on", "hover"]));
        InstructionKind::Hover
    } else if contains_any(&lower, &["sort ", "sortable "])
        && contains_any(
            &lower,
            &[
                "number",
                "numbers",
                "increasing",
                "decreasing",
                "ascending",
                "descending",
                "lowest",
                "highest",
            ],
        )
    {
        target_hint = Some("sortable numbers".to_string());
        InstructionKind::Drag
    } else if contains_any(&lower, &["drag ", "dragged ", "move "]) {
        target_hint = quoted.first().cloned();
        if lower.contains(" to ") && (target_hint.is_none() || secondary_hint.is_none()) {
            let (first, second) = split_around_to(instruction);
            target_hint = target_hint.or(first);
            secondary_hint = secondary_hint.or(second);
        } else if target_hint.is_none() {
            target_hint = trailing_hint(instruction, &["drag", "dragged", "move"]);
        }
        InstructionKind::Drag
    } else if starts_with_any(&lower, &["turn off ", "disable "])
        || contains_any(&lower, &[" turn off ", " disable "])
    {
        checked = Some(false);
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["turn off", "disable"]));
        InstructionKind::SetChecked
    } else if starts_with_any(&lower, &["turn on ", "enable "])
        || contains_any(&lower, &[" turn on ", " enable "])
    {
        checked = Some(true);
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["turn on", "enable"]));
        InstructionKind::SetChecked
    } else if contains_any(&lower, &["uncheck", "untick", "deselect "]) {
        checked = Some(false);
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["uncheck", "untick", "deselect"]));
        InstructionKind::SetChecked
    } else if starts_with_any(&lower, &["check ", "tick "]) {
        checked = Some(true);
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| trailing_hint(instruction, &["check", "tick"]));
        InstructionKind::SetChecked
    } else if let Some(key) = key_press_value(instruction) {
        value = Some(key);
        InstructionKind::PressKey
    } else if is_scoped_workflow_instruction(&lower) {
        target_hint = Some("workflow".to_string());
        InstructionKind::Click
    } else if is_command_surface_instruction(&lower) {
        target_hint = Some("command surface".to_string());
        InstructionKind::Fill
    } else if starts_with_any(&lower, &["clear ", "empty ", "erase "])
        || contains_any(
            &lower,
            &[
                " clear ",
                " empty ",
                " erase ",
                "delete text",
                "delete the text",
            ],
        )
    {
        target_hint = quoted.first().cloned().or_else(|| {
            trailing_hint(
                instruction,
                &["clear", "empty", "erase", "delete text", "delete the text"],
            )
        });
        InstructionKind::ClearField
    } else if starts_with_any(&lower, &["append "])
        || (starts_with_any(&lower, &["add "])
            && contains_any(
                &lower,
                &[
                    " to the field",
                    " to field",
                    " to the input",
                    " to input",
                    " to the textbox",
                    " to textbox",
                    " to the message",
                    " to message",
                    " to the comment",
                    " to comment",
                    " to the editor",
                    " to editor",
                ],
            ))
    {
        let (parsed_value, parsed_target) =
            value_target_from_markers(instruction, &["append", "add"], &[" to ", " into ", " in "]);
        value = quoted.first().cloned().or(parsed_value);
        target_hint = field_hint(instruction).or(parsed_target);
        InstructionKind::AppendField
    } else if contains_any(
        &lower,
        &["upload ", "attach ", "import file", "choose file"],
    ) {
        let (parsed_value, parsed_target) = value_target_from_markers(
            instruction,
            &["upload", "attach", "import file", "choose file"],
            &[" to ", " into ", " in ", " for ", " on "],
        );
        value = quoted
            .first()
            .cloned()
            .or(parsed_value)
            .or_else(|| first_file_path(instruction));
        target_hint = field_hint(instruction).or(parsed_target);
        InstructionKind::UploadFile
    } else if contains_any(&lower, &["draw ", "render ", "make ", "copy "])
        && contains_any(&lower, &["checkbox", "checkboxes", "grid", "pattern"])
        && contains_any(&lower, &["number", "digit", "pattern", "shape"])
    {
        value = value.or_else(|| first_number(instruction));
        target_hint = Some("checkbox grid".to_string());
        InstructionKind::RenderPattern
    } else if contains_any(&lower, &["draw ", "sketch ", "stroke ", "create ", "make "])
        && contains_any(&lower, &["line", "stroke", "path"])
    {
        value = Some("line".to_string());
        target_hint = trailing_hint(instruction, &["draw", "sketch", "stroke", "create", "make"]);
        InstructionKind::Drag
    } else if contains_any(&lower, &["draw ", "sketch ", "stroke "])
        && contains_any(&lower, &["circle", "round", "ellipse", "oval", "arc"])
    {
        value = Some("circle".to_string());
        target_hint = trailing_hint(instruction, &["draw", "sketch", "stroke"]);
        InstructionKind::Drag
    } else if let Some((parsed_value, parsed_target)) = select_option_value_target(instruction) {
        value = value.or(Some(parsed_value));
        target_hint = Some(parsed_target);
        InstructionKind::SelectOption
    } else if has_value_control_hint(&lower)
        && !starts_with_click_instruction(&lower)
        && (starts_with_any(
            &lower,
            &[
                "select ", "choose ", "pick ", "set ", "move ", "use ", "enter ", "input ",
            ],
        ) || lower.contains(" with ")
            || lower.contains(" using ")
            || lower.contains(" on "))
    {
        let (mut parsed_value, mut parsed_target) = value_target_from_markers(
            instruction,
            &[
                "select", "choose", "pick", "set", "move", "use", "enter", "input",
            ],
            &[
                " with ", " using ", " on ", " into ", " in ", " to ", " as ",
            ],
        );
        let parsed_value_is_control = parsed_value
            .as_deref()
            .map(|text| has_value_control_hint(&text.to_lowercase()))
            .unwrap_or(false);
        if parsed_value_is_control && parsed_target.is_some() {
            std::mem::swap(&mut parsed_value, &mut parsed_target);
        }
        let numeric = first_number(instruction);
        let fallback_control_hint = value_control_hint(instruction);
        let numeric_control = parsed_target
            .as_deref()
            .or(fallback_control_hint.as_deref())
            .map(|text| {
                let text = text.to_lowercase();
                contains_any(
                    &text,
                    &[
                        "slider",
                        "range",
                        "spinner",
                        "spinbutton",
                        "stepper",
                        "number",
                        "numeric",
                    ],
                )
            })
            .unwrap_or(false);
        let parsed_value_has_digit = parsed_value
            .as_deref()
            .map(|text| text.chars().any(|ch| ch.is_ascii_digit()))
            .unwrap_or(false);
        let parsed_value = if (numeric_control || !parsed_value_has_digit) && numeric.is_some() {
            numeric
        } else {
            parsed_value
        };
        value = value.or(parsed_value);
        target_hint = parsed_target.or(fallback_control_hint);
        InstructionKind::Fill
    } else if let Some((parsed_value, parsed_target)) = set_field_value_target(instruction) {
        value = value.or(Some(parsed_value));
        target_hint = Some(parsed_target);
        InstructionKind::Fill
    } else if starts_with_any(
        &lower,
        &["select ", "choose ", "pick ", "set dropdown", "set option"],
    ) || lower.contains(" dropdown")
        || lower.contains(" option")
    {
        let (parsed_value, parsed_target) = value_target_from_markers(
            instruction,
            &["select", "choose", "pick"],
            &[" from ", " in ", " for "],
        );
        value = value.or(parsed_value);
        target_hint = parsed_target;
        InstructionKind::SelectOption
    } else if starts_with_any(
        &lower,
        &["type ", "enter ", "fill ", "input ", "write ", "search "],
    ) || contains_any(&lower, &[" into ", " in the field", " in field"])
        || (lower.contains("enter ") && !quoted.is_empty())
        || contains_any(&lower, &["copy ", "paste ", " copy ", " paste "])
    {
        let (parsed_value, parsed_target) = value_target_from_markers(
            instruction,
            &[
                "type", "enter", "fill", "input", "write", "search", "copy", "paste",
            ],
            &[" into ", " in ", " to "],
        );
        value = value.or(parsed_value);
        target_hint = field_hint(instruction).or(parsed_target);
        InstructionKind::Fill
    } else if starts_with_any(
        &lower,
        &[
            "book ",
            "create ",
            "schedule ",
            "reserve ",
            "plan ",
            "order ",
        ],
    ) {
        value = quoted.first().cloned();
        target_hint = Some("workflow".to_string());
        InstructionKind::Fill
    } else if is_scoped_multi_action_shorthand(instruction, &lower) {
        target_hint = Some("scoped multi-action".to_string());
        InstructionKind::Fill
    } else if lower.contains("scroll") {
        direction = if lower.contains("up")
            || lower.contains("top")
            || lower.contains("start")
            || lower.contains("beginning")
        {
            Some("up".to_string())
        } else {
            Some("down".to_string())
        };
        InstructionKind::Scroll
    } else if starts_with_any(
        &lower,
        &[
            "click ",
            "press ",
            "tap ",
            "open ",
            "close ",
            "dismiss ",
            "submit",
            "continue",
            "confirm",
            "save",
            "done",
            "next",
            "buy ",
            "expand ",
            "collapse ",
            "switch ",
            "find ",
            "hit ",
        ],
    ) || lower.contains(" click ")
        || lower.contains(" clicking ")
        || lower.contains(" press ")
        || lower.contains(" tap ")
        || lower.contains(" hit ")
    {
        target_hint = quoted
            .first()
            .cloned()
            .or_else(|| {
                trailing_hint(
                    instruction,
                    &[
                        "click on", "click", "press on", "press", "tap on", "tap", "hit on", "hit",
                        "open", "expand", "collapse", "find", "buy",
                    ],
                )
            })
            .or_else(|| {
                if contains_any(&lower, &["close", "dismiss"]) {
                    Some("close".to_string())
                } else {
                    None
                }
            });
        InstructionKind::Click
    } else {
        InstructionKind::Unknown
    };

    let mut cleaned_value = clean_hint(value);
    if let Some(current) = cleaned_value.as_deref() {
        let was_quoted = quoted
            .iter()
            .any(|quoted_value| quoted_value.eq_ignore_ascii_case(current));
        if !was_quoted && unresolved_reference_value(current) {
            cleaned_value = None;
        }
    }

    InstructionAnalysis {
        kind,
        value: cleaned_value,
        target_hint: clean_hint(target_hint),
        secondary_hint: clean_hint(secondary_hint),
        checked,
        direction,
    }
}

pub(super) fn build_intent(instruction: &str, analysis: &InstructionAnalysis) -> InstructionIntent {
    let lower = instruction.to_lowercase();
    let action_verbs = click_verbs(instruction);
    let ordered_click_hints = ordered_click_hints(instruction);
    let menu_path = menu_path(instruction, analysis);
    let order = if contains_any(&lower, &["descending", "decreasing", "reverse"]) {
        Some("descending".to_string())
    } else if contains_any(
        &lower,
        &[
            "ascending",
            "increasing",
            "smallest to largest",
            "lowest to highest",
            "in order",
        ],
    ) {
        Some("ascending".to_string())
    } else {
        None
    };
    let wants_numeric_targets = contains_any(
        &lower,
        &[
            "number", "numbers", "numeric", "digit", "digits", "value", "values",
        ],
    );
    let wants_ordered_values = order.is_some()
        && wants_numeric_targets
        && (analysis.kind == InstructionKind::Click
            || action_verbs
                .iter()
                .any(|verb| matches!(verb.as_str(), "click" | "press" | "tap" | "hit")));
    let follow_up_click_hint = follow_up_click_hint(instruction);

    InstructionIntent {
        action_verbs,
        ordered_click_hints,
        menu_path,
        order,
        wants_ordered_values,
        wants_numeric_targets,
        follow_up_click_hint,
    }
}

fn menu_path(instruction: &str, analysis: &InstructionAnalysis) -> Vec<String> {
    let lower = instruction.to_lowercase();
    let raw = if matches!(
        analysis.kind,
        InstructionKind::SelectOption | InstructionKind::Click
    ) && starts_with_any(
        &lower,
        &[
            "select ",
            "choose ",
            "pick ",
            "open ",
            "click ",
            "go to ",
            "navigate to ",
        ],
    ) {
        analysis
            .value
            .as_deref()
            .or(analysis.target_hint.as_deref())
            .map(str::to_string)
            .or_else(|| {
                for verb in [
                    "select",
                    "choose",
                    "pick",
                    "open",
                    "click",
                    "go to",
                    "navigate to",
                ] {
                    if let Some(index) = lower.find(verb) {
                        let start = index + verb.len();
                        let tail = instruction[start..].trim();
                        if !tail.is_empty() {
                            return Some(tail.to_string());
                        }
                    }
                }
                None
            })
    } else {
        None
    };
    let Some(raw) = raw else {
        return Vec::new();
    };
    if !raw.contains('>') && !raw.contains('›') && !raw.contains('→') {
        return Vec::new();
    }
    raw.split(['>', '›', '→'])
        .filter_map(clean_menu_path_item)
        .collect()
}

fn clean_menu_path_item(input: &str) -> Option<String> {
    let item = input
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ':' | ';'))
        .trim();
    if item.is_empty() {
        None
    } else {
        Some(item.to_string())
    }
}

fn click_verbs(input: &str) -> Vec<String> {
    let lower = input.to_lowercase();
    let mut verbs = Vec::new();
    for token in lower.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        let verb = match token {
            "click" | "clicking" => Some("click"),
            "press" | "pressing" => Some("press"),
            "tap" | "tapping" => Some("tap"),
            "hit" | "hitting" => Some("hit"),
            _ => None,
        };
        if let Some(verb) = verb {
            if !verbs.iter().any(|existing| existing == verb) {
                verbs.push(verb.to_string());
            }
        }
    }
    verbs
}

fn ordered_click_hints(input: &str) -> Vec<String> {
    split_click_clauses(input)
        .into_iter()
        .filter_map(extract_click_hint)
        .collect()
}

fn split_click_clauses(input: &str) -> Vec<&str> {
    let mut clauses = Vec::new();
    let mut start = 0;
    let lower = input.to_lowercase();
    let mut indices = lower
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    indices.push(lower.len());
    let mut cursor = 0;
    while cursor < indices.len() {
        let index = indices[cursor];
        let tail = &lower[index..];
        let separator = if tail.starts_with(", then ") {
            Some(", then ".len())
        } else if tail.starts_with(" then ") {
            Some(" then ".len())
        } else if tail.starts_with(" and ") {
            Some(" and ".len())
        } else if tail.starts_with(';') || tail.starts_with('.') {
            Some(1)
        } else {
            None
        };
        if let Some(length) = separator {
            let clause = input[start..index].trim();
            if !clause.is_empty() {
                clauses.push(clause);
            }
            start = index + length;
            while cursor < indices.len() && indices[cursor] < start {
                cursor += 1;
            }
        } else {
            cursor += 1;
        }
    }
    let clause = input[start..].trim();
    if !clause.is_empty() {
        clauses.push(clause);
    }
    clauses
}

fn extract_click_hint(clause: &str) -> Option<String> {
    let lower = clause.to_lowercase();
    let mut match_index = None;
    let mut match_verb_len = 0;
    for raw in [
        "clicking", "click", "pressing", "press", "tapping", "tap", "hitting", "hit",
    ] {
        if let Some(index) = lower.find(raw) {
            let before_ok = index == 0
                || !lower[..index]
                    .chars()
                    .last()
                    .map(|ch| ch.is_ascii_alphanumeric())
                    .unwrap_or(false);
            let after_index = index + raw.len();
            let after_ok = after_index >= lower.len()
                || !lower[after_index..]
                    .chars()
                    .next()
                    .map(|ch| ch.is_ascii_alphanumeric())
                    .unwrap_or(false);
            if before_ok && after_ok && match_index.map(|current| index < current).unwrap_or(true) {
                match_index = Some(index);
                match_verb_len = raw.len();
            }
        }
    }
    let start = match_index? + match_verb_len;
    clean_click_hint(&clause[start..])
}

fn clean_click_hint(input: &str) -> Option<String> {
    let ignored = [
        "the", "a", "an", "on", "button", "link", "control", "item", "element", "labelled",
        "labeled", "called", "named", "with", "using", "icon", "icons",
    ];
    let cleaned = input
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ':' | ';'))
        .split_whitespace()
        .filter_map(|word| {
            let trimmed =
                word.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ':' | ';'));
            let lower = word
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ':' | ';'))
                .to_lowercase();
            if trimmed.is_empty() || ignored.contains(&lower.as_str()) {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = cleaned
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ':' | ';'))
        .trim()
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn follow_up_click_hint(input: &str) -> Option<String> {
    let hints = ordered_click_hints(input);
    if hints.len() < 2 {
        None
    } else {
        hints.last().cloned()
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn starts_with_any(haystack: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| haystack.starts_with(prefix))
}

fn starts_with_click_instruction(lower: &str) -> bool {
    starts_with_any(
        lower,
        &[
            "click ",
            "click on ",
            "press ",
            "press on ",
            "tap ",
            "tap on ",
            "hit ",
            "hit on ",
        ],
    )
}

fn is_feedback_value_search_instruction(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "guess ",
            "find hidden",
            "find a hidden",
            "find the hidden",
            "find secret",
            "find a secret",
            "find the secret",
            "find unknown",
            "find an unknown",
            "find the unknown",
        ],
    ) && contains_any(lower, &["number", "value"])
}

fn is_command_surface_instruction(lower: &str) -> bool {
    let surface = contains_command_surface_noun(lower);
    let action = [
        "use", "run", "execute", "type", "enter", "delete", "remove", "list",
    ]
    .iter()
    .any(|word| contains_word(lower, word));
    surface && action
}

fn contains_command_surface_noun(lower: &str) -> bool {
    ["terminal", "shell", "console", "cli", "repl"]
        .iter()
        .any(|word| contains_word(lower, word))
        || contains_phrase(lower, "command prompt")
        || contains_phrase(lower, "command line")
}

fn contains_word(lower: &str, word: &str) -> bool {
    lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token == word)
}

fn contains_phrase(lower: &str, phrase: &str) -> bool {
    let normalized = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(phrase.split_whitespace().count())
        .any(|window| window.join(" ") == phrase)
}

fn is_scoped_workflow_instruction(lower: &str) -> bool {
    if starts_with_any(lower, &["enter ", "type ", "write ", "fill ", "input "])
        && !contains_any(lower, &["reply", "respond", "forward"])
    {
        return false;
    }
    if contains_any(
        lower,
        &[
            " starts with ",
            " ends with ",
            "autocomplete",
            "auto-complete",
        ],
    ) {
        return false;
    }
    let has_item_surface = contains_any(
        lower,
        &[
            "email",
            "e-mail",
            "message",
            "conversation",
            "thread",
            "ticket",
            "record",
            "row",
            "card",
            "item",
            "post",
            "entry",
            "information",
            "details",
            "content",
        ],
    );
    has_item_surface
        && contains_any(
            lower,
            &[
                "reply",
                "respond",
                "forward",
                "send",
                "delete",
                "remove",
                "trash",
                "archive",
                "star",
                "important",
                "favorite",
                "favourite",
                "turn on",
                "enable",
                "check",
                "tick",
                "turn off",
                "disable",
                "uncheck",
                "untick",
                "waiting for",
                "wants the email",
                "wants the e-mail",
                "wants the message",
            ],
        )
}

fn has_value_control_hint(lower: &str) -> bool {
    contains_any(
        lower,
        &[
            "slider",
            "range",
            "spinner",
            "spinbutton",
            "stepper",
            "number",
            "numeric",
            "date",
            "time",
            "month",
            "week",
            "color",
            "colour",
            "datetime",
        ],
    )
}

fn value_control_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    for word in [
        "slider",
        "range",
        "spinner",
        "spinbutton",
        "stepper",
        "number",
        "numeric",
        "date",
        "time",
        "month",
        "week",
        "color",
        "colour",
        "datetime",
    ] {
        if lower.contains(word) {
            return Some(word.to_string());
        }
    }
    None
}

fn first_number(input: &str) -> Option<String> {
    let mut current = String::new();
    let mut started = false;
    let mut seen_digit = false;
    for ch in input.chars() {
        if ch.is_ascii_digit() || (ch == '-' && !started) || (ch == '.' && started) {
            current.push(ch);
            started = true;
            if ch.is_ascii_digit() {
                seen_digit = true;
            }
        } else if started {
            break;
        }
    }
    if seen_digit {
        Some(current)
    } else {
        None
    }
}

fn first_file_path(input: &str) -> Option<String> {
    input
        .split_whitespace()
        .map(|part| {
            part.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '.' | ',' | ':' | ';' | '(' | ')' | '[' | ']'
                )
            })
        })
        .find(|part| {
            part.starts_with('/')
                || part.starts_with("./")
                || part.starts_with("../")
                || part.starts_with("~/")
        })
        .map(ToString::to_string)
}

fn key_press_value(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    if !starts_with_any(
        &lower,
        &["press ", "hit ", "type key ", "press key ", "send key "],
    ) && !contains_any(&lower, &[" press key ", " hit key ", " press the key "])
    {
        return None;
    }
    let quoted = quoted_strings(input);
    let raw = quoted.first().cloned().or_else(|| {
        trailing_hint(
            input,
            &[
                "press the key",
                "press key",
                "type key",
                "send key",
                "press",
                "hit",
            ],
        )
    })?;
    normalize_key_name(&raw)
}

fn normalize_key_name(raw: &str) -> Option<String> {
    let cleaned = clean_hint(Some(
        raw.replace(" key", "")
            .replace("keyboard", "")
            .replace("button", ""),
    ))?;
    let compact = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("+")
        .replace("++", "+");
    if compact.contains('+') {
        let parts: Vec<String> = compact
            .split('+')
            .filter_map(normalize_single_key)
            .collect();
        if parts.len() >= 2 {
            return Some(parts.join("+"));
        }
        return None;
    }
    normalize_single_key(&compact)
}

fn normalize_single_key(raw: &str) -> Option<String> {
    let key = raw
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ':' | ';'))
        .to_lowercase();
    let normalized = match key.as_str() {
        "enter" | "return" => "Enter",
        "escape" | "esc" => "Escape",
        "tab" => "Tab",
        "space" | "spacebar" => " ",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "arrowup" | "up" | "up-arrow" | "arrow-up" => "ArrowUp",
        "arrowdown" | "down" | "down-arrow" | "arrow-down" => "ArrowDown",
        "arrowleft" | "left" | "left-arrow" | "arrow-left" => "ArrowLeft",
        "arrowright" | "right" | "right-arrow" | "arrow-right" => "ArrowRight",
        "home" => "Home",
        "end" => "End",
        "pageup" | "page-up" => "PageUp",
        "pagedown" | "page-down" => "PageDown",
        "meta" | "cmd" | "command" => "Meta",
        "control" | "ctrl" => "Control",
        "shift" => "Shift",
        "alt" | "option" => "Alt",
        _ if key.len() == 1 && key.chars().all(|ch| ch.is_ascii_alphanumeric()) => raw.trim(),
        _ => return None,
    };
    Some(normalized.to_string())
}

fn quoted_strings(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut active = None;
    let mut current = String::new();
    for ch in input.chars() {
        if ch == '"' || ch == '\'' {
            match active {
                Some(q) if q == ch => {
                    if !current.trim().is_empty() {
                        out.push(current.trim().to_string());
                    }
                    current.clear();
                    active = None;
                }
                None => active = Some(ch),
                _ => current.push(ch),
            }
        } else if active.is_some() {
            current.push(ch);
        }
    }
    out
}

fn trailing_hint(input: &str, verbs: &[&str]) -> Option<String> {
    let lower = input.to_lowercase();
    for verb in verbs {
        if let Some(index) = lower.find(verb) {
            let start = index + verb.len();
            let hint = input[start..]
                .trim()
                .trim_start_matches(|ch: char| ch == ':' || ch == '-' || ch.is_whitespace())
                .trim();
            if !hint.is_empty() {
                return Some(hint.to_string());
            }
        }
    }
    None
}

fn is_assert_instruction(lower: &str) -> bool {
    starts_with_any(
        lower,
        &[
            "verify ",
            "verify that ",
            "assert ",
            "assert that ",
            "expect ",
            "expect that ",
            "ensure ",
            "ensure that ",
            "confirm that ",
            "check that ",
        ],
    )
}

fn is_screenshot_instruction(lower: &str) -> bool {
    starts_with_any(
        lower,
        &[
            "screenshot",
            "take screenshot",
            "take a screenshot",
            "take a full page screenshot",
            "take full page screenshot",
            "take an entire page screenshot",
            "take entire page screenshot",
            "capture screenshot",
            "capture a screenshot",
            "capture a full page screenshot",
            "capture full page screenshot",
            "grab screenshot",
            "grab a screenshot",
        ],
    ) || contains_any(
        lower,
        &[
            " take screenshot",
            " capture screenshot",
            " screenshot of ",
            " full page screenshot",
            " entire page screenshot",
        ],
    )
}

fn screenshot_target_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    for marker in [
        "screenshot of ",
        "screenshot for ",
        "screenshot from ",
        "screenshot ",
        "capture a screenshot of ",
        "capture screenshot of ",
        "take a screenshot of ",
        "take screenshot of ",
        "grab a screenshot of ",
        "grab screenshot of ",
    ] {
        if let Some(index) = lower.find(marker) {
            let start = index + marker.len();
            let target = input[start..]
                .trim()
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ';'));
            if target.is_empty()
                || target.eq_ignore_ascii_case("the page")
                || target.eq_ignore_ascii_case("page")
                || target.eq_ignore_ascii_case("full page")
                || target.eq_ignore_ascii_case("entire page")
                || target.eq_ignore_ascii_case("whole page")
            {
                return None;
            }
            return clean_hint(Some(target.to_string()));
        }
    }
    None
}

fn assert_target_hint(input: &str) -> Option<String> {
    let raw = trailing_hint(
        input,
        &[
            "verify that",
            "verify",
            "assert that",
            "assert",
            "expect that",
            "expect",
            "ensure that",
            "ensure",
            "confirm that",
            "check that",
        ],
    )?;
    let lower = raw.to_lowercase();
    let mut end = raw.len();
    for marker in [
        " is visible",
        " visible",
        " is shown",
        " shown",
        " appears",
        " appear",
        " is present",
        " present",
        " is not visible",
        " not visible",
        " is hidden",
        " hidden",
        " is not shown",
        " not shown",
        " is absent",
        " absent",
        " is not present",
        " not present",
        " disappears",
        " disappeared",
        " is not checked",
        " not checked",
        " is not selected",
        " not selected",
        " is checked",
        " checked",
        " is unchecked",
        " unchecked",
        " is selected",
        " selected",
        " is unselected",
        " unselected",
        " is on",
        " on",
        " is off",
        " off",
        " is enabled",
        " enabled",
    ] {
        if let Some(index) = lower.find(marker) {
            end = end.min(index);
        }
    }
    clean_hint(Some(raw[..end].trim().to_string()))
}

fn assert_value_target(input: &str) -> Option<(String, String)> {
    let raw = trailing_hint(
        input,
        &[
            "verify that",
            "verify",
            "assert that",
            "assert",
            "expect that",
            "expect",
            "ensure that",
            "ensure",
            "confirm that",
            "check that",
        ],
    )?;
    let lower = raw.to_lowercase();
    for marker in [
        " value equals ",
        " value is ",
        " equals ",
        " should equal ",
        " should be ",
    ] {
        if let Some(index) = lower.find(marker) {
            let target = clean_hint(Some(
                raw[..index].trim().trim_end_matches([':', '-']).to_string(),
            ))?;
            let value = clean_hint(Some(raw[index + marker.len()..].trim().to_string()))?;
            let value_lower = value.to_lowercase();
            if matches!(
                value_lower.as_str(),
                "visible"
                    | "shown"
                    | "present"
                    | "hidden"
                    | "absent"
                    | "checked"
                    | "unchecked"
                    | "selected"
                    | "unselected"
                    | "on"
                    | "off"
                    | "enabled"
                    | "disabled"
            ) {
                return None;
            }
            return Some((value, target));
        }
    }
    None
}

fn text_after_markers(input: &str, markers: &[&str]) -> Option<String> {
    let lower = input.to_lowercase();
    for marker in markers {
        if let Some(index) = lower.find(marker) {
            let start = index + marker.len();
            return clean_hint(Some(input[start..].trim().to_string()));
        }
    }
    None
}

fn wait_target_hint(input: &str) -> Option<String> {
    let raw = trailing_hint(input, &["wait until", "wait for", "wait"])?;
    let lower = raw.to_lowercase();
    let mut end = raw.len();
    for marker in [
        " to appear",
        " appears",
        " appear",
        " is visible",
        " visible",
        " is shown",
        " shown",
        " displays",
        " display",
        " to disappear",
        " disappears",
        " disappear",
        " is hidden",
        " hidden",
        " is gone",
        " gone",
        " is removed",
        " removed",
        " to load",
        " loads",
        " loaded",
    ] {
        if let Some(index) = lower.find(marker) {
            end = end.min(index);
        }
    }
    clean_hint(Some(raw[..end].trim().to_string()))
}

pub(super) fn looks_like_selector(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains(' ') {
        return false;
    }
    trimmed.starts_with('#')
        || trimmed.starts_with('.')
        || trimmed.starts_with('[')
        || trimmed.contains('>')
        || trimmed.contains(':')
        || trimmed.contains("[")
        || matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "button"
                | "input"
                | "select"
                | "textarea"
                | "form"
                | "main"
                | "nav"
                | "dialog"
                | "table"
        )
}

pub(super) fn wait_timeout_ms(instruction: &str) -> Option<u64> {
    let lower = instruction.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    for window in words.windows(2) {
        let Some(amount) = window[0].parse::<u64>().ok() else {
            continue;
        };
        let unit = window[1].trim_matches(|ch: char| !ch.is_ascii_alphabetic());
        if unit.starts_with("ms") || unit.starts_with("millisecond") {
            return Some(amount.clamp(1, 60_000));
        }
        if unit.starts_with("sec") || unit == "s" || unit.starts_with("second") {
            return Some(amount.saturating_mul(1000).clamp(1, 60_000));
        }
    }
    None
}

fn instruction_url(instruction: &str) -> Option<String> {
    let lower = instruction.to_lowercase();
    if !starts_with_any(
        &lower,
        &[
            "open ",
            "navigate ",
            "navigate to ",
            "go to ",
            "visit ",
            "load ",
            "browse to ",
        ],
    ) {
        return None;
    }
    for candidate in quoted_strings(instruction)
        .into_iter()
        .chain(instruction.split_whitespace().map(str::to_string))
    {
        if let Some(url) = normalize_navigation_url(&candidate) {
            return Some(url);
        }
    }
    trailing_hint(
        instruction,
        &[
            "navigate to",
            "navigate",
            "go to",
            "open",
            "visit",
            "load",
            "browse to",
        ],
    )
    .and_then(|hint| normalize_navigation_url(&hint))
}

fn normalize_navigation_url(candidate: &str) -> Option<String> {
    let trimmed = candidate
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ';'));
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("about:")
        || lower.starts_with("file:")
        || lower.starts_with("data:")
    {
        return Some(trimmed.to_string());
    }
    if lower.starts_with("localhost:")
        || lower.starts_with("127.0.0.1:")
        || lower.starts_with("[::1]:")
    {
        return Some(format!("http://{trimmed}"));
    }
    if trimmed.contains('.') && !trimmed.contains(char::is_whitespace) {
        return Some(format!("https://{trimmed}"));
    }
    None
}

fn is_viewport_instruction(lower: &str) -> bool {
    (starts_with_any(
        lower,
        &[
            "set viewport",
            "resize viewport",
            "change viewport",
            "set browser size",
            "resize browser",
            "set window size",
            "resize window",
            "set screen size",
            "set resolution",
            "switch viewport",
            "use viewport",
        ],
    ) || (starts_with_any(lower, &["emulate ", "switch to ", "use ", "view as "])
        && contains_any(lower, &[" mobile", " tablet", " desktop", " wide"])))
        && contains_any(
            lower,
            &[
                "viewport",
                "browser",
                "window",
                "screen",
                "resolution",
                "mobile",
                "tablet",
                "desktop",
                "wide",
            ],
        )
}

fn viewport_preset(lower: &str) -> Option<&'static str> {
    if lower.contains("mobile") || lower.contains("phone") || lower.contains("narrow") {
        Some("mobile")
    } else if lower.contains("tablet") {
        Some("tablet")
    } else if lower.contains("wide") || lower.contains("large desktop") {
        Some("wide")
    } else if lower.contains("desktop") || lower.contains("laptop") {
        Some("desktop")
    } else {
        None
    }
}

pub(super) fn viewport_dimensions(input: &str) -> Option<(i64, i64)> {
    let numbers = positive_numbers(input);
    if numbers.len() < 2 {
        return None;
    }
    let width = numbers[0];
    let height = numbers[1];
    if (100..=10_000).contains(&width) && (100..=10_000).contains(&height) {
        Some((width, height))
    } else {
        None
    }
}

fn positive_numbers(input: &str) -> Vec<i64> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    for ch in input.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(value) = current.parse::<i64>() {
                numbers.push(value);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Ok(value) = current.parse::<i64>() {
            numbers.push(value);
        }
    }
    numbers
}

fn device_emulation_target(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    if !starts_with_any(
        &lower,
        &[
            "emulate ",
            "emulate device ",
            "switch to device ",
            "use device ",
            "view as ",
            "test as ",
            "simulate ",
        ],
    ) {
        return None;
    }

    let target = trailing_hint(
        input,
        &[
            "emulate device",
            "emulate",
            "switch to device",
            "use device",
            "view as",
            "test as",
            "simulate",
        ],
    )?;
    let target = target
        .trim()
        .trim_start_matches("the ")
        .trim_end_matches(" device")
        .trim()
        .to_string();
    let target_lower = target.to_lowercase();
    if target.is_empty()
        || matches!(
            target_lower.as_str(),
            "mobile" | "phone" | "tablet" | "desktop" | "wide" | "browser"
        )
        || contains_any(&target_lower, &["viewport", "window size", "screen size"])
    {
        None
    } else {
        Some(target)
    }
}

fn is_read_text_instruction(lower: &str) -> bool {
    if contains_any(lower, &[" into ", " to field", " to the field", " paste "]) {
        return false;
    }
    starts_with_any(
        lower,
        &[
            "read ",
            "read the ",
            "get text ",
            "get the text ",
            "extract text ",
            "extract the text ",
            "show text ",
            "show the text ",
            "return text ",
            "return the text ",
            "tell me what ",
            "what does ",
        ],
    ) || contains_any(
        lower,
        &[
            " read text",
            " extract text",
            " get text from",
            " text from ",
        ],
    )
}

fn read_text_target_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    for marker in [
        " text from ",
        " text in ",
        " text of ",
        " from ",
        " in ",
        " of ",
        "read ",
        "read the ",
        "get text ",
        "get the text ",
        "extract text ",
        "extract the text ",
        "show text ",
        "show the text ",
        "return text ",
        "return the text ",
        "what does ",
        "tell me what ",
    ] {
        if let Some(index) = lower.find(marker) {
            let start = index + marker.len();
            let raw = input[start..]
                .trim()
                .trim_start_matches("the ")
                .trim_end_matches(" say")
                .trim_end_matches(" says")
                .trim_end_matches(" contain")
                .trim_end_matches(" contains")
                .trim_end_matches(" text")
                .trim_end_matches('.')
                .trim();
            if matches!(
                raw.to_lowercase().as_str(),
                "" | "page" | "the page" | "current page" | "body" | "document"
            ) {
                return None;
            }
            return clean_hint(Some(raw.to_string()));
        }
    }
    None
}

fn is_analyze_form_instruction(lower: &str) -> bool {
    if contains_any(
        lower,
        &[
            " fill ",
            " enter ",
            " type ",
            " submit ",
            " value equals",
            " is checked",
            " not checked",
        ],
    ) {
        return false;
    }
    starts_with_any(
        lower,
        &[
            "analyze form",
            "analyze the form",
            "analyze ",
            "inspect form",
            "inspect the form",
            "inspect ",
            "describe form",
            "describe the form",
            "list form fields",
            "list the form fields",
            "show form fields",
            "show the form fields",
            "get form fields",
            "get the form fields",
        ],
    ) && contains_any(
        lower,
        &["form", "field", "input", "checkout", "signup", "sign up"],
    )
}

fn form_analysis_target_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    for marker in [
        " fields in ",
        " fields from ",
        " form in ",
        " form from ",
        " form ",
        "analyze ",
        "inspect ",
        "describe ",
        "list form fields in ",
        "show form fields in ",
        "get form fields in ",
    ] {
        if let Some(index) = lower.find(marker) {
            let raw = input[index + marker.len()..]
                .trim()
                .trim_start_matches("the ")
                .trim_end_matches(" fields")
                .trim_end_matches(" inputs")
                .trim_end_matches(" form")
                .trim_end_matches('.')
                .trim();
            if matches!(
                raw.to_lowercase().as_str(),
                "" | "page" | "the page" | "current page" | "form" | "fields"
            ) {
                return None;
            }
            return clean_hint(Some(raw.to_string()));
        }
    }
    None
}

fn is_accessibility_tree_instruction(lower: &str) -> bool {
    starts_with_any(
        lower,
        &[
            "accessibility tree",
            "show accessibility tree",
            "get accessibility tree",
            "inspect accessibility tree",
            "a11y tree",
            "show a11y tree",
            "get a11y tree",
            "inspect a11y tree",
            "show roles",
            "list roles",
            "inspect roles",
            "show role names",
            "list role names",
        ],
    ) || contains_any(
        lower,
        &[
            " accessibility tree",
            " a11y tree",
            " role tree",
            " roles and names",
            " role names",
        ],
    )
}

fn accessibility_tree_target_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    for marker in [
        " tree for ",
        " tree from ",
        " tree in ",
        " roles for ",
        " roles from ",
        " roles in ",
        " names for ",
        " names from ",
        " names in ",
        " of ",
        " for ",
        " from ",
        " in ",
    ] {
        if let Some(index) = lower.find(marker) {
            let raw = input[index + marker.len()..]
                .trim()
                .trim_start_matches("the ")
                .trim_end_matches(" accessibility tree")
                .trim_end_matches(" a11y tree")
                .trim_end_matches(" role tree")
                .trim_end_matches('.')
                .trim();
            if matches!(
                raw.to_lowercase().as_str(),
                "" | "page" | "the page" | "current page" | "document" | "body"
            ) {
                return None;
            }
            return clean_hint(Some(raw.to_string()));
        }
    }
    None
}

#[derive(Debug, Clone, Default)]
pub(super) struct FindElementsQuery {
    pub(super) role: Option<String>,
    pub(super) text: Option<String>,
    pub(super) selector: Option<String>,
}

impl FindElementsQuery {
    fn direction(&self) -> String {
        if self.selector.is_some() {
            "selector".to_string()
        } else if self.role.is_some() && self.text.is_some() {
            "role_text".to_string()
        } else if self.role.is_some() {
            "role".to_string()
        } else {
            "text".to_string()
        }
    }
}

fn is_find_elements_instruction(lower: &str) -> bool {
    if has_downstream_action_after_find(lower) {
        return false;
    }
    if is_endpoint_form_workflow_instruction(lower) {
        return false;
    }
    starts_with_any(
        lower,
        &[
            "find ",
            "locate ",
            "search for ",
            "list elements",
            "list the elements",
            "show elements",
            "show the elements",
            "find elements",
            "find the elements",
        ],
    ) || contains_any(lower, &[" find elements ", " locate elements "])
}

fn is_endpoint_form_workflow_instruction(lower: &str) -> bool {
    let has_endpoint_pair = ((lower.contains("from:") || lower.contains(" from "))
        && (lower.contains("to:") || lower.contains(" to ")))
        || (lower.contains(" between ") && lower.contains(" and "));
    if !has_endpoint_pair {
        return false;
    }
    let has_form_signal = contains_any(
        lower,
        &[
            " option",
            " request",
            " route",
            " service",
            " booking",
            " reservation",
            " date",
            " on ",
            "/",
            "cheapest",
            "lowest",
            "least expensive",
            "shortest",
            "fastest",
        ],
    );
    let action_input = strip_request_prefix(lower);
    let has_action_signal = starts_with_any(action_input, ENDPOINT_FORM_ACTION_PREFIXES);
    has_form_signal && (has_action_signal || is_implicit_endpoint_request(action_input))
}

const ENDPOINT_FORM_ACTION_PREFIXES: &[&str] = &[
    "find ",
    "search ",
    "search for ",
    "look for ",
    "look up ",
    "show ",
    "show me ",
    "get ",
    "compare ",
    "choose ",
    "select ",
    "pick ",
    "reserve ",
    "book ",
    "order ",
    "schedule ",
    "plan ",
];

fn is_implicit_endpoint_request(input: &str) -> bool {
    let implicit_start = starts_with_any(
        input,
        &[
            "the ",
            "a ",
            "an ",
            "my ",
            "our ",
            "some ",
            "any ",
            "cheapest ",
            "lowest ",
            "least expensive ",
            "shortest ",
            "fastest ",
        ],
    );
    implicit_start
        && contains_any(
            input,
            &[
                "cheapest",
                "lowest",
                "least expensive",
                "shortest",
                "fastest",
                " option",
                " service",
                " request",
            ],
        )
}

fn strip_request_prefix(input: &str) -> &str {
    let trimmed = input.trim_start();
    for prefix in REQUEST_PREFIXES {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim_start();
        }
    }
    trimmed
}

const REQUEST_PREFIXES: &[&str] = &[
    "can you ",
    "could you ",
    "would you ",
    "please ",
    "please help me ",
    "help me ",
    "i need ",
    "i need you to ",
    "i want ",
    "i want you to ",
    "i'd like ",
    "i'd like you to ",
    "i would like ",
    "i would like you to ",
    "can we ",
    "could we ",
    "let's ",
];

fn has_downstream_action_after_find(lower: &str) -> bool {
    let Some(start) = ["find ", "locate ", "search for "]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
    else {
        return false;
    };
    let tail = &lower[start..];
    let action_verbs = [
        "click", "press", "tap", "hit", "reply", "respond", "forward", "delete", "remove", "trash",
        "archive", "mark", "star", "type", "fill", "enter", "write", "set", "select", "choose",
        "submit", "send", "save", "turn on", "enable", "check", "tick", "turn off", "disable",
        "uncheck", "untick", "deselect",
    ];
    action_verbs.iter().any(|verb| {
        contains_any(
            tail,
            &[
                &format!(" and {verb} "),
                &format!(" then {verb} "),
                &format!(" and {verb}."),
                &format!(" then {verb}."),
                &format!(" and {verb},"),
                &format!(" then {verb},"),
                &format!(" and {verb} to "),
                &format!(" then {verb} to "),
            ],
        )
    })
}

fn is_scoped_multi_action_shorthand(input: &str, lower: &str) -> bool {
    let leading_named_container_colon_index = leading_named_container_colon_index(input);
    let starts_scoped = starts_with_any(
        lower,
        &[
            "in ",
            "inside ",
            "within ",
            "in the ",
            "inside the ",
            "within the ",
        ],
    );
    let starts_record_first = starts_with_any(lower, &["for ", "on "])
        || lower
            .split_once(':')
            .or_else(|| lower.split_once(','))
            .or_else(|| lower.split_once(';'))
            .map(|(head, _)| {
                contains_any(
                    head,
                    &[" row", " card", " item", " record", " entry", " result"],
                )
            })
            .unwrap_or(false);
    let starts_named_container_first = leading_named_container_colon_index.is_some();
    if !starts_scoped && !starts_record_first && !starts_named_container_first {
        return false;
    }
    if starts_scoped
        && !contains_any(
            lower,
            &[
                " row ",
                " card ",
                " item ",
                " record ",
                " entry ",
                " result ",
                " section",
                " panel",
                " region",
                " group",
                " fieldset",
                " form",
                " area",
                " containing ",
                " with ",
                " for ",
                " named ",
                " called ",
            ],
        )
    {
        return false;
    }

    let row_colon_index = input.find(':').and_then(|index| {
        let before_colon = input[..index].to_lowercase();
        if contains_any(
            &before_colon,
            &[" row", " card", " item", " record", " entry", " result"],
        ) {
            Some(index)
        } else {
            None
        }
    });
    let Some(split_index) = [
        input.find(','),
        input.find(';'),
        row_colon_index,
        leading_named_container_colon_index,
    ]
    .into_iter()
    .flatten()
    .min() else {
        return false;
    };
    let body = input[split_index + 1..].trim();
    if body.is_empty() {
        return false;
    }
    let normalized = body
        .replace(';', ",")
        .replace(['\r', '\n'], ",")
        .replace(" and ", ",")
        .replace(" then ", ",");
    let (field_like_clauses, action_like_clauses) = normalized
        .split(',')
        .map(str::trim)
        .map(|clause| {
            clause
                .trim_start_matches(|ch: char| ch.is_ascii_digit())
                .trim_start_matches(['-', '*', '.', ')', ' '])
                .trim_matches(|ch: char| matches!(ch, '.' | ';' | ','))
        })
        .fold((0usize, 0usize), |(field_count, action_count), clause| {
            if clause.is_empty() {
                return (field_count, action_count);
            }
            let lower_clause = clause.to_lowercase();
            if starts_with_any(
                &lower_clause,
                &[
                    "click ", "press ", "tap ", "hit ", "submit", "save", "continue", "confirm",
                    "done",
                ],
            ) {
                return (field_count, action_count + 1);
            }
            let has_key_value_separator = clause
                .split_once(':')
                .or_else(|| clause.split_once('='))
                .map(|(left, right)| !left.trim().is_empty() && !right.trim().is_empty())
                .unwrap_or(false);
            if has_key_value_separator || clause.split_whitespace().count() >= 2 {
                (field_count + 1, action_count)
            } else {
                (field_count, action_count)
            }
        });
    field_like_clauses >= 2 || (field_like_clauses >= 1 && action_like_clauses >= 1)
}

fn leading_named_container_colon_index(input: &str) -> Option<usize> {
    let index = input.find(':')?;
    let head = input[..index]
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\''));
    if head.is_empty() {
        return None;
    }
    let lower_head = head.to_lowercase();
    if starts_with_any(
        &lower_head,
        &[
            "http", "https", "click ", "press ", "tap ", "set ", "enter ", "type ", "fill ",
            "choose ", "select ", "open ", "go ",
        ],
    ) {
        return None;
    }
    if contains_any(
        &lower_head,
        &[" row", " card", " item", " record", " entry", " result"],
    ) {
        return None;
    }
    let word_count = head.split_whitespace().count();
    if !(1..=6).contains(&word_count) {
        return None;
    }
    if !head.chars().any(|ch| ch.is_alphabetic()) {
        return None;
    }
    Some(index)
}

pub(super) fn find_elements_query(input: &str) -> FindElementsQuery {
    let lower = input.to_lowercase();
    let quoted = quoted_strings(input);
    if let Some(selector) = quoted
        .iter()
        .find(|value| looks_like_selector(value))
        .cloned()
        .or_else(|| {
            input
                .split_whitespace()
                .map(|word| {
                    word.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ';'))
                })
                .find(|word| looks_like_selector(word))
                .map(str::to_string)
        })
    {
        return FindElementsQuery {
            selector: Some(selector),
            ..Default::default()
        };
    }

    let role = find_role_hint(&lower);
    let mut text = quoted
        .iter()
        .find(|value| !looks_like_selector(value))
        .cloned()
        .or_else(|| text_after_markers(input, &["named", "called", "with text", "containing"]));
    if text.is_none() {
        text = trailing_hint(
            input,
            &[
                "find elements",
                "find the elements",
                "find",
                "locate elements",
                "locate",
                "search for",
                "list elements",
                "list the elements",
                "show elements",
                "show the elements",
            ],
        )
        .map(|hint| clean_find_text_hint(&hint));
    }

    FindElementsQuery {
        role,
        text: text.and_then(|value| clean_hint(Some(value))),
        selector: None,
    }
}

fn find_role_hint(lower: &str) -> Option<String> {
    for (needle, role) in [
        ("button", "button"),
        ("buttons", "button"),
        ("link", "link"),
        ("links", "link"),
        ("textbox", "textbox"),
        ("text box", "textbox"),
        ("input", "textbox"),
        ("inputs", "textbox"),
        ("field", "textbox"),
        ("fields", "textbox"),
        ("checkbox", "checkbox"),
        ("checkboxes", "checkbox"),
        ("radio", "radio"),
        ("radios", "radio"),
        ("combobox", "combobox"),
        ("combo box", "combobox"),
        ("dropdown", "combobox"),
        ("select", "combobox"),
        ("option", "option"),
        ("options", "option"),
        ("tab", "tab"),
        ("tabs", "tab"),
        ("dialog", "dialog"),
        ("menuitem", "menuitem"),
        ("menu item", "menuitem"),
        ("slider", "slider"),
        ("spinbutton", "spinbutton"),
        ("spinner", "spinbutton"),
    ] {
        if lower.contains(needle) {
            return Some(role.to_string());
        }
    }
    None
}

fn clean_find_text_hint(hint: &str) -> String {
    let mut text = hint
        .replace("buttons", " ")
        .replace("button", " ")
        .replace("links", " ")
        .replace("link", " ")
        .replace("textboxes", " ")
        .replace("textbox", " ")
        .replace("text boxes", " ")
        .replace("text box", " ")
        .replace("inputs", " ")
        .replace("input", " ")
        .replace("fields", " ")
        .replace("field", " ")
        .replace("checkboxes", " ")
        .replace("checkbox", " ")
        .replace("dropdowns", " ")
        .replace("dropdown", " ")
        .replace("elements", " ")
        .replace("element", " ");
    for marker in [" named ", " called ", " with text ", " containing "] {
        if let Some(index) = text.to_lowercase().find(marker) {
            text = text[index + marker.len()..].to_string();
        }
    }
    text.trim().to_string()
}

fn split_around_to(input: &str) -> (Option<String>, Option<String>) {
    let lower = input.to_lowercase();
    if let Some(index) = lower.find(" to ") {
        let before = input[..index]
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");
        let after = input[index + 4..].trim().to_string();
        (clean_hint(Some(before)), clean_hint(Some(after)))
    } else {
        (None, None)
    }
}

fn value_target_from_markers(
    input: &str,
    verbs: &[&str],
    markers: &[&str],
) -> (Option<String>, Option<String>) {
    let lower = input.to_lowercase();
    let start = verbs
        .iter()
        .filter_map(|verb| {
            let index = lower.find(verb)?;
            Some(index + verb.len())
        })
        .min()
        .unwrap_or(0);
    let tail = input[start..]
        .trim()
        .trim_start_matches(|ch: char| ch == ':' || ch == '-' || ch.is_whitespace())
        .trim();
    if tail.is_empty() {
        return (None, None);
    }

    let tail_lower = tail.to_lowercase();
    for marker in markers {
        if let Some(index) = tail_lower.find(marker) {
            let raw_value = tail[..index].trim();
            let raw_target = tail[index + marker.len()..].trim();
            return (
                clean_hint(Some(raw_value.to_string())),
                clean_hint(Some(raw_target.to_string())),
            );
        }
    }

    (clean_hint(Some(tail.to_string())), None)
}

fn select_option_value_target(input: &str) -> Option<(String, String)> {
    let lower = input.to_lowercase();
    if !starts_with_any(&lower, &["select ", "choose ", "pick "]) {
        return None;
    }
    let (value, target) = value_target_from_markers(
        input,
        &["select", "choose", "pick"],
        &[" from ", " in ", " for "],
    );
    Some((value?, target?))
}

fn set_field_value_target(input: &str) -> Option<(String, String)> {
    let lower = input.to_lowercase();
    let verbs = ["set", "fill", "input", "write"];
    let (start_index, verb) = verbs
        .iter()
        .filter_map(|verb| {
            let prefix = format!("{verb} ");
            lower
                .find(&prefix)
                .map(|index| (index + prefix.len(), *verb))
        })
        .min_by_key(|(index, _)| *index)?;
    if verb == "set" {
        let has_explicit_set_target = contains_any(
            &lower,
            &[
                " field",
                " input",
                " textbox",
                " text box",
                " box",
                " row ",
                " card ",
                " item ",
                " record ",
                " entry ",
                " result ",
                " section ",
                " panel ",
            ],
        ) || has_short_assignment_target(input, start_index)
            || first_number(input)
                .as_deref()
                .map(|number| lower.contains(&format!(" to {number}")))
                .unwrap_or(false);
        if !has_explicit_set_target {
            return None;
        }
    }
    let tail = input[start_index..].trim();
    let tail_lower = tail.to_lowercase();
    for marker in [" to ", " with ", " as ", " = ", ":"] {
        if let Some(index) = tail_lower.find(marker) {
            let raw_target = tail[..index].trim();
            let raw_value = tail[index + marker.len()..].trim();
            let target = clean_hint(Some(raw_target.to_string()))?;
            let value = clean_hint(Some(raw_value.to_string()))?;
            return Some((value, target));
        }
    }
    None
}

fn has_short_assignment_target(input: &str, start_index: usize) -> bool {
    let tail = input[start_index..].trim();
    let tail_lower = tail.to_lowercase();
    for marker in [" to ", " with ", " as ", " = ", ":"] {
        if let Some(index) = tail_lower.find(marker) {
            let raw_target = tail[..index]
                .trim()
                .trim_start_matches(['"', '\'', '.', ',', ':', ';'])
                .trim();
            if raw_target.is_empty() {
                return false;
            }
            let word_count = raw_target.split_whitespace().count();
            return word_count <= 5
                && raw_target
                    .chars()
                    .any(|ch| ch.is_ascii_alphabetic() || ch.is_alphabetic());
        }
    }
    false
}

fn field_hint(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    for marker in [" into ", " in ", " to "] {
        if let Some(index) = lower.find(marker) {
            let hint = input[index + marker.len()..].trim();
            if !hint.is_empty() {
                return Some(hint.to_string());
            }
        }
    }
    None
}

fn clean_hint(value: Option<String>) -> Option<String> {
    value
        .map(|text| {
            strip_follow_up_suffix(text.trim())
                .trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '"' | '\'' | '.' | ',' | ':' | ';' | '(' | ')' | '[' | ']'
                    )
                })
                .trim()
                .to_string()
        })
        .filter(|text| !text.is_empty())
}

fn unresolved_reference_value(value: &str) -> bool {
    let normalized = value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ':' | ';'))
        .to_lowercase();
    matches!(
        normalized.as_str(),
        "it" | "this"
            | "that"
            | "them"
            | "the answer"
            | "answer"
            | "the result"
            | "result"
            | "the value"
            | "value"
            | "the text"
            | "text"
            | "the word"
            | "word"
            | "the last word"
            | "last word"
            | "the first word"
            | "first word"
    )
}

fn strip_follow_up_suffix(text: &str) -> &str {
    let lower = text.to_lowercase();
    for marker in [
        " and click ",
        " and press ",
        " and tap ",
        " and hit ",
        " and find and click ",
        " and find and press ",
        " and find and tap ",
        " and find and hit ",
        " then click ",
        " then press ",
        " then tap ",
        " then hit ",
        " then find and click ",
        " then find and press ",
        " then find and tap ",
        " then find and hit ",
    ] {
        if let Some(index) = lower.find(marker) {
            return &text[..index];
        }
    }
    text
}
