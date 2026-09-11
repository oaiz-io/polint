use crate::go::semantic::protocol::decode_ndjson_str;

#[test]
fn semantic_protocol_accepts_sidecar_shape() {
    let output = decode_ndjson_str(
        r#"{"schema":"polint-go-semantic-3","kind":"session_begin","go_version":"go1.25.0","x_tools_version":"v0.45.0"}
{"schema":"polint-go-semantic-3","kind":"package","package_id":"example.test/pkg","package_path":"example.test/pkg"}
{"schema":"polint-go-semantic-3","kind":"session_end"}"#,
    )
    .expect("protocol decodes");
    assert_eq!(output.rows.len(), 1);
}
