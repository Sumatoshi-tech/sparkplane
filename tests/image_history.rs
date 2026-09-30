use serde_json::{Value, json};
use sparkplane::spark::image_history::{Protocol, project, project_reader};

fn frame(i: usize) -> Value {
    json!({"type":"function_call_output","call_id":format!("call_{i}"),"output":[
        {"type":"input_text","text":format!("screenshot {i}")},
        {"type":"input_image","image_url":format!("data:image/png;base64,aW1hZ2U{i:04}")}
    ]})
}
fn images(value: &Value) -> usize {
    match value {
        Value::Array(a) => a.iter().map(images).sum(),
        Value::Object(o) => {
            usize::from(value["type"] == "input_image" || value["type"] == "image")
                + o.values().map(images).sum::<usize>()
        }
        _ => 0,
    }
}

#[test]
fn five_hundred_views_keep_new_images_and_tool_pairing() {
    let mut input = vec![];
    for i in 0..500 {
        input.push(json!({"type":"function_call","call_id":format!("call_{i}"),"name":"view_image","arguments":format!("{{\"path\":\"/tmp/frame-{i}.png\"}}") }));
        input.push(frame(i));
        if i != 499 {
            input.push(json!({"type":"message","role":"assistant","content":format!("Observation {i}: button visible")}));
        }
    }
    let original = json!({"model":"model","input":input,"store":false});
    let mut value = original.clone();
    let stats = project(&mut value, Protocol::Responses, 16, None).unwrap();
    assert!(images(&value) <= 12);
    assert!(stats.evicted > 480);
    assert_eq!(value["input"].as_array().unwrap().len(), 1499);
    assert_eq!(value["input"][1]["call_id"], "call_0");
    assert_eq!(value["input"][1]["output"][0]["text"], "screenshot 0");
    assert!(
        value["input"][1]["output"][1]["text"]
            .as_str()
            .unwrap()
            .contains("/tmp/frame-0.png")
    );
    assert_eq!(value["input"][1498], original["input"][1498]);
    assert_eq!(value["input"][2], original["input"][2]);
    let projected = project_reader(
        std::io::Cursor::new(serde_json::to_vec(&original).unwrap()),
        Protocol::Responses,
        16,
        None,
    )
    .unwrap();
    assert_eq!(projected.0, value);
    let repeated = project(&mut value, Protocol::Responses, 16, None).unwrap();
    assert_eq!(repeated.evicted, 0);
}

#[test]
fn unseen_batch_is_never_silently_discarded() {
    let mut value = json!({"input":(0..17).map(frame).collect::<Vec<_>>()});
    let before = value.clone();
    assert!(project(&mut value, Protocol::Responses, 16, None).is_err());
    assert_eq!(value, before);
    let mut value = json!({"input":(0..16).map(frame).collect::<Vec<_>>()});
    project(&mut value, Protocol::Responses, 16, None).unwrap();
    assert_eq!(images(&value), 16);
}

#[test]
fn anthropic_nested_results_and_current_comparison_survive() {
    let mut messages = vec![];
    for i in 0..20 {
        messages.push(json!({"role":"assistant","content":[{"type":"tool_use","id":format!("t{i}"),"name":"screenshot","input":{}}]}));
        messages.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":format!("t{i}"),"content":[{"type":"text","text":"observed"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"aW1hZ2U="}}]}]}));
    }
    messages.push(json!({"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"Y29tcGFyZTE="}},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"Y29tcGFyZTI="}}]}));
    let mut value = json!({"messages":messages,"system":"preserve instructions"});
    let last = value["messages"][40].clone();
    project(&mut value, Protocol::Anthropic, 16, None).unwrap();
    assert!(images(&value) <= 12);
    assert_eq!(value["messages"][40], last);
    assert_eq!(value["messages"][1]["content"][0]["tool_use_id"], "t0");
    assert_eq!(value["system"], "preserve instructions");
}

#[test]
fn byte_pressure_evicts_processed_images_before_the_count_threshold() {
    let image = format!("data:image/png;base64,{}", "a".repeat(6 * 1024 * 1024));
    let mut input = Vec::new();
    for index in 0..6 {
        input.push(json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("Frame {index}")},{"type":"input_image","image_url":image}]}));
        if index < 5 {
            input.push(json!({"type":"message","role":"assistant","content":"Observed frame."}));
        }
    }
    let mut request = json!({"input":input});
    let newest = request["input"][10].clone();
    let stats = project(&mut request, Protocol::Responses, 16, None).unwrap();
    assert_eq!(stats.evicted, 4);
    assert_eq!(images(&request), 2);
    assert_eq!(request["input"][10], newest);
    assert!(serde_json::to_vec(&request).unwrap().len() < 16 * 1024 * 1024);
}
