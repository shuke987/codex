#![cfg(not(target_os = "windows"))]
#![allow(clippy::unwrap_used)]

use core_test_support::responses;
use core_test_support::test_codex_exec::test_codex_exec;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;

fn tool_names(body: &Value) -> Vec<String> {
    body.get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    tool.get("name")
                        .or_else(|| tool.get("type"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn tool_is_exposed(body: &Value, tool_name: &str) -> bool {
    if tool_names(body).iter().any(|name| name == tool_name) {
        return true;
    }

    let nested_tool_heading = format!("### `{tool_name}`");
    body.get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("additional_tools"))
        .filter_map(|item| item.get("tools").and_then(Value::as_array))
        .flatten()
        .filter_map(|tool| tool.get("description").and_then(Value::as_str))
        .any(|description| description.contains(&nested_tool_heading))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_flag_starts_goal_mode_and_waits_for_completion() -> anyhow::Result<()> {
    let test = test_codex_exec();
    let server = responses::start_mock_server().await;
    let call_id = "complete-goal";
    let first_response = responses::sse(vec![
        responses::ev_response_created("resp-goal-1"),
        responses::ev_function_call(call_id, "update_goal", r#"{"status":"complete"}"#),
        responses::ev_completed("resp-goal-1"),
    ]);
    let second_response = responses::sse(vec![
        responses::ev_response_created("resp-goal-2"),
        responses::ev_assistant_message("msg-goal", "goal complete"),
        responses::ev_completed("resp-goal-2"),
    ]);
    let response_mock =
        responses::mount_sse_sequence(&server, vec![first_response, second_response]).await;

    let objective = "finish the goal-mode exec task";
    test.cmd_with_server(&server)
        .arg("--skip-git-repo-check")
        .arg("--goal")
        .arg(objective)
        .assert()
        .success();

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 2);

    let first_request = requests[0].body_json();
    assert!(
        first_request.to_string().contains(objective),
        "goal objective should be injected into the continuation context"
    );
    assert!(
        tool_is_exposed(&first_request, "update_goal"),
        "active goal turns should expose update_goal"
    );

    let tool_output = requests[1]
        .function_call_output_text(call_id)
        .expect("second request should include update_goal output");
    assert!(
        tool_output.contains("\"status\":\"complete\""),
        "update_goal output should mark the goal complete"
    );

    Ok(())
}

fn capacity_response() -> String {
    responses::sse(vec![serde_json::json!({
        "type": "response.failed",
        "response": {
            "id": "capacity-response",
            "error": {
                "code": "server_is_overloaded",
                "message": "Selected model is at capacity. Please try a different model."
            }
        }
    })])
}

fn json_events(output: &[u8]) -> Vec<Value> {
    std::str::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_resume_after_capacity_completes_the_same_session() -> anyhow::Result<()> {
    let test = test_codex_exec();
    let server = responses::start_mock_server().await;
    let resumed_response = responses::sse(vec![
        responses::ev_response_created("resumed-goal"),
        responses::ev_function_call(
            "complete-resumed-goal",
            "update_goal",
            r#"{"status":"complete"}"#,
        ),
        responses::ev_completed("resumed-goal"),
    ]);
    let final_response = responses::sse(vec![
        responses::ev_response_created("resumed-final"),
        responses::ev_assistant_message("resumed-message", "review recovered"),
        responses::ev_completed("resumed-final"),
    ]);
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![capacity_response(), resumed_response, final_response],
    )
    .await;

    let first = test
        .cmd_with_server(&server)
        .args([
            "--skip-git-repo-check",
            "--json",
            "--goal",
            "finish the review",
        ])
        .timeout(Duration::from_secs(30))
        .assert()
        .failure()
        .get_output()
        .clone();
    let first_events = json_events(&first.stdout);
    let thread_id = first_events[0]["thread_id"].as_str().unwrap();
    assert!(
        first_events
            .iter()
            .any(|event| event["type"] == "turn.failed")
    );

    let resumed = test
        .cmd_with_server(&server)
        .args([
            "--skip-git-repo-check",
            "--json",
            "--goal",
            "resume",
            thread_id,
            "continue the review",
        ])
        .timeout(Duration::from_secs(30))
        .assert()
        .success()
        .get_output()
        .clone();
    let resumed_events = json_events(&resumed.stdout);
    assert_eq!(resumed_events[0], first_events[0]);
    assert!(
        resumed_events
            .iter()
            .any(|event| event["type"] == "turn.started")
    );
    assert_eq!(resumed_events.last().unwrap()["type"], "turn.completed");
    assert_eq!(response_mock.requests().len(), 3);
    assert!(String::from_utf8(resumed.stderr)?.contains("Goal activation observed:"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_resume_reports_a_new_capacity_failure() -> anyhow::Result<()> {
    let test = test_codex_exec();
    let server = responses::start_mock_server().await;
    let response_mock =
        responses::mount_sse_sequence(&server, vec![capacity_response(), capacity_response()])
            .await;
    let first = test
        .cmd_with_server(&server)
        .args([
            "--skip-git-repo-check",
            "--json",
            "--goal",
            "finish the review",
        ])
        .timeout(Duration::from_secs(30))
        .assert()
        .failure()
        .get_output()
        .clone();
    let first_events = json_events(&first.stdout);
    let thread_id = first_events[0]["thread_id"].as_str().unwrap();
    let resumed = test
        .cmd_with_server(&server)
        .args([
            "--skip-git-repo-check",
            "--json",
            "--goal",
            "resume",
            thread_id,
            "continue the review",
        ])
        .timeout(Duration::from_secs(30))
        .assert()
        .failure()
        .get_output()
        .clone();
    let resumed_events = json_events(&resumed.stdout);
    assert_eq!(resumed_events[0], first_events[0]);
    assert_eq!(resumed_events.last().unwrap()["type"], "turn.failed");
    assert_eq!(response_mock.requests().len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_model_stream_disconnect_is_a_failed_turn() -> anyhow::Result<()> {
    let test = test_codex_exec();
    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_response_created("truncated-goal")]),
    )
    .await;
    let provider = format!(
        "model_providers.fault={{name='fault',base_url='{}/v1',wire_api='responses',stream_max_retries=0,request_max_retries=0}}",
        server.uri()
    );
    let output = test
        .cmd_with_server(&server)
        .args([
            "--skip-git-repo-check",
            "--json",
            "--goal",
            "-c",
            "model_provider='fault'",
            "-c",
            &provider,
            "finish the review",
        ])
        .timeout(Duration::from_secs(30))
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert_eq!(
        json_events(&output.stdout).last().unwrap()["type"],
        "turn.failed"
    );
    assert_eq!(response_mock.requests().len(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_interrupt_during_a_model_request_exits_with_failure() -> anyhow::Result<()> {
    let test = test_codex_exec();
    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_response_once(
        &server,
        responses::sse_response(responses::sse(vec![responses::ev_completed("too-late")]))
            .set_delay(Duration::from_secs(30)),
    )
    .await;
    let base_url = serde_json::to_string(&format!("{}/v1", server.uri()))?;
    let child = tokio::process::Command::new(codex_utils_cargo_bin::cargo_bin("codex-exec")?)
        .current_dir(test.cwd_path())
        .env("CODEX_HOME", test.home_path())
        .env("CODEX_SQLITE_HOME", test.home_path())
        .env(codex_login::CODEX_API_KEY_ENV_VAR, "dummy")
        .args([
            "--skip-git-repo-check",
            "--json",
            "--goal",
            "-c",
            &format!("openai_base_url={base_url}"),
            "wait for the review to finish",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::timeout(Duration::from_secs(10), async {
        while response_mock.requests().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let pid = i32::try_from(child.id().unwrap())?;
    // SAFETY: this is the live child owned by the test; no pointer is involved.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGINT) }, 0);
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output()).await??;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(response_mock.requests().len(), 1);
    Ok(())
}
