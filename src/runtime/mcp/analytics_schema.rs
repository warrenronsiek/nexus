// @feature runtime
// @feature usage-analytics
// @spec docs/features/runtime.md
// @spec docs/features/usage-analytics.md
// @boundary static-mcp-schema
use serde_json::{json, Value};
pub(super) fn definition() -> Value {
    json!({"name":"nexus_analytics","description":"Inspect observed activities, concrete capabilities including unused inventoried tools, and recurring ad-hoc work. Inventory and classification reports are observational; summaries retain coverage and counting semantics.","inputSchema":{
        "type":"object","required":["action"],"properties":{
            "action":{"type":"string","enum":["summary","catalog","opportunities","categories","examples","inventory","reserve_classification","classify","classification_failed","rename_category","merge_category"]},
            "project_id":{"type":"string","maxLength":256},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":64},
            "category_id":{"type":"string","maxLength":80},"resource_key":{"type":"string","maxLength":1024},
            "session_id":{"type":"string","maxLength":512},"agent":{"type":"string","maxLength":256},"project_root":{"type":"string","maxLength":4096},"turn_id":{"type":"string","maxLength":512},"model":{"type":"string","maxLength":128},
            "scope":{"type":"string","maxLength":80},"complete":{"type":"boolean"},"resources":{"type":"array","maxItems":256,"items":resource_schema()},
            "target":target_schema(),"draft":draft_schema(),
            "id":{"type":"string","maxLength":80},"name":{"type":"string","maxLength":96},"definition":{"type":"string","maxLength":384},"source":{"type":"string","maxLength":80},"classifier_model":{"type":"string","maxLength":128}
        },"additionalProperties":false
    }})
}
fn resource_schema() -> Value {
    json!({"type":"object","required":["name","kind","origin","exposure"],"properties":{
        "name":{"type":"string","maxLength":512},"kind":{"type":"string","enum":["tool","script","cli","inline","skill"]},
        "origin":{"type":"string","enum":["host_tool","repository","standard_cli","agent_authored","unknown","internal"]},
        "exposure":{"type":"string","enum":["declared","discoverable","exposed"]},"description":{"type":"string","maxLength":512},"activities":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":80}},"version":{"type":["string","null"],"maxLength":128}
    },"additionalProperties":false})
}

fn target_schema() -> Value {
    let pattern = json!({"type":"string","pattern":"^[a-f0-9]{64}$"});
    json!({"description":"Companion classification subject, or target category id when merging.","oneOf":[
        {"type":"string","maxLength":80},
        {"type":"object","required":["kind","invocation_id","index","pattern_key"],"properties":{"kind":{"const":"action"},"invocation_id":{"type":"string","maxLength":512},"index":{"type":"integer","minimum":0,"maximum":63},"pattern_key":pattern},"additionalProperties":false},
        {"type":"object","required":["kind","name","resource_kind","pattern_key"],"properties":{"kind":{"const":"resource"},"name":{"type":"string","maxLength":512},"resource_kind":{"type":"string","enum":["tool","script","cli","inline","skill"]},"pattern_key":pattern},"additionalProperties":false}
    ]})
}
fn draft_schema() -> Value {
    json!({"type":"object","required":["category_id","name","definition","description"],"properties":{
        "category_id":{"type":"string","maxLength":80},"name":{"type":"string","maxLength":96},"definition":{"type":"string","maxLength":384},"description":{"type":"string","maxLength":256}
    },"additionalProperties":false})
}
