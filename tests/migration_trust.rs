#![cfg(feature = "appliance")]
use sparkplane::migration::{Transition, verify_transition};

#[test]
fn recovery_approval_binds_executable_record_host_expiry_and_installed_key() {
    let key = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let bytes = serde_json::to_vec(&serde_json::json!({"schema":"sparkplane.recovery-approval/v1", "host":"a".repeat(64), "record_sha256":"b".repeat(64), "executable_sha256":"c".repeat(64), "expires_at_unix_seconds":200, "replacements":[]})).unwrap();
    let signature = minisign::sign(None, &key.sk, std::io::Cursor::new(&bytes), None, None)
        .unwrap()
        .to_string();
    let wrong = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    assert!(
        sparkplane::migration::recovery::Approval::verify(
            &bytes,
            &signature,
            &wrong.pk.to_base64(),
            [&"a".repeat(64), &"b".repeat(64), &"c".repeat(64)],
            100
        )
        .is_err()
    );
    let verify = |host: &str, record: &str, exe: &str, now| {
        sparkplane::migration::recovery::Approval::verify(
            &bytes,
            &signature,
            &key.pk.to_base64(),
            [host, record, exe],
            now,
        )
    };
    assert!(verify(&"a".repeat(64), &"b".repeat(64), &"c".repeat(64), 100).is_ok());
    for (host, record, exe, now) in [
        ('d', 'b', 'c', 100),
        ('a', 'd', 'c', 100),
        ('a', 'b', 'd', 100),
        ('a', 'b', 'c', 200),
    ] {
        assert!(
            verify(
                &host.to_string().repeat(64),
                &record.to_string().repeat(64),
                &exe.to_string().repeat(64),
                now
            )
            .is_err()
        );
    }
}

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
