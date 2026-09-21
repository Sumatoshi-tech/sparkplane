#![cfg(feature = "appliance")]
use sparkplane::migration::{
    container::{LegacyContainer, Namespace, Network},
    docker::Containers,
};
use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
};

#[test]
fn wrong_docker_identity_never_receives_a_stop_or_delete() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("docker.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            connection.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let body = r#"{"Id":"wrong","Name":"unrelated","Image":"wrong","State":{"Running":true},"Config":{"Labels":{}}}"#;
        write!(connection, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        String::from_utf8(request).unwrap()
    });
    let expected = identity();
    let docker = Containers::connect(socket.to_str().unwrap()).unwrap();
    assert!(docker.remove_exact(&expected, Namespace::Legacy).is_err());
    let request = server.join().unwrap();
    assert!(
        request.starts_with("GET ")
            && request.contains(&format!("/containers/{}/json", expected.container_id)),
        "{request:?}"
    );
}

fn identity() -> LegacyContainer {
    LegacyContainer {
        container_id: "a".repeat(64),
        instance_id: format!("i_{}", "b".repeat(32)),
        generation: 8,
        image_digest: format!("sha256:{}", "c".repeat(64)),
        engine_id: "engine".into(),
        engine_fingerprint: format!("sha256:{}", "d".repeat(64)),
        artifact_fingerprint: format!("sha256:{}", "e".repeat(64)),
        model_repository: "owner/model".into(),
        model_commit: "f".repeat(40),
    }
}

fn inspected(running: bool) -> String {
    let expected = identity();
    serde_json::json!({"Id":expected.container_id,"Name":format!("/sy-spark-{}-g8",expected.instance_id),
        "Image":expected.image_digest,"Config":{"Labels":expected.labels()},"State":{"Running":running},"HostConfig":{"RestartPolicy":{"Name":"unless-stopped"}}}).to_string()
}

#[test]
fn recovery_rejects_restart_policy_drift_before_any_start() {
    let changed = inspected(true).replace("unless-stopped", "always");
    let requests = exercise(vec![(200, changed)], false, |docker| {
        docker.restore_legacy(&identity())
    });
    assert_eq!(requests.len(), 1);
}

fn cleanup(responses: Vec<(u16, String)>, succeeds: bool) -> Vec<String> {
    exercise(responses, succeeds, |docker| {
        docker.remove_exact(&identity(), Namespace::Legacy)
    })
}

fn exercise(
    responses: Vec<(u16, String)>,
    succeeds: bool,
    operation: impl FnOnce(&Containers) -> anyhow::Result<()>,
) -> Vec<String> {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("docker.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut connection = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "missing Docker request"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            connection
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                connection.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            requests.push(String::from_utf8(request).unwrap());
            write!(connection, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    let docker = Containers::connect(socket.to_str().unwrap()).unwrap();
    assert_eq!(operation(&docker).is_ok(), succeeds);
    server.join().unwrap()
}

#[test]
fn cleanup_rechecks_ownership_and_observes_absence_after_nonforced_exact_delete() {
    let requests = cleanup(
        vec![
            (200, inspected(true)),
            (204, String::new()),
            (200, inspected(false)),
            (200, inspected(false)),
            (204, String::new()),
            (404, r#"{"message":"absent"}"#.into()),
        ],
        true,
    );
    assert_eq!(
        requests
            .iter()
            .map(|r| r.split_whitespace().next().unwrap())
            .collect::<Vec<_>>(),
        ["GET", "POST", "GET", "GET", "DELETE", "GET"]
    );
    assert!(
        requests
            .iter()
            .all(|r| r.contains(&format!("/containers/{}", identity().container_id)))
    );
    assert!(requests[1].contains("/stop?t=30"));
    assert!(requests[4].contains("force=false"));
    assert!(requests[4].contains("v=false"));
}

#[test]
fn post_stop_identity_drift_prevents_deletion() {
    let changed = inspected(false).replace("/sy-spark-", "/unrelated-");
    let requests = cleanup(
        vec![(200, inspected(true)), (204, String::new()), (200, changed)],
        false,
    );
    assert!(!requests.iter().any(|r| r.starts_with("DELETE")));
}

fn network() -> serde_json::Value {
    serde_json::json!({"Id":"a".repeat(64),"Name":"sy-spark-internal","Driver":"bridge","Internal":true,
        "Labels":{"io.sy.spark.managed":"true","io.sy.spark.role":"network"},"Containers":{}})
}

#[test]
fn network_cleanup_is_exact_and_requires_owned_empty_internal_bridge() {
    let expected = Network::capture(&network(), Namespace::Legacy).unwrap();
    let requests = exercise(
        vec![
            (200, network().to_string()),
            (204, String::new()),
            (404, r#"{"message":"absent"}"#.into()),
        ],
        true,
        |docker| docker.remove_network(&expected, Namespace::Legacy),
    );
    assert!(
        requests
            .iter()
            .all(|r| r.contains(&format!("/networks/{}", expected.id)))
    );
    assert!(requests[1].starts_with("DELETE "));
    for change in ["attached", "owner"] {
        let mut value = network();
        if change == "attached" {
            value["Containers"] = serde_json::json!({"unrelated":{"Name":"other"}});
        } else {
            value["Labels"]["io.sy.spark.managed"] = "false".into();
        }
        let requests = exercise(vec![(200, value.to_string())], false, |docker| {
            docker.remove_network(&expected, Namespace::Legacy)
        });
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET "));
    }
}
