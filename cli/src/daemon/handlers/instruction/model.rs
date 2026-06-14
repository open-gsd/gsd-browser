use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InstructionKind {
    Click,
    Focus,
    Hover,
    Fill,
    ClearField,
    AppendField,
    SelectOption,
    SetChecked,
    UploadFile,
    PressKey,
    Wait,
    Navigate,
    SetViewport,
    EmulateDevice,
    ReadText,
    AnalyzeForm,
    AccessibilityTree,
    FindElements,
    Assert,
    Screenshot,
    RenderPattern,
    Count,
    Drag,
    Scroll,
    Unknown,
}

impl InstructionKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Focus => "focus",
            Self::Hover => "hover",
            Self::Fill => "fill",
            Self::ClearField => "clear_field",
            Self::AppendField => "append_field",
            Self::SelectOption => "select_option",
            Self::SetChecked => "set_checked",
            Self::UploadFile => "upload_file",
            Self::PressKey => "press_key",
            Self::Wait => "wait",
            Self::Navigate => "navigate",
            Self::SetViewport => "set_viewport",
            Self::EmulateDevice => "emulate_device",
            Self::ReadText => "read_text",
            Self::AnalyzeForm => "analyze_form",
            Self::AccessibilityTree => "accessibility_tree",
            Self::FindElements => "find_elements",
            Self::Assert => "assert",
            Self::Screenshot => "screenshot",
            Self::RenderPattern => "render_pattern",
            Self::Count => "count",
            Self::Drag => "drag",
            Self::Scroll => "scroll",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InstructionAnalysis {
    pub(super) kind: InstructionKind,
    pub(super) value: Option<String>,
    pub(super) target_hint: Option<String>,
    pub(super) secondary_hint: Option<String>,
    pub(super) checked: Option<bool>,
    pub(super) direction: Option<String>,
}

impl InstructionAnalysis {
    pub(super) fn to_json(&self) -> Value {
        json!({
            "kind": self.kind.as_str(),
            "value": self.value,
            "targetHint": self.target_hint,
            "secondaryHint": self.secondary_hint,
            "checked": self.checked,
            "direction": self.direction,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InstructionIntent {
    pub(super) action_verbs: Vec<String>,
    pub(super) ordered_click_hints: Vec<String>,
    pub(super) menu_path: Vec<String>,
    pub(super) order: Option<String>,
    pub(super) wants_ordered_values: bool,
    pub(super) wants_numeric_targets: bool,
    pub(super) follow_up_click_hint: Option<String>,
}

impl InstructionIntent {
    pub(super) fn to_json(&self) -> Value {
        json!({
            "actionVerbs": self.action_verbs,
            "orderedClickHints": self.ordered_click_hints,
            "menuPath": self.menu_path,
            "order": self.order,
            "wantsOrderedValues": self.wants_ordered_values,
            "wantsNumericTargets": self.wants_numeric_targets,
            "followUpClickHint": self.follow_up_click_hint,
        })
    }
}

pub(super) fn json_literal<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}
