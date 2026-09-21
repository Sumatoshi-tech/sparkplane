#![cfg(feature = "appliance")]
use sparkplane::migration::commands::Action;

#[test]
fn privileged_actions_never_restart_docker_or_run_caller_selected_programs() {
    for action in Action::ALL {
        let (program, args) = action.command();
        assert!(program.starts_with('/'));
        assert!(!args.contains(&"docker.service"));
        assert!(!["/bin/sh", "/bin/bash", "/usr/bin/env"].contains(&program));
    }
}

#[test]
fn traffic_fence_preserves_established_requests_and_allows_local_qualification() {
    let rules = sparkplane::migration::commands::FENCE;
    assert!(rules.contains("iifname != \"lo\" tcp dport 9843 ct state new reject"));
    assert!(!rules.contains("flush ruleset"));
}
