//! Proves the pair the macro generates from this declaration still fits itself: a stub server on
//! one end of an in-memory duplex, the generated client on the other. The real implementation is
//! Attacca's; what is checked here is the shape both sides agree on.

use std::time::Duration;

use futures_util::StreamExt;
use zyris::{Datum, Node, NodeKind, Streaming, Transfer};
use zyris_attacca::{
    attacca_api_capability, AttaccaApi, AttaccaApiCancelTurnRequest, AttaccaApiClient,
    AttaccaApiPeerLookupRequest, AttaccaApiServer, ZAgent, ZDelivered, ZDeltaKind, ZHistoryQuery,
    ZJob, ZJobFilter, ZJobState, ZJobUpdate, ZMe, ZNewAgent, ZNewJob, ZNewMessage, ZNewProject, ZNewSession,
    ZNewWork, ZFileOffered, ZPeerAddr, ZPeerEntry, ZPeerKind, ZProject, ZProjectUpdate, ZScope, ZSession, ZSessionEvent,
    ZSessionFilter, ZTask, ZTaskDep, ZTaskState, ZTurnFrame, ZTurnStatus, ZUsage, ZWork,
    ZWorkFilter, ZWorkState, ZWorkTasks, ZWorkUpdate, ATTACCA_API_CAPABILITY,
};
use zyris_attacca::{
    ZAutoHandover, ZChatMessage, ZFollowedUp, ZGoal, ZGoalDetail, ZGoalState, ZGoalUpdate,
    ZHumanTask, ZHumanTaskResolution, ZHumanTaskState, ZJobFollowedUp, ZMilestone,
    ZMilestoneState, ZMilestoneUpdate, ZNewHumanTask, ZNewMilestone, ZNewNextStep, ZNewThread,
    ZNextStep, ZNextStepState, ZNextStepUpdate, ZProbeReport, ZRouted, ZThread, ZThreadArtifact,
    ZThreadDetail, ZThreadFilter, ZThreadFollowUp, ZThreadSummary, ZThreadUpdate, ZTurnAck,
    ZVerification, ZVerificationOutcome,
};

struct StubApi;

fn stub_job(id: &str, state: ZJobState) -> ZJob {
    ZJob {
        id: id.into(),
        title: "Summarize the changelog".into(),
        description: None,
        state,
        project_id: Some("project-1".into()),
        agent_id: Some("agent-1".into()),
        session_id: Some("session-1".into()),
        last_activity: Some("reading CHANGELOG.md".into()),
        failure_reason: None,
        planning: false,
        thread_id: None,
        created_at: Some("2026-07-29T00:00:00Z".into()),
        updated_at: Some("2026-07-29T00:00:01Z".into()),
    }
}

fn stub_thread(id: &str) -> ZThread {
    ZThread {
        id: id.into(),
        title: "Ship the release".into(),
        description: None,
        agent_id: Some("agent-1".into()),
        project_id: Some("project-1".into()),
        goal_id: Some("goal-1".into()),
        created_at: None,
        updated_at: None,
    }
}

fn stub_goal(id: &str, state: ZGoalState) -> ZGoal {
    ZGoal {
        id: id.into(),
        thread_id: "thread-1".into(),
        thread_title: "Ship the release".into(),
        agent_id: None,
        project_id: None,
        goal: Some("v1 is tagged and published".into()),
        goal_approved_at: None,
        state,
        derived: None,
        planner_session_id: Some("session-7".into()),
        verification_default: true,
        auto_handover: false,
        max_loops: 3,
        loop_count: 0,
        last_activity: None,
        failure_reason: None,
        created_at: None,
        updated_at: None,
    }
}

fn stub_step(goal_id: &str, id: &str, state: ZNextStepState) -> ZNextStep {
    ZNextStep {
        id: id.into(),
        goal_id: goal_id.into(),
        milestone_id: None,
        position: 0,
        title: "Bump the version".into(),
        body: "Bump every crate to 1.0.0".into(),
        acceptance: None,
        verification: true,
        depends_on: vec![],
        state,
        job_id: None,
        handed_over_at: None,
    }
}

fn stub_human_task(goal_id: &str, id: &str, state: ZHumanTaskState) -> ZHumanTask {
    ZHumanTask {
        id: id.into(),
        goal_id: goal_id.into(),
        title: "Log in to crates.io".into(),
        detail: None,
        next_step_id: None,
        milestone_id: None,
        probe: None,
        user_note: None,
        last_probe_ok: None,
        last_probe_output: None,
        state,
    }
}

fn stub_verification(goal_id: &str, id: &str) -> ZVerification {
    ZVerification {
        id: id.into(),
        goal_id: goal_id.into(),
        next_step_id: "step-1".into(),
        work_job_id: "job-1".into(),
        verification_job_id: None,
        verdict: None,
        report: None,
        change_request: None,
        proposed_next_step: None,
        outcome: None,
        capped: false,
    }
}

fn stub_work(id: &str, state: ZWorkState) -> ZWork {
    ZWork {
        id: id.into(),
        title: "Port the parser".into(),
        description: None,
        state,
        project_id: Some("project-1".into()),
        agent_id: Some("agent-1".into()),
        planner_session_id: Some("session-9".into()),
        final_goal: Some("every fixture round-trips".into()),
        requirements_report: None,
        integration_branch: Some("work/work-1/integration".into()),
        failure_reason: None,
        last_activity: None,
        created_at: Some("2026-07-29T00:00:00Z".into()),
        updated_at: Some("2026-07-29T00:00:01Z".into()),
    }
}

fn stub_task(id: &str, work_id: &str, state: ZTaskState) -> ZTask {
    ZTask {
        id: id.into(),
        work_id: work_id.into(),
        title: "Move the lexer".into(),
        description: "Lift it out of the old crate".into(),
        state,
        phase_id: None,
        main_session_id: Some("session-10".into()),
        branch: Some(format!("work/{work_id}/task/{id}")),
        micro_measurables: serde_json::json!([{ "text": "tests pass", "met": false }]),
        verification_result: None,
        failure_reason: None,
        last_activity: None,
        created_at: Some("2026-07-29T00:00:00Z".into()),
        updated_at: Some("2026-07-29T00:00:01Z".into()),
    }
}

#[zyris::async_trait]
impl AttaccaApi for StubApi {
    async fn me(&self) -> zyris::Result<ZMe> {
        Ok(ZMe {
            user_id: "user-1".into(),
            email: "ada@example.com".into(),
            display_name: "Ada".into(),
            scopes: vec![
                ZScope::AgentsRead.as_str().into(),
                ZScope::ProjectsRead.as_str().into(),
                // A scope this crate does not know: a newer deployment's grant, which must not
                // take the whole call down with it.
                "starships:read".into(),
            ],
            plan: Some("pro".into()),
            credits: Some("42.50".into()),
        })
    }

    async fn list_projects(&self) -> zyris::Result<Vec<ZProject>> {
        Ok(vec![
            ZProject {
                id: "project-1".into(),
                name: "Default".into(),
                description: None,
                is_default: true,
            },
            ZProject {
                id: "project-2".into(),
                name: "Rollout".into(),
                description: Some("Q3".into()),
                is_default: false,
            },
        ])
    }

    async fn get_project(&self, project_id: String) -> zyris::Result<ZProject> {
        Ok(ZProject {
            id: project_id,
            name: "Rollout".into(),
            description: Some("Q3".into()),
            is_default: false,
        })
    }

    async fn create_project(&self, project: ZNewProject) -> zyris::Result<ZProject> {
        Ok(ZProject {
            id: "project-3".into(),
            name: project.name,
            description: project.description,
            is_default: false,
        })
    }

    /// Echoes the three-valued description back as it arrived, which is the only way the test can
    /// tell "clear it" apart from "leave it alone".
    async fn update_project(
        &self,
        project_id: String,
        update: ZProjectUpdate,
    ) -> zyris::Result<ZProject> {
        Ok(ZProject {
            id: project_id,
            name: update.name.unwrap_or_else(|| "Rollout".into()),
            description: update.description.unwrap_or(Some("Q3".into())),
            is_default: false,
        })
    }

    async fn delete_project(&self, _project_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn list_agents(&self) -> zyris::Result<Vec<ZAgent>> {
        Ok(vec![ZAgent {
            id: "agent-1".into(),
            name: "Researcher".into(),
            description: None,
            model: Some("claude-opus-5".into()),
        }])
    }

    async fn create_agent(&self, agent: ZNewAgent) -> zyris::Result<ZAgent> {
        Ok(ZAgent {
            id: "agent-2".into(),
            name: agent.name,
            description: agent.description,
            model: agent.model,
        })
    }

    async fn list_sessions(&self, filter: ZSessionFilter) -> zyris::Result<Vec<ZSession>> {
        Ok(vec![ZSession {
            id: "session-1".into(),
            title: None,
            agent_id: Some("agent-1".into()),
            project_id: filter.project_id,
            running: false,
            preamble: None,
        }])
    }

    async fn create_session(
        &self,
        agent_id: String,
        title: Option<String>,
        project_id: Option<String>,
    ) -> zyris::Result<ZSession> {
        self.create_session_with(ZNewSession { agent_id, title, project_id, preamble: None })
            .await
    }

    async fn create_session_with(&self, session: ZNewSession) -> zyris::Result<ZSession> {
        Ok(ZSession {
            id: "session-2".into(),
            title: session.title,
            agent_id: Some(session.agent_id),
            project_id: session.project_id,
            running: false,
            preamble: session.preamble,
        })
    }

    async fn session_history(
        &self,
        _session_id: String,
        query: ZHistoryQuery,
    ) -> zyris::Result<Vec<ZSessionEvent>> {
        let all = vec![
            ZSessionEvent {
                id: None,
                seq: 1,
                cursor: 1,
                kind: "chat_user".into(),
                payload: serde_json::json!({ "kind": "chat_user", "content": "go" }),
                created_at: Some("2026-07-29T00:00:00Z".into()),
            },
            ZSessionEvent {
                id: None,
                seq: 2,
                cursor: 2,
                kind: "chat_agent".into(),
                payload: serde_json::json!({ "kind": "chat_agent", "content": "done" }),
                created_at: Some("2026-07-29T00:00:01Z".into()),
            },
        ];
        let after = query.after.unwrap_or(0);
        let mut out: Vec<ZSessionEvent> = all.into_iter().filter(|e| e.cursor > after).collect();
        if let Some(limit) = query.limit {
            out.truncate(limit as usize);
        }
        Ok(out)
    }

    /// Reports some of what it meters and not the rest, which is the shape a node has to cope
    /// with: `credits_used` is left absent on purpose.
    async fn session_usage(&self, _session_id: String) -> zyris::Result<ZUsage> {
        Ok(ZUsage {
            model: Some("claude-opus-5".into()),
            context_tokens: Some(12_400),
            input_tokens: Some(9_100),
            output_tokens: Some(3_300),
            total_tokens: Some(12_400),
            credits_used: None,
        })
    }

    async fn send_message(
        &self,
        _session_id: String,
        _message: String,
        _data: Vec<Datum>,
    ) -> zyris::Result<()> {
        Ok(())
    }

    // Stub: refuse anything but the agent the test names, so the client side can see the
    // override arrived intact.
    async fn send_message_with(&self, message: ZNewMessage) -> zyris::Result<()> {
        match message.agent_id.as_deref() {
            None | Some("agent-voice") => Ok(()),
            Some(other) => Err(zyris::WireError::invalid_params(format!(
                "unexpected agent {other}"
            ))),
        }
    }

    // Stub: echo the delivery point back through an error so the client side can see it arrived
    // intact.
    async fn cancel_turn(
        &self,
        session_id: String,
        delivered: Option<ZDelivered>,
    ) -> zyris::Result<()> {
        match delivered {
            None => Ok(()),
            Some(d) => Err(zyris::WireError::invalid_params(format!(
                "{session_id}:{:?}:{}",
                d.cursor, d.chars
            ))),
        }
    }

    async fn list_jobs(&self, filter: ZJobFilter) -> zyris::Result<Vec<ZJob>> {
        let mut out = vec![stub_job("job-1", ZJobState::Processing), stub_job("job-2", ZJobState::Done)];
        if let Some(project_id) = filter.project_id {
            out.retain(|job| job.project_id.as_deref() == Some(project_id.as_str()));
        }
        if let Some(limit) = filter.limit {
            out.truncate(limit as usize);
        }
        Ok(out)
    }

    async fn get_job(&self, job_id: String) -> zyris::Result<ZJob> {
        Ok(stub_job(&job_id, ZJobState::Processing))
    }

    async fn create_job(&self, job: ZNewJob) -> zyris::Result<ZJob> {
        let mut created = stub_job("job-3", ZJobState::Processing);
        created.description = Some(job.message);
        created.project_id = job.project_id;
        created.planning = job.planning;
        Ok(created)
    }

    async fn update_job(&self, job_id: String, update: ZJobUpdate) -> zyris::Result<ZJob> {
        let mut job = stub_job(&job_id, ZJobState::Processing);
        if let Some(title) = update.title {
            job.title = title;
        }
        job.description = update.description.or(job.description);
        job.project_id = update.project_id.or(job.project_id);
        Ok(job)
    }

    async fn delete_job(&self, _job_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn list_works(&self, filter: ZWorkFilter) -> zyris::Result<Vec<ZWork>> {
        let mut out = vec![
            stub_work("work-1", ZWorkState::AwaitingGoalApproval),
            stub_work("work-2", ZWorkState::Executing),
        ];
        if let Some(project_id) = filter.project_id {
            out.retain(|work| work.project_id.as_deref() == Some(project_id.as_str()));
        }
        if let Some(limit) = filter.limit {
            out.truncate(limit as usize);
        }
        Ok(out)
    }

    async fn get_work(&self, work_id: String) -> zyris::Result<ZWork> {
        Ok(stub_work(&work_id, ZWorkState::Executing))
    }

    /// Comes back at gate 1, which is where a real deployment leaves it — a node that treats
    /// creation as "it is running now" is the mistake this stub is shaped to expose.
    async fn create_work(&self, work: ZNewWork) -> zyris::Result<ZWork> {
        let mut created = stub_work("work-3", ZWorkState::AwaitingGoalApproval);
        created.description = Some(work.message);
        created.project_id = work.project_id;
        Ok(created)
    }

    async fn update_work(&self, work_id: String, update: ZWorkUpdate) -> zyris::Result<ZWork> {
        let mut work = stub_work(&work_id, ZWorkState::Executing);
        if let Some(title) = update.title {
            work.title = title;
        }
        work.description = update.description.or(work.description);
        work.project_id = update.project_id.or(work.project_id);
        Ok(work)
    }

    async fn delete_work(&self, _work_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn approve_work_goal(&self, work_id: String) -> zyris::Result<ZWork> {
        Ok(stub_work(&work_id, ZWorkState::Planning))
    }

    async fn approve_work_plan(&self, work_id: String) -> zyris::Result<ZWork> {
        Ok(stub_work(&work_id, ZWorkState::Executing))
    }

    async fn work_tasks(&self, work_id: String) -> zyris::Result<ZWorkTasks> {
        Ok(ZWorkTasks {
            tasks: vec![
                stub_task("task-1", &work_id, ZTaskState::Done),
                stub_task("task-2", &work_id, ZTaskState::Creating),
            ],
            deps: vec![ZTaskDep {
                upstream_task_id: "task-1".into(),
                downstream_task_id: "task-2".into(),
            }],
        })
    }

    async fn stop_work(&self, _work_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn continue_work(&self, work_id: String) -> zyris::Result<ZWork> {
        Ok(stub_work(&work_id, ZWorkState::Executing))
    }

    async fn work_message(
        &self,
        _work_id: String,
        _message: String,
        _data: Vec<Datum>,
    ) -> zyris::Result<()> {
        Ok(())
    }

    async fn turn_events(
        &self,
        session_id: String,
        after: Option<i64>,
    ) -> zyris::Result<Streaming<ZTurnStatus, ZTurnFrame>> {
        let head = ZTurnStatus { session_id, running: true, last_cursor: after };
        let frames = vec![
            Ok(ZTurnFrame::Event {
                cursor: 7,
                event: ZSessionEvent {
                    id: None,
                    seq: 1,
                    cursor: 7,
                    kind: "assistant_message".into(),
                    payload: serde_json::json!({ "text": "hi" }),
                    created_at: None,
                },
            }),
            Ok(ZTurnFrame::Delta { kind: ZDeltaKind::Assistant, text: "hi".into() }),
            Ok(ZTurnFrame::Cancelled),
            Ok(ZTurnFrame::Status { running: false }),
        ];
        Ok(Streaming::new(head, futures_util::stream::iter(frames)))
    }

    async fn peer_publish(&self, _endpoint_id: String, _addrs: Vec<String>) -> zyris::Result<()> {
        Ok(())
    }

    /// Echoes the requested path back on the answer, which is what a node calling this with the
    /// path a user named should get: the same path it asked with, not one the server substituted.
    async fn peer_lookup(&self, path: String) -> zyris::Result<ZPeerAddr> {
        Ok(ZPeerAddr {
            node_id: "sibling-1".into(),
            path,
            endpoint_id: "ed25519:sibling-endpoint".into(),
            addrs: vec!["203.0.113.5:4433".into()],
            relay_url: Some("https://relay.attacca.cc".into()),
            online: true,
        })
    }

    async fn peer_list(&self) -> zyris::Result<Vec<ZPeerEntry>> {
        Ok(vec![ZPeerEntry {
            node_id: "sibling-1".into(),
            path: "laptop/zyris-code/myrepo".into(),
            endpoint_id: "ed25519:sibling-endpoint".into(),
            online: true,
            kind: ZPeerKind::Node,
        }])
    }

    async fn peer_relay(&self) -> zyris::Result<Option<String>> {
        Ok(Some("https://relay.attacca.cc".into()))
    }

    async fn request_file(&self, _file_id: String) -> zyris::Result<ZFileOffered> {
        Ok(ZFileOffered {
            transfer_id: "t-1".into(),
            name: "report.md".into(),
            size: 3,
            sha256: "0".repeat(64),
        })
    }

    // Stub: route anything that asks for it to a new job in a new thread.
    async fn send_chat(&self, message: ZChatMessage) -> zyris::Result<ZTurnAck> {
        Ok(if message.route && !message.plan_mode {
            ZTurnAck {
                running: false,
                routed: Some(ZRouted {
                    job_id: "job-9".into(),
                    job_session_id: Some("session-9".into()),
                    thread_id: Some("thread-1".into()),
                    goal_id: None,
                }),
            }
        } else {
            ZTurnAck { running: true, routed: None }
        })
    }

    async fn job_follow_up(
        &self,
        job_id: String,
        message: String,
        _title: Option<String>,
    ) -> zyris::Result<ZJobFollowedUp> {
        let mut job = stub_job("job-4", ZJobState::Processing);
        job.description = Some(message);
        job.thread_id = Some(format!("thread-of-{job_id}"));
        Ok(ZJobFollowedUp { thread_id: format!("thread-of-{job_id}"), goal_id: None, job, next_step: None })
    }

    async fn list_threads(&self, _filter: ZThreadFilter) -> zyris::Result<Vec<ZThreadSummary>> {
        Ok(vec![serde_json::from_value(serde_json::json!({
            "id": "thread-1", "title": "Ship the release", "state": "awaiting_handover"
        }))
        .unwrap()])
    }

    async fn get_thread(&self, thread_id: String) -> zyris::Result<ZThreadDetail> {
        let mut job = stub_job("job-1", ZJobState::Done);
        job.thread_id = Some(thread_id.clone());
        Ok(ZThreadDetail { thread: stub_thread(&thread_id), artifacts: vec![], jobs: vec![job] })
    }

    async fn create_thread(&self, thread: ZNewThread) -> zyris::Result<ZThread> {
        let mut created = stub_thread("thread-2");
        created.title = thread.title.unwrap_or_default();
        created.description = thread.description.or(thread.message);
        Ok(created)
    }

    async fn update_thread(&self, thread_id: String, update: ZThreadUpdate) -> zyris::Result<ZThread> {
        let mut thread = stub_thread(&thread_id);
        if let Some(title) = update.title {
            thread.title = title;
        }
        Ok(thread)
    }

    async fn delete_thread(&self, _thread_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn thread_follow_up(
        &self,
        thread_id: String,
        follow_up: ZThreadFollowUp,
    ) -> zyris::Result<ZFollowedUp> {
        let mut job = stub_job("job-5", ZJobState::Processing);
        job.thread_id = Some(thread_id);
        job.description = Some(follow_up.message);
        Ok(ZFollowedUp { job, next_step: Some(stub_step("goal-1", "step-2", ZNextStepState::HandedOver)) })
    }

    async fn add_thread_artifact(
        &self,
        thread_id: String,
        file_id: String,
        resident: bool,
    ) -> zyris::Result<ZThreadArtifact> {
        Ok(ZThreadArtifact { thread_id, file_id, resident, position: 0, created_at: None })
    }

    async fn remove_thread_artifact(&self, _thread_id: String, _file_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn create_goal(&self, thread_id: String, goal: Option<String>) -> zyris::Result<ZGoal> {
        let mut created = stub_goal("goal-2", ZGoalState::Planning);
        created.thread_id = thread_id;
        created.goal = goal;
        Ok(created)
    }

    async fn get_goal(&self, goal_id: String) -> zyris::Result<ZGoalDetail> {
        Ok(ZGoalDetail {
            goal: stub_goal(&goal_id, ZGoalState::AwaitingHandover),
            milestones: vec![],
            next_steps: vec![stub_step(&goal_id, "step-1", ZNextStepState::Proposed)],
            human_tasks: vec![stub_human_task(&goal_id, "task-1", ZHumanTaskState::Open)],
            verifications: vec![],
        })
    }

    async fn update_goal(&self, goal_id: String, update: ZGoalUpdate) -> zyris::Result<ZGoal> {
        let mut goal = stub_goal(&goal_id, ZGoalState::Running);
        if let Some(max_loops) = update.max_loops {
            goal.max_loops = max_loops;
        }
        Ok(goal)
    }

    async fn delete_goal(&self, _goal_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn set_goal(&self, goal_id: String, text: Option<String>) -> zyris::Result<ZGoal> {
        let mut goal = stub_goal(&goal_id, ZGoalState::Planning);
        goal.goal = text;
        Ok(goal)
    }

    async fn approve_goal(&self, goal_id: String) -> zyris::Result<ZGoal> {
        let mut goal = stub_goal(&goal_id, ZGoalState::AwaitingHandover);
        goal.goal_approved_at = Some("2026-10-21T00:00:00Z".into());
        Ok(goal)
    }

    async fn create_milestone(
        &self,
        goal_id: String,
        milestone: ZNewMilestone,
    ) -> zyris::Result<ZMilestone> {
        Ok(ZMilestone {
            id: "milestone-1".into(),
            goal_id,
            position: milestone.position.unwrap_or(0),
            title: milestone.title,
            detail: milestone.detail,
            state: ZMilestoneState::Pending,
            blocked_reason: None,
        })
    }

    async fn update_milestone(
        &self,
        goal_id: String,
        milestone_id: String,
        update: ZMilestoneUpdate,
    ) -> zyris::Result<ZMilestone> {
        Ok(ZMilestone {
            id: milestone_id,
            goal_id,
            position: 0,
            title: update.title.unwrap_or_default(),
            detail: None,
            state: update.state.unwrap_or(ZMilestoneState::Pending),
            blocked_reason: update.blocked_reason,
        })
    }

    async fn delete_milestone(&self, _goal_id: String, _milestone_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn create_next_step(&self, goal_id: String, step: ZNewNextStep) -> zyris::Result<ZNextStep> {
        let mut created = stub_step(&goal_id, "step-3", ZNextStepState::Proposed);
        created.title = step.title;
        created.depends_on = step.depends_on;
        Ok(created)
    }

    async fn update_next_step(
        &self,
        goal_id: String,
        next_step_id: String,
        update: ZNextStepUpdate,
    ) -> zyris::Result<ZNextStep> {
        let mut step = stub_step(&goal_id, &next_step_id, ZNextStepState::Proposed);
        if let Some(depends_on) = update.depends_on {
            step.depends_on = depends_on;
        }
        Ok(step)
    }

    async fn delete_next_step(&self, _goal_id: String, _next_step_id: String) -> zyris::Result<()> {
        Ok(())
    }

    async fn hand_over_next_step(
        &self,
        goal_id: String,
        next_step_id: String,
        verification: Option<bool>,
    ) -> zyris::Result<ZNextStep> {
        let mut step = stub_step(&goal_id, &next_step_id, ZNextStepState::HandedOver);
        step.verification = verification.unwrap_or(step.verification);
        step.job_id = Some("job-6".into());
        Ok(step)
    }

    async fn set_auto_handover(&self, goal_id: String, enabled: bool) -> zyris::Result<ZAutoHandover> {
        let mut goal = stub_goal(&goal_id, ZGoalState::Running);
        goal.auto_handover = enabled;
        let report = enabled.then(|| zyris_attacca::ZAdvanceReport {
            accepted: vec![],
            handed_over: vec!["step-1".into()],
            halted: Some(zyris_attacca::ZAutoHalt::InFlight),
        });
        Ok(ZAutoHandover { goal, report })
    }

    async fn accept_verification(
        &self,
        goal_id: String,
        verification_id: String,
    ) -> zyris::Result<ZVerification> {
        let mut verification = stub_verification(&goal_id, &verification_id);
        verification.outcome = Some(ZVerificationOutcome::Accepted);
        Ok(verification)
    }

    async fn dismiss_verification(
        &self,
        goal_id: String,
        verification_id: String,
    ) -> zyris::Result<ZVerification> {
        let mut verification = stub_verification(&goal_id, &verification_id);
        verification.outcome = Some(ZVerificationOutcome::Dismissed);
        Ok(verification)
    }

    async fn create_human_task(&self, goal_id: String, task: ZNewHumanTask) -> zyris::Result<ZHumanTask> {
        let mut created = stub_human_task(&goal_id, "task-2", ZHumanTaskState::Open);
        created.title = task.title;
        created.probe = task.probe;
        Ok(created)
    }

    async fn resolve_human_task(
        &self,
        goal_id: String,
        task_id: String,
        resolution: ZHumanTaskResolution,
    ) -> zyris::Result<ZHumanTask> {
        let state = match resolution {
            ZHumanTaskResolution::Done => ZHumanTaskState::Done,
            ZHumanTaskResolution::Dismissed => ZHumanTaskState::Dismissed,
        };
        Ok(stub_human_task(&goal_id, &task_id, state))
    }

    async fn recheck_human_task(&self, goal_id: String, task_id: String) -> zyris::Result<ZProbeReport> {
        Ok(ZProbeReport {
            status: "no_probe".into(),
            ran: false,
            ok: None,
            output: String::new(),
            task: stub_human_task(&goal_id, &task_id, ZHumanTaskState::Open),
            next_step_state: None,
        })
    }

    async fn set_human_task_note(
        &self,
        goal_id: String,
        task_id: String,
        note: Option<String>,
    ) -> zyris::Result<ZHumanTask> {
        let mut task = stub_human_task(&goal_id, &task_id, ZHumanTaskState::Open);
        task.user_note = note;
        Ok(task)
    }
}

fn server_node() -> Node {
    Node::builder()
        .name("attacca")
        .kind(NodeKind::Service)
        .capability(AttaccaApiServer(StubApi))
        .build()
        .unwrap()
}

async fn client() -> AttaccaApiClient {
    let node = Node::builder().name("node").kind(NodeKind::Service).build().unwrap();
    let (_server_side, node_side) = zyris::testing::duplex(&server_node(), &node).await.unwrap();
    node_side.wait_capability(Duration::from_secs(2)).await.unwrap()
}

#[test]
fn descriptor_matches_the_reserved_name() {
    let descriptor = attacca_api_capability();
    assert_eq!(descriptor.name, ATTACCA_API_CAPABILITY);
    assert_eq!(descriptor.version, 1);
    assert_eq!(descriptor.tools.len(), 68);
    // A credential exists to create nodes, so nothing on this surface registers, lists or deletes
    // one any more. A deployment still serving these would be one this crate cannot call.
    for gone in ["register_node", "list_nodes", "delete_node"] {
        assert!(descriptor.tool(gone).is_none(), "{gone} is still declared");
    }
    assert_eq!(descriptor.tool("list_agents").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("me").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("list_projects").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("turn_events").unwrap().transfer, Transfer::UniStream);

    // Tools added within v1: a node discovers them here, and one built before them keeps calling
    // `create_session` unaffected.
    assert_eq!(descriptor.tool("create_session").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("create_session_with").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("session_history").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("session_usage").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("create_project").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("create_job").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("create_work").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("approve_work_plan").unwrap().transfer, Transfer::Unary);
    assert_eq!(descriptor.tool("work_tasks").unwrap().transfer, Transfer::Unary);

    // `turn_events` stays the only stream: everything added since answers once.
    let streams: Vec<&str> = descriptor
        .tools
        .iter()
        .filter(|tool| tool.transfer == Transfer::UniStream)
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(streams, ["turn_events"]);
}

/// The three rendezvous tools added for P2P peer discovery: all unary (a lookup is a single
/// request/response, not a feed), and `turn_events` must still be the only streamed tool.
#[test]
fn rendezvous_tools_are_in_the_descriptor() {
    let descriptor = attacca_api_capability();
    for name in [
        "peer_publish",
        "peer_lookup",
        "peer_list",
        "peer_relay",
        "request_file",
    ] {
        let tool = descriptor.tool(name).unwrap_or_else(|| panic!("{name} is missing"));
        assert_eq!(tool.transfer, Transfer::Unary, "{name}");
    }

    let streams: Vec<&str> = descriptor
        .tools
        .iter()
        .filter(|tool| tool.transfer == Transfer::UniStream)
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(streams, ["turn_events"]);
}

/// The steer toward an unset `title` — leave it off and Attacca's title agent names the session
/// from its first message — reaches a caller only as prose, so nothing but this test notices if it
/// is dropped. It travels by two different routes, hence two assertions: schemars carries a field
/// doc comment into the request schema, while a tool's own doc comment becomes its description. The
/// three-argument `create_session` has only the second route available, since the macro synthesizes
/// its request struct from the signature and arguments cannot carry doc comments.
#[test]
fn the_title_guidance_reaches_both_session_tools() {
    let descriptor = attacca_api_capability();

    // The one struct argument comes back as a `$ref`, so the field docs are under `$defs`.
    let schema = &descriptor.tool("create_session_with").unwrap().request_schema;
    let title = &schema["$defs"]["ZNewSession"]["properties"]["title"];
    let description = title["description"].as_str().unwrap_or_default();
    assert!(
        description.contains("unset") && description.contains("first message"),
        "ZNewSession::title lost its guidance: {description:?}",
    );

    let legacy = &descriptor.tool("create_session").unwrap().description;
    assert!(
        legacy.contains("null") && legacy.contains("first message"),
        "create_session lost its guidance: {legacy:?}",
    );
}

#[test]
fn scope_spellings_survive_a_round_trip() {
    for scope in ZScope::ALL {
        assert_eq!(ZScope::from_str(scope.as_str()), Some(scope));
        assert_eq!(scope.to_string(), scope.as_str());
        let json = serde_json::to_string(&scope).unwrap();
        assert_eq!(json, format!("\"{}\"", scope.as_str()), "serde and as_str disagree");
        assert_eq!(serde_json::from_str::<ZScope>(&json).unwrap(), scope);
    }
    assert_eq!(ZScope::from_str("starships:read"), None);
}

#[tokio::test]
async fn me_reports_identity_and_tolerates_an_unknown_scope() {
    let api = client().await;

    let me = api.me().await.unwrap();
    assert_eq!(me.email, "ada@example.com");
    assert_eq!(me.display_name, "Ada");
    assert!(me.has(ZScope::AgentsRead));
    assert!(!me.has(ZScope::SessionsWrite));

    // The unknown scope rides through as a string and drops out of the typed view.
    assert_eq!(me.scopes.len(), 3);
    assert_eq!(me.known_scopes(), vec![ZScope::AgentsRead, ZScope::ProjectsRead]);
}

#[tokio::test]
async fn list_projects_returns_the_default_and_the_rest() {
    let api = client().await;

    let projects = api.list_projects().await.unwrap();
    assert_eq!(projects.len(), 2);
    assert!(projects[0].is_default);
    assert_eq!(projects[0].name, "Default");
    assert_eq!(projects[1].description.as_deref(), Some("Q3"));
}

#[tokio::test]
async fn node_calls_the_unary_tools() {
    let api = client().await;

    let agents = api.list_agents().await.unwrap();
    assert_eq!(agents[0].name, "Researcher");

    let session = api.create_session("agent-1".into(), Some("Rollout".into()), None).await.unwrap();
    assert_eq!(session.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(session.title.as_deref(), Some("Rollout"));

    api.send_message("session-1".into(), "go".into(), vec![]).await.unwrap();
    api.cancel_turn("session-1".into(), None).await.unwrap();
}

#[tokio::test]
async fn send_message_with_carries_the_agent_override() {
    let api = client().await;
    let message = |agent: Option<&str>| ZNewMessage {
        session_id: "session-1".into(),
        message: "go".into(),
        agent_id: agent.map(str::to_string),
    };
    api.send_message_with(message(Some("agent-voice"))).await.unwrap();
    api.send_message_with(message(None)).await.unwrap();
    let refused = api
        .send_message_with(message(Some("agent-main")))
        .await
        .unwrap_err();
    assert!(refused.message.contains("agent-main"), "{refused:?}");
}

#[tokio::test]
async fn create_session_with_carries_a_preamble() {
    let api = client().await;

    let session = api
        .create_session_with(ZNewSession {
            agent_id: "agent-1".into(),
            title: Some("Triage".into()),
            project_id: None,
            preamble: Some("Answer in exactly three words.".into()),
        })
        .await
        .unwrap();

    assert_eq!(session.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(session.title.as_deref(), Some("Triage"));
    assert_eq!(session.preamble.as_deref(), Some("Answer in exactly three words."));

    // The shape a node should reach for by default: no title, so Attacca names the session itself.
    let untitled = api
        .create_session_with(ZNewSession {
            agent_id: "agent-1".into(),
            title: None,
            project_id: None,
            preamble: None,
        })
        .await
        .unwrap();
    assert_eq!(untitled.title, None);

    // The older three-argument tool still works and leaves the preamble unset.
    let plain = api.create_session("agent-1".into(), None, None).await.unwrap();
    assert_eq!(plain.preamble, None);
}

#[tokio::test]
async fn session_history_reads_the_timeline_back() {
    let api = client().await;

    let all = api.session_history("session-1".into(), ZHistoryQuery::default()).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].kind, "chat_user");
    assert_eq!(all[0].payload["content"], "go");
    assert_eq!(all[1].kind, "chat_agent");
    assert!(all[1].created_at.is_some());

    // `after` is exclusive, and `limit` caps what comes back.
    let tail = api
        .session_history("session-1".into(), ZHistoryQuery { after: Some(1), limit: None })
        .await
        .unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].cursor, 2);

    let capped = api
        .session_history("session-1".into(), ZHistoryQuery { after: None, limit: Some(1) })
        .await
        .unwrap();
    assert_eq!(capped.len(), 1);
    assert_eq!(capped[0].cursor, 1);
}

/// Every field is optional, so what matters is that a partially-reporting deployment round-trips
/// with the absences intact rather than defaulted into zeroes a node would then display.
#[tokio::test]
async fn session_usage_reports_what_the_deployment_meters() {
    let api = client().await;

    let usage = api.session_usage("session-1".into()).await.unwrap();
    assert_eq!(usage.model.as_deref(), Some("claude-opus-5"));
    assert_eq!(usage.context_tokens, Some(12_400));
    assert_eq!(usage.input_tokens, Some(9_100));
    assert_eq!(usage.output_tokens, Some(3_300));
    assert_eq!(usage.credits_used, None);

    // A deployment that meters nothing is a legitimate answer, not an error.
    assert_eq!(serde_json::from_str::<ZUsage>("{}").unwrap(), ZUsage::default());
}

/// The account-level half of the same surface, and for the same reason: a deployment that does not
/// bill leaves both absent rather than sending an empty string a node would render as a blank row.
#[tokio::test]
async fn me_carries_the_accounts_plan_and_balance() {
    let api = client().await;

    let me = api.me().await.unwrap();
    assert_eq!(me.plan.as_deref(), Some("pro"));
    assert_eq!(me.credits.as_deref(), Some("42.50"));

    let bare: ZMe = serde_json::from_str(
        r#"{"user_id":"u","email":"e","display_name":"d"}"#,
    )
    .unwrap();
    assert_eq!(bare.plan, None);
    assert_eq!(bare.credits, None);
}

#[tokio::test]
async fn turn_events_streams_head_then_frames() {
    let api = client().await;

    let mut stream = api.turn_events("session-1".into(), Some(3)).await.unwrap();
    assert_eq!(stream.head.session_id, "session-1");
    assert!(stream.head.running);
    assert_eq!(stream.head.last_cursor, Some(3));

    let mut frames = Vec::new();
    while let Some(frame) = stream.items.next().await {
        frames.push(frame.unwrap());
    }
    assert_eq!(frames.len(), 4);
    assert!(matches!(frames[0], ZTurnFrame::Event { cursor: 7, .. }));
    assert!(matches!(frames[1], ZTurnFrame::Delta { kind: ZDeltaKind::Assistant, .. }));
    assert!(matches!(frames[2], ZTurnFrame::Cancelled));
    assert!(matches!(frames[3], ZTurnFrame::Status { running: false }));
}

#[tokio::test]
async fn cancel_turn_carries_a_delivery_point() {
    let api = client().await;
    api.cancel_turn("session-1".into(), None).await.unwrap();
    let err = api
        .cancel_turn(
            "session-1".into(),
            Some(ZDelivered {
                cursor: Some(42),
                chars: 17,
            }),
        )
        .await
        .unwrap_err();
    assert!(err.message.contains("session-1:Some(42):17"), "{err:?}");
    let err = api
        .cancel_turn(
            "session-1".into(),
            Some(ZDelivered {
                cursor: None,
                chars: 3,
            }),
        )
        .await
        .unwrap_err();
    assert!(err.message.contains("session-1:None:3"), "{err:?}");
}

/// A node built before `delivered` existed sends only `session_id`; that must still decode, as a
/// plain stop.
#[test]
fn cancel_turn_request_without_delivered_decodes_as_a_plain_stop() {
    let old: AttaccaApiCancelTurnRequest =
        serde_json::from_value(serde_json::json!({ "session_id": "s" })).unwrap();
    assert_eq!(old.delivered, None);
    assert_eq!(
        serde_json::to_value(ZDelivered {
            cursor: None,
            chars: 5
        })
        .unwrap(),
        serde_json::json!({ "chars": 5 })
    );
}

#[tokio::test]
async fn project_crud_round_trips() {
    let api = client().await;

    let created = api
        .create_project(ZNewProject { name: "Rollout".into(), description: Some("Q3".into()) })
        .await
        .unwrap();
    assert_eq!((created.name.as_str(), created.description.as_deref()), ("Rollout", Some("Q3")));
    assert!(!created.is_default);

    assert_eq!(api.get_project("project-2".into()).await.unwrap().id, "project-2");

    let renamed = api
        .update_project(
            "project-2".into(),
            ZProjectUpdate { name: Some("Rollout 2".into()), ..ZProjectUpdate::default() },
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "Rollout 2");

    api.delete_project("project-2".into()).await.unwrap();
}

/// The one thing a single `Option` could not express. An omitted `description` has to leave the
/// project's alone while an explicit `null` clears it, and the two are indistinguishable by the time
/// they reach the deployment unless the double wrapping survives the wire — which is what this
/// checks, since the stub echoes whatever it was handed.
#[tokio::test]
async fn clearing_a_description_is_distinct_from_leaving_it_alone() {
    let api = client().await;

    let untouched =
        api.update_project("project-2".into(), ZProjectUpdate::default()).await.unwrap();
    assert_eq!(untouched.description.as_deref(), Some("Q3"));

    let cleared = api
        .update_project(
            "project-2".into(),
            ZProjectUpdate { description: Some(None), ..ZProjectUpdate::default() },
        )
        .await
        .unwrap();
    assert_eq!(cleared.description, None);

    // And the same distinction on the way in, since that is where serde has to be told about it.
    let omitted: ZProjectUpdate = serde_json::from_str("{}").unwrap();
    assert_eq!(omitted.description, None);
    let explicit_null: ZProjectUpdate = serde_json::from_str(r#"{"description":null}"#).unwrap();
    assert_eq!(explicit_null.description, Some(None));
}

#[tokio::test]
async fn job_crud_round_trips() {
    let api = client().await;

    let created = api
        .create_job(ZNewJob {
            message: "Summarize the changelog".into(),
            agent_id: None,
            project_id: Some("project-1".into()),
            timezone: Some("Europe/Berlin".into()),
            planning: false,
            plan_mode: false,
            factcheck: false,
            data: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(created.description.as_deref(), Some("Summarize the changelog"));
    assert_eq!(created.state, ZJobState::Processing);
    // The session the node then streams with the tools it already had.
    assert_eq!(created.session_id.as_deref(), Some("session-1"));

    assert_eq!(api.get_job("job-1".into()).await.unwrap().id, "job-1");

    let all = api.list_jobs(ZJobFilter::default()).await.unwrap();
    assert_eq!(all.len(), 2);
    let capped = api.list_jobs(ZJobFilter { limit: Some(1), ..ZJobFilter::default() }).await.unwrap();
    assert_eq!(capped.len(), 1);

    let renamed = api
        .update_job("job-1".into(), ZJobUpdate { title: Some("Renamed".into()), ..ZJobUpdate::default() })
        .await
        .unwrap();
    assert_eq!(renamed.title, "Renamed");

    api.delete_job("job-1".into()).await.unwrap();
}

/// A created work is not a running work: it stops at gate 1 and stays there. The two approvals are
/// the whole reason works have tools of their own rather than being jobs with extra fields.
#[tokio::test]
async fn a_work_is_created_then_let_through_both_gates() {
    let api = client().await;

    let created = api
        .create_work(ZNewWork {
            message: "Port the parser\nand keep the fixtures green".into(),
            agent_id: None,
            project_id: Some("project-1".into()),
        })
        .await
        .unwrap();
    assert_eq!(created.state, ZWorkState::AwaitingGoalApproval);
    // What gate 1 is actually deciding on, and the session to argue with it over.
    assert_eq!(created.final_goal.as_deref(), Some("every fixture round-trips"));
    assert_eq!(created.planner_session_id.as_deref(), Some("session-9"));

    api.work_message(created.id.clone(), "narrow it to the lexer".into(), Vec::new()).await.unwrap();

    let planning = api.approve_work_goal(created.id.clone()).await.unwrap();
    assert_eq!(planning.state, ZWorkState::Planning);

    let executing = api.approve_work_plan(created.id.clone()).await.unwrap();
    assert_eq!(executing.state, ZWorkState::Executing);
}

#[tokio::test]
async fn work_tasks_come_back_as_a_graph() {
    let api = client().await;

    let graph = api.work_tasks("work-1".into()).await.unwrap();
    assert_eq!(graph.tasks.len(), 2);
    assert_eq!(graph.tasks[0].state, ZTaskState::Done);
    assert_eq!(graph.tasks[1].state, ZTaskState::Creating);
    // The planner's own shape, passed through rather than typed by the protocol.
    assert_eq!(graph.tasks[0].micro_measurables[0]["text"], "tests pass");
    assert_eq!(graph.deps.len(), 1);
    assert_eq!(graph.deps[0].upstream_task_id, "task-1");
    assert_eq!(graph.deps[0].downstream_task_id, "task-2");

    // An unplanned work has a graph, it is just empty — not an error and not an absent field.
    assert_eq!(serde_json::from_str::<ZWorkTasks>("{}").unwrap(), ZWorkTasks::default());
}

#[tokio::test]
async fn work_read_and_lifecycle_tools_round_trip() {
    let api = client().await;

    assert_eq!(api.list_works(ZWorkFilter::default()).await.unwrap().len(), 2);
    let capped =
        api.list_works(ZWorkFilter { limit: Some(1), ..ZWorkFilter::default() }).await.unwrap();
    assert_eq!(capped.len(), 1);

    assert_eq!(api.get_work("work-1".into()).await.unwrap().id, "work-1");

    let renamed = api
        .update_work("work-1".into(), ZWorkUpdate { title: Some("Renamed".into()), ..ZWorkUpdate::default() })
        .await
        .unwrap();
    assert_eq!(renamed.title, "Renamed");

    api.stop_work("work-1".into()).await.unwrap();
    assert_eq!(api.continue_work("work-1".into()).await.unwrap().state, ZWorkState::Executing);
    api.delete_work("work-1".into()).await.unwrap();
}

/// The two scopes added alongside the work tools. `ZScope::ALL` is what a node asks for when it
/// wants everything, so a scope missing from it is a scope no node ever requests.
#[tokio::test]
async fn works_scopes_are_spelled_the_way_the_wire_spells_them() {
    assert_eq!(ZScope::WorksRead.as_str(), "works:read");
    assert_eq!(ZScope::WorksWrite.as_str(), "works:write");
    assert_eq!(ZScope::from_str("works:write"), Some(ZScope::WorksWrite));
    assert!(ZScope::ALL.contains(&ZScope::WorksRead));
    assert!(ZScope::ALL.contains(&ZScope::WorksWrite));
}

/// There is no scope for creating nodes: that is what a credential is for. `ZScope::ALL` is what a
/// node asks for when it wants everything, and a deployment that no longer knows `nodes:write`
/// refuses that whole request by name — so a scope left in here breaks every such enrollment.
#[test]
fn there_is_no_scope_for_creating_nodes() {
    assert_eq!(ZScope::from_str("nodes:write"), None);
    assert!(ZScope::ALL.iter().all(|scope| scope.as_str() != "nodes:write"));
}

/// The scope guarding the three rendezvous tools.
#[test]
fn peers_write_scope_exists() {
    let scope: ZScope = serde_json::from_str("\"peers:write\"").unwrap();
    assert_eq!(scope, ZScope::PeersWrite);
    assert_eq!(scope.as_str(), "peers:write");
    assert_eq!(ZScope::from_str("peers:write"), Some(ZScope::PeersWrite));
    assert!(ZScope::ALL.contains(&ZScope::PeersWrite));
}

/// The lookup key is the node path, not `node_id`: this is what a node calling
/// `peer_lookup("laptop/zyris-code/myrepo")` gets back, and it must be the same path it asked with
/// rather than one the server minted, since what the caller said is what TOFU pinning in
/// `zyris-p2p` keys on.
#[tokio::test]
async fn peer_rendezvous_tools_round_trip() {
    let api = client().await;

    api.peer_publish("ed25519:my-endpoint".into(), vec!["198.51.100.9:4433".into()]).await.unwrap();

    let addr = api.peer_lookup("laptop/zyris-code/myrepo".into()).await.unwrap();
    assert_eq!(addr.path, "laptop/zyris-code/myrepo", "answered under the path it was asked for");
    assert_eq!(addr.node_id, "sibling-1");
    assert!(!addr.addrs.is_empty());
    assert!(addr.online);

    let peers = api.peer_list().await.unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].path, "laptop/zyris-code/myrepo");
    assert_eq!(peers[0].endpoint_id, "ed25519:sibling-endpoint");
}

/// The request key changed from `slug` to `path` when the lookup moved to a full node path. A
/// server still reading `slug` off the wire would find nothing and refuse every lookup, so this
/// pins the generated request's serialized shape directly rather than trusting the round trip
/// above to notice a stale key on both ends at once.
#[test]
fn peer_lookup_request_serializes_with_a_path_key_not_slug() {
    let request = AttaccaApiPeerLookupRequest { path: "laptop/zyris-code/myrepo".into() };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"path": "laptop/zyris-code/myrepo"})
    );
}

/// `ZPeerAddr` is exchanged between two independently-deployed systems, so its optionality is a
/// wire contract, not an internal detail: a deployment that omits `addrs` or `relay_url` must still
/// decode, with the field defaulting rather than the whole call failing.
#[test]
fn peer_addr_optional_fields_default_when_absent() {
    let missing_both = serde_json::json!({
        "node_id": "sibling-1",
        "path": "laptop/zyris-code/myrepo",
        "endpoint_id": "ed25519:sibling-endpoint",
        "online": true,
    });
    let addr: ZPeerAddr = serde_json::from_value(missing_both).unwrap();
    assert_eq!(addr.addrs, Vec::<String>::new());
    assert_eq!(addr.relay_url, None);

    // Checked with the other field present too, so each default is pinned to its own JSON key
    // rather than both happening to pass only when both are absent together.
    let missing_relay_only = serde_json::json!({
        "node_id": "sibling-1",
        "path": "laptop/zyris-code/myrepo",
        "endpoint_id": "ed25519:sibling-endpoint",
        "addrs": ["203.0.113.5:4433"],
        "online": true,
    });
    let addr: ZPeerAddr = serde_json::from_value(missing_relay_only).unwrap();
    assert_eq!(addr.addrs, vec!["203.0.113.5:4433".to_string()]);
    assert_eq!(addr.relay_url, None);
}

/// The other half of the same contract: an unset `relay_url` must not appear in the JSON at all, on
/// the same grounds as [`ZMe::credits`] and [`ZUsage`] — a deployment that does not have one sends
/// nothing, not `null`, so a strict decoder on the other side never has to special-case it.
#[test]
fn peer_addr_relay_url_is_omitted_not_null_when_unset() {
    let addr = ZPeerAddr {
        node_id: "sibling-1".into(),
        path: "laptop/zyris-code/myrepo".into(),
        endpoint_id: "ed25519:sibling-endpoint".into(),
        addrs: vec![],
        relay_url: None,
        online: true,
    };
    let json = serde_json::to_value(&addr).unwrap();
    assert!(
        !json.as_object().unwrap().contains_key("relay_url"),
        "relay_url must be omitted rather than sent as null: {json}",
    );
}

/// `ZPeerEntry` has no optional fields — unlike `ZPeerAddr`, every one of these is required, and a
/// deployment that omits one must fail to decode rather than silently default it away. A
/// deployment still sending the old `slug` is one of those.
#[test]
fn peer_entry_rejects_a_missing_required_field() {
    let missing_endpoint_id = serde_json::json!({
        "node_id": "sibling-1",
        "path": "laptop/zyris-code/myrepo",
        "online": true,
    });
    assert!(serde_json::from_value::<ZPeerEntry>(missing_endpoint_id).is_err());

    let still_a_slug = serde_json::json!({
        "node_id": "sibling-1",
        "slug": "laptop",
        "endpoint_id": "ed25519:sibling-endpoint",
        "online": true,
    });
    assert!(serde_json::from_value::<ZPeerEntry>(still_a_slug).is_err());
}

#[tokio::test]
async fn peer_relay_and_request_file_cross_the_wire() {
    let api = client().await;
    assert_eq!(api.peer_relay().await.unwrap().as_deref(), Some("https://relay.attacca.cc"));
    let offered = api.request_file("f-1".into()).await.unwrap();
    assert_eq!(offered.transfer_id, "t-1");
    assert_eq!(offered.size, 3);
}

#[tokio::test]
async fn send_chat_says_where_a_routed_message_went() {
    let api = client().await;
    let chat = |route, plan_mode| ZChatMessage {
        session_id: "session-1".into(),
        message: "refactor the parser".into(),
        agent_id: None,
        route,
        plan_mode,
    };

    let routed = api.send_chat(chat(true, false)).await.unwrap();
    assert!(!routed.running);
    let to = routed.routed.unwrap();
    assert_eq!(to.job_id, "job-9");
    assert_eq!(to.job_session_id.as_deref(), Some("session-9"));

    // Plan mode is never routed: the turn runs where it was sent.
    let here = api.send_chat(chat(true, true)).await.unwrap();
    assert!(here.running && here.routed.is_none());
}

#[test]
fn chat_message_without_route_or_plan_mode_decodes_as_a_plain_message() {
    let message: ZChatMessage =
        serde_json::from_value(serde_json::json!({ "session_id": "s", "message": "hi" })).unwrap();
    assert!(!message.route && !message.plan_mode);
}

#[test]
fn a_job_from_before_threads_decodes_without_a_thread() {
    let mut wire = serde_json::to_value(stub_job("job-1", ZJobState::Done)).unwrap();
    assert!(!wire.as_object().unwrap().contains_key("thread_id"));
    wire.as_object_mut().unwrap().remove("planning");
    let back: ZJob = serde_json::from_value(wire).unwrap();
    assert_eq!(back.thread_id, None);
}

#[tokio::test]
async fn a_thread_is_opened_followed_up_and_read_back() {
    let api = client().await;

    let thread = api
        .create_thread(ZNewThread {
            message: Some("Ship the release".into()),
            ..ZNewThread::default()
        })
        .await
        .unwrap();
    assert_eq!(thread.description.as_deref(), Some("Ship the release"));

    let followed = api
        .thread_follow_up(
            "thread-1".into(),
            ZThreadFollowUp {
                message: "also update the changelog".into(),
                source_job_id: Some("job-1".into()),
                agent_id: None,
                plan_mode: false,
                factcheck: false,
                data: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(followed.job.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(followed.next_step.unwrap().state, ZNextStepState::HandedOver);

    let from_job = api.job_follow_up("job-1".into(), "and tag it".into(), None).await.unwrap();
    assert_eq!(from_job.job.thread_id.as_deref(), Some(from_job.thread_id.as_str()));

    let detail = api.get_thread("thread-1".into()).await.unwrap();
    assert_eq!(detail.jobs[0].thread_id.as_deref(), Some("thread-1"));

    let listed = api.list_threads(ZThreadFilter::default()).await.unwrap();
    assert_eq!(listed[0].state, ZGoalState::AwaitingHandover);
    assert!(listed[0].jobs.is_empty());

    let pinned = api.add_thread_artifact("thread-1".into(), "file-1".into(), true).await.unwrap();
    assert!(pinned.resident);
    api.remove_thread_artifact("thread-1".into(), "file-1".into()).await.unwrap();
    api.delete_thread("thread-1".into()).await.unwrap();
}

#[tokio::test]
async fn a_plan_is_approved_handed_over_and_unblocked() {
    let api = client().await;

    let detail = api.get_goal("goal-1".into()).await.unwrap();
    assert_eq!(detail.next_steps[0].state, ZNextStepState::Proposed);
    let approved = api.approve_goal("goal-1".into()).await.unwrap();
    assert!(approved.goal_approved_at.is_some());

    let step = api
        .hand_over_next_step("goal-1".into(), "step-1".into(), Some(false))
        .await
        .unwrap();
    assert_eq!(step.state, ZNextStepState::HandedOver);
    assert!(!step.verification);

    let auto = api.set_auto_handover("goal-1".into(), true).await.unwrap();
    assert!(auto.goal.auto_handover);
    assert_eq!(auto.report.unwrap().halted, Some(zyris_attacca::ZAutoHalt::InFlight));
    assert!(api.set_auto_handover("goal-1".into(), false).await.unwrap().report.is_none());

    let resolved = api
        .resolve_human_task("goal-1".into(), "task-1".into(), ZHumanTaskResolution::Dismissed)
        .await
        .unwrap();
    assert_eq!(resolved.state, ZHumanTaskState::Dismissed);
    let noted = api
        .set_human_task_note("goal-1".into(), "task-1".into(), Some("done on my phone".into()))
        .await
        .unwrap();
    assert_eq!(noted.user_note.as_deref(), Some("done on my phone"));

    let accepted = api.accept_verification("goal-1".into(), "v-1".into()).await.unwrap();
    assert_eq!(accepted.outcome, Some(ZVerificationOutcome::Accepted));

    let step = api
        .create_next_step(
            "goal-1".into(),
            ZNewNextStep {
                title: "Publish".into(),
                body: "cargo publish every crate".into(),
                depends_on: vec!["step-1".into()],
                ..ZNewNextStep::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(step.depends_on, ["step-1"]);
}

#[test]
fn goal_enums_use_attaccas_spelling() {
    assert_eq!(serde_json::to_value(ZGoalState::AwaitingHandover).unwrap(), "awaiting_handover");
    assert_eq!(serde_json::to_value(ZNextStepState::HandedOver).unwrap(), "handed_over");
    assert_eq!(
        serde_json::to_value(zyris_attacca::ZVerificationVerdict::ChangeRequest).unwrap(),
        "change_request"
    );
    assert_eq!(
        serde_json::to_value(zyris_attacca::ZAutoHalt::BlockedOnPerson).unwrap(),
        "blocked_on_person"
    );
}
