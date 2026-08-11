use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use aoe_agent::{AgentAdapter, AgentInvocation, AgentStatus, CommandAdapter};
use aoe_domain::{AgentConfig, Budget};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "agents-of-empires-adapter-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn invocation() -> AgentInvocation {
    AgentInvocation {
        config: AgentConfig {
            id: "test-agent".into(),
            territory: "test-territory".into(),
            adapter: "shell".into(),
            model: "fake/model".into(),
            reasoning_effort: "low".into(),
            budget: Budget {
                resource_units: 10,
                max_cost_microusd: None,
            },
        },
        territory_host: "127.0.0.1".into(),
        ssh_port: 22000,
        instruction: "hold the line".into(),
        credential_file: None,
    }
}

#[tokio::test]
async fn command_adapter_reads_normalized_result() {
    let root = temp_root();
    fs::create_dir_all(&root).expect("temp root");
    let script = root.join("adapter.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
set -eu
printf '%s' '{"schema_version":1,"agent":"test-agent","territory":"test-territory","status":"completed","summary":"held","usage":{"resource_units":3},"transcript":null}' > "$AOE_RESULT_FILE"
"#,
    )
    .expect("script");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
    }
    fs::set_permissions(&script, permissions).expect("permissions");
    let adapter = CommandAdapter::new(&script, root.join("runs"));
    let result = adapter
        .run(invocation(), Duration::from_secs(2))
        .await
        .expect("adapter result");
    assert_eq!(result.status, AgentStatus::Completed);
    assert_eq!(result.usage.resource_units, 3);
    assert_eq!(
        fs::read_to_string(root.join("runs/test-agent/instruction.md")).expect("instruction"),
        "hold the line"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn timeout_preserves_partial_result() {
    let root = temp_root();
    fs::create_dir_all(&root).expect("temp root");
    let script = root.join("adapter.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
set -eu
printf '%s' '{"schema_version":1,"agent":"test-agent","territory":"test-territory","status":"failed","summary":"partial","usage":{"rounds":4,"resource_units":4},"transcript":null}' > "$AOE_RESULT_FILE"
sleep 5
"#,
    )
    .expect("script");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
    }
    fs::set_permissions(&script, permissions).expect("permissions");
    let adapter = CommandAdapter::new(&script, root.join("runs"));
    let result = adapter
        .run(invocation(), Duration::from_millis(50))
        .await
        .expect("partial result");
    assert_eq!(result.status, AgentStatus::TimedOut);
    assert_eq!(result.usage.rounds, Some(4));
    fs::remove_dir_all(root).expect("cleanup");
}
