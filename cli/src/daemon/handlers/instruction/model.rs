use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InstructionKind {
    Click,
    Fill,
    SelectOption,
    SetChecked,
    Count,
    Drag,
    Scroll,
    Unknown,
}

impl InstructionKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Fill => "fill",
            Self::SelectOption => "select_option",
            Self::SetChecked => "set_checked",
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
