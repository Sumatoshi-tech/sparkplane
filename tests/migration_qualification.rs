#![cfg(feature = "appliance")]
use sparkplane::migration::qualification::{Evidence, compare, stream_sample};
use std::collections::BTreeMap;

#[test]
fn performance_gate_rejects_missing_nonfinite_or_regressed_samples() {
    let evidence = |samples| Evidence {
        decode_tokens_per_second: BTreeMap::from([("instance".into(), samples)]),
    };
    let before = evidence(vec![40.0, 41.0, 42.0]);
    assert!(compare(&before, &evidence(vec![41.0, 42.0, 43.0])).is_ok());
    for samples in [
        vec![],
        vec![40.0],
        vec![30.0, 31.0, 32.0],
        vec![40.0, f64::NAN, 42.0],
        vec![40.0, f64::INFINITY, 42.0],
    ] {
        assert!(compare(&before, &evidence(samples)).is_err());
    }
}

#[test]
fn truncated_stream_cannot_count_as_qualified_and_cancellation_requires_content() {
    let content = b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n";
    assert!(stream_sample(content.as_slice(), false).is_err());
    assert_eq!(stream_sample(content.as_slice(), true).unwrap(), 0.0);
    assert!(stream_sample(b"data: [DONE]\n\n".as_slice(), true).is_err());
    let complete = [content.as_slice(), b"data: {\"choices\":[{\"finish_reason\":\"length\"}],\"usage\":{\"completion_tokens\":16}}\n\ndata: [DONE]\n\n"].concat();
    assert!(
        stream_sample(complete.as_slice(), false)
            .unwrap()
            .is_finite()
    );
}
