#![cfg(not(target_os = "windows"))]
#![allow(clippy::unwrap_used)]

use core_test_support::responses;
use core_test_support::test_codex_exec::test_codex_exec;
use pretty_assertions::assert_eq;
use serde_json::Value;

fn tool_is_exposed(body: &Value, tool_name: &str) -> bool {
    let nested_tool_heading = format!("### `{tool_name}`");
    let additional_tools = body
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("additional_tools"));

    std::iter::once(body)
        .chain(additional_tools)
        .filter_map(|item| item.get("tools").and_then(Value::as_array))
        .flatten()
        .flat_map(|tool| {
            std::iter::once(tool).chain(
                tool.get("tools")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            )
        })
        .any(|tool| {
            tool.get("name").and_then(Value::as_str) == Some(tool_name)
                || tool
                    .get("description")
                    .and_then(Value::as_str)
                    .is_some_and(|description| description.contains(&nested_tool_heading))
        })
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_flag_waits_across_automatic_continuation_turns() -> anyhow::Result<()> {
    let test = test_codex_exec();
    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("resp-progress"),
                responses::ev_assistant_message("msg-progress", "More work remains."),
                responses::ev_completed("resp-progress"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("resp-complete"),
                responses::ev_function_call(
                    "complete-goal",
                    "update_goal",
                    r#"{"status":"complete"}"#,
                ),
                responses::ev_completed("resp-complete"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("resp-final"),
                responses::ev_assistant_message("msg-final", "goal complete"),
                responses::ev_completed("resp-final"),
            ]),
        ],
    )
    .await;

    let assert = test
        .cmd_with_server(&server)
        .args([
            "--skip-git-repo-check",
            "--goal",
            "--json",
            "finish the multi-turn task",
        ])
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .success();
    let events: Vec<Value> = String::from_utf8_lossy(&assert.get_output().stdout)
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let event_types: Vec<&str> = events
        .iter()
        .filter_map(|event| event["type"].as_str())
        .filter(|kind| kind.starts_with("turn."))
        .collect();
    assert_eq!(
        event_types,
        vec![
            "turn.started",
            "turn.completed",
            "turn.started",
            "turn.completed"
        ]
    );
    assert_eq!(response_mock.requests().len(), 3);
    Ok(())
}

#[test]
fn goal_flag_rejects_fork_without_creating_a_thread() {
    let test = test_codex_exec();
    test.cmd()
        .args([
            "--skip-git-repo-check",
            "--goal",
            "fork",
            "missing-thread",
            "finish the task",
        ])
        .timeout(std::time::Duration::from_secs(30))
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "`codex exec --goal` cannot be combined with `fork`",
        ));
    assert!(!test.home_path().join("sessions").exists());
}
