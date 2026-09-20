#![cfg(feature = "appliance")]
use sparkplane::migration::{Transition, verify_transition};

#[test]
fn transition_requires_the_installed_authority_and_exact_host_and_release() {
    let old = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let new = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let plan = Transition {
        schema: "sparkplane.trust-transition/v1".into(),
        host_identity: "a".repeat(64),
        release_sha256: "b".repeat(64),
        new_public_key: new.pk.to_base64(),
        expires_at_unix_seconds: 200,
    };
    let bytes = serde_json::to_vec(&plan).unwrap();
    let signature = minisign::sign(None, &old.sk, std::io::Cursor::new(&bytes), None, None)
        .unwrap()
        .to_string();
    assert!(
        verify_transition(
            &bytes,
            &signature,
            &old.pk.to_base64(),
            &plan.host_identity,
            &plan.release_sha256,
            100
        )
        .is_ok()
    );
    assert!(
        verify_transition(
            &bytes,
            &signature,
            &new.pk.to_base64(),
            &plan.host_identity,
            &plan.release_sha256,
            100
        )
        .is_err()
    );
    assert!(
        verify_transition(
            &bytes,
            &signature,
            &old.pk.to_base64(),
            &"c".repeat(64),
            &plan.release_sha256,
            100
        )
        .is_err()
    );
    assert!(
        verify_transition(
            &bytes,
            &signature,
            &old.pk.to_base64(),
            &plan.host_identity,
            &"c".repeat(64),
            100
        )
        .is_err()
    );
    assert!(
        verify_transition(
            &bytes,
            &signature,
            &old.pk.to_base64(),
            &plan.host_identity,
            &plan.release_sha256,
            200
        )
        .is_err()
    );
}
