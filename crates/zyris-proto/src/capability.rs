use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transfer {
    Unary,
    UniStream,
    BiStream,
    Video,
}

/// How long one call of a tool may take to answer.
///
/// A caller cannot know this from the schema. Some tools return in milliseconds and some build a
/// repository, and the only party that knows which is the node declaring the tool — so it says so
/// here rather than leaving every caller to pick one number for all of them.
///
/// This is a declaration, not a promise: it says how long the caller should be willing to wait,
/// never how long the work will take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallLimit {
    /// Wait at most this many seconds for the answer.
    Secs(u32),
    /// Wait for as long as the connection lives.
    ///
    /// Not "forever": the heartbeat still applies, so a node that stops answering has its socket
    /// torn down and the pending call fails with it. What this removes is the clock on a node that
    /// is demonstrably alive and still working — a build, a deploy, a long test run.
    ///
    /// The cost is that a node whose handler deadlocks while its heartbeat keeps running holds the
    /// caller until the connection ends. A tool asking for this is asking to be trusted with that.
    Unlimited,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub transfer: Transfer,
    pub request_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_schema: Option<serde_json::Value>,
    /// What this tool asks of a caller's clock. Absent means the caller's own default, which is
    /// what every tool declared before this field existed — so an older node keeps the behaviour
    /// it has always had, and a newer node talking to an older caller is simply not heard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_limit: Option<CallLimit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub name: String,
    pub version: u32,
    pub tools: Vec<ToolDescriptor>,
}

impl CapabilityDescriptor {
    pub fn tool(&self, name: &str) -> Option<&ToolDescriptor> {
        self.tools.iter().find(|t| t.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnounceParams {
    pub capabilities: Vec<CapabilityDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedCapability {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AnnounceResult {
    pub accepted: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<RejectedCapability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosingParams {
    pub reason: String,
}

pub fn method_name(capability: &str, tool: &str) -> String {
    format!("{capability}.{tool}")
}

pub fn split_method(method: &str) -> Option<(&str, &str)> {
    method.split_once('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(call_limit: Option<CallLimit>) -> ToolDescriptor {
        ToolDescriptor {
            name: "build".to_string(),
            description: "Build the workspace.".to_string(),
            transfer: Transfer::Unary,
            request_schema: serde_json::json!({ "type": "object" }),
            response_schema: None,
            item_schema: None,
            call_limit,
        }
    }

    /// The whole point of the field being optional: a node built before it existed announces
    /// nothing, and must still deserialize into a descriptor that asks for nothing.
    #[test]
    fn a_descriptor_from_before_this_field_existed_still_reads() {
        let old = serde_json::json!({
            "name": "build",
            "description": "Build the workspace.",
            "transfer": "unary",
            "request_schema": { "type": "object" }
        });
        let parsed: ToolDescriptor = serde_json::from_value(old).expect("an older descriptor reads");
        assert_eq!(parsed.call_limit, None, "silence asks for the caller's own default");
    }

    /// And the reverse direction: a tool that asks for nothing must not start putting a null on
    /// the wire, or every announce grows a field that older peers have to be lenient about.
    #[test]
    fn asking_for_nothing_puts_nothing_on_the_wire() {
        let json = serde_json::to_value(descriptor(None)).unwrap();
        assert!(
            json.get("call_limit").is_none(),
            "an unset limit is absent, not null: {json}"
        );
    }

    /// The two shapes a caller has to tell apart, pinned as text. `Unlimited` is a bare string
    /// rather than a number so there is no sentinel to mistake for a duration — `0` would be both
    /// "no time at all" and "no limit" depending on who read it.
    #[test]
    fn the_two_answers_are_distinguishable_on_the_wire() {
        let secs = serde_json::to_value(descriptor(Some(CallLimit::Secs(600)))).unwrap();
        assert_eq!(secs["call_limit"], serde_json::json!({ "secs": 600 }));

        let forever = serde_json::to_value(descriptor(Some(CallLimit::Unlimited))).unwrap();
        assert_eq!(forever["call_limit"], serde_json::json!("unlimited"));

        for limit in [CallLimit::Secs(600), CallLimit::Unlimited] {
            let round_tripped: ToolDescriptor =
                serde_json::from_value(serde_json::to_value(descriptor(Some(limit))).unwrap())
                    .unwrap();
            assert_eq!(round_tripped.call_limit, Some(limit));
        }
    }
}
