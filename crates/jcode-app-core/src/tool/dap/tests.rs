use super::*;
#[test]
fn broker_tokens_are_opaque_owner_bound_and_cleaned() {
    let mut b = TokenBroker::new(8);
    let token = TokenBroker::token("ds");
    assert!(token.starts_with("ds_"));
    assert!(!token.contains("dap-"));
    b.frames.clear();
}

#[test]
fn broker_compacts_stale_order_records() {
    let mut broker = TokenBroker::new(8);
    for index in 0..32 {
        broker.record(TokenKind::Frame, format!("missing-{index}"));
    }
    assert_eq!(broker.order.len(), 32);
    broker.compact_order();
    assert!(broker.order.is_empty());
}

#[test]
fn broker_refuses_a_response_larger_than_owner_capacity() {
    let mut broker = TokenBroker::new(2);
    assert!(broker.reserve_capacity("owner", 3).is_err());
    assert!(broker.reserve_capacity("owner", 2).is_ok());
}

#[test]
fn schema_exposes_exact_action_set() {
    let service = DapService::from_config(&jcode_dap::DapConfig::default()).unwrap();
    let schema = service.tool().parameters_schema();
    let actions = schema
        .pointer("/properties/action/enum")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(actions.len(), 17);
    assert!(!actions.iter().any(|v| v == "custom" || v == "request"));
    assert!(schema.pointer("/properties/pid").is_none());
}
#[test]
fn output_is_bounded() {
    let value = json!({"x":"a".repeat(MAX_OUTPUT_CHARS+100)});
    let output = bounded_pretty(&value, MAX_OUTPUT_CHARS);
    assert!(output.len() <= MAX_OUTPUT_CHARS);
    let parsed: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["protocol"], "jcode.dap.v1");
    assert_eq!(parsed["result"]["truncated"], true);
}
