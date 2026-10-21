//! `attacca_api` — the capability the server side of the connection announces.
//!
//! Every other capability in this workspace is something a *node* offers. This one runs the other
//! way: [Attacca](https://attacca.cc) reserves the name `attacca_api`, rejects it from any node
//! that tries to announce it, and announces it itself right after the handshake. A node picks the
//! client up on connect and drives agents, projects, jobs and threads over the same websocket it
//! serves its own tools on.
//!
//! Attacca's composer has three kinds of chat, and each is one call here: "Chat" is `create_job`,
//! "Plan" is `create_job` with `plan_mode`, and "Thread" is `create_thread`. Bare sessions and works
//! are no longer started from Attacca's UI; their tools stay for what already exists.
//!
//! It sits in its own crate rather than in `zyris-caps` because `zyris-caps` is the catalogue of
//! things a node implements, and this is the one surface a node only ever calls.
//!
//! Tools are added within version 1 rather than by bumping it: additive tool changes inside a
//! version are permitted, and consumers discover tools by descriptor. An older node keeps working
//! because it never asks for the new ones; a newer node against an older deployment finds the tool
//! absent from the announcement and gets `capability_not_announced` if it calls anyway.
//!
//! A node consumes it this way, in two lines of imports. Depending on the crate is still a
//! convenience rather than a requirement: matching is by `(name, version)` and the announced tool
//! list is never compared, so a node may instead declare just the slice it calls with its own
//! `#[zyris::capability]` trait — which is what consuming any capability nobody has published a
//! crate for looks like.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zyris::{Datum, Streaming};

/// The capability name Attacca reserves to itself on every connection.
pub const ATTACCA_API_CAPABILITY: &str = "attacca_api";

/// What a node's grant may contain — Attacca's scope vocabulary, spelled the way the wire spells
/// it. A node asks for a subset at enrollment (`Runner::request_scopes`, or `$ZYRIS_SCOPES`), the
/// approving user may grant fewer, and `me` reports what was actually granted.
///
/// Every `attacca_api` tool but `me` is scope-checked at call time. The descriptor lists them all
/// regardless of the grant, so a tool being announced is not permission to call it: a node that
/// wants to know what it may do calls `me` and reads [`ZMe::scopes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum ZScope {
    #[serde(rename = "agents:read")]
    AgentsRead,
    #[serde(rename = "agents:write")]
    AgentsWrite,
    #[serde(rename = "projects:read")]
    ProjectsRead,
    #[serde(rename = "projects:write")]
    ProjectsWrite,
    #[serde(rename = "sessions:read")]
    SessionsRead,
    #[serde(rename = "sessions:write")]
    SessionsWrite,
    #[serde(rename = "files:read")]
    FilesRead,
    #[serde(rename = "files:write")]
    FilesWrite,
    #[serde(rename = "jobs:read")]
    JobsRead,
    #[serde(rename = "jobs:write")]
    JobsWrite,
    /// Works are scoped apart from jobs rather than folded in with them: a work owns a git
    /// integration branch and a graph of task worktrees, so granting a node the run of them is a
    /// materially larger thing to hand over than letting it queue a job.
    #[serde(rename = "works:read")]
    WorksRead,
    #[serde(rename = "works:write")]
    WorksWrite,
    #[serde(rename = "artifacts:read")]
    ArtifactsRead,
    #[serde(rename = "artifacts:write")]
    ArtifactsWrite,
    #[serde(rename = "kanban:read")]
    KanbanRead,
    #[serde(rename = "kanban:write")]
    KanbanWrite,
    /// The account-wide event stream, including `turn_events`. One scope rather than the union of
    /// every read scope: a partially-scoped stream that silently omits frame kinds is worse than no
    /// stream, because a caller cannot tell "nothing happened" from "you weren't allowed to see it".
    #[serde(rename = "events:read")]
    EventsRead,
    /// P2P rendezvous between nodes on the same account — publish this node's own address and ask
    /// for a sibling's. The file itself never passes through Attacca, so what this scope opens is
    /// **the address book only**.
    #[serde(rename = "peers:write")]
    PeersWrite,
}

impl ZScope {
    /// Every scope, in the order Attacca lists them. Useful for a node that wants to ask for
    /// everything and let the approving user cut it down.
    pub const ALL: [ZScope; 18] = [
        ZScope::AgentsRead,
        ZScope::AgentsWrite,
        ZScope::ProjectsRead,
        ZScope::ProjectsWrite,
        ZScope::SessionsRead,
        ZScope::SessionsWrite,
        ZScope::FilesRead,
        ZScope::FilesWrite,
        ZScope::JobsRead,
        ZScope::JobsWrite,
        ZScope::WorksRead,
        ZScope::WorksWrite,
        ZScope::ArtifactsRead,
        ZScope::ArtifactsWrite,
        ZScope::KanbanRead,
        ZScope::KanbanWrite,
        ZScope::EventsRead,
        ZScope::PeersWrite,
    ];

    /// The wire spelling, which is also what `request_scopes` and `$ZYRIS_SCOPES` take.
    pub fn as_str(self) -> &'static str {
        match self {
            ZScope::AgentsRead => "agents:read",
            ZScope::AgentsWrite => "agents:write",
            ZScope::ProjectsRead => "projects:read",
            ZScope::ProjectsWrite => "projects:write",
            ZScope::SessionsRead => "sessions:read",
            ZScope::SessionsWrite => "sessions:write",
            ZScope::FilesRead => "files:read",
            ZScope::FilesWrite => "files:write",
            ZScope::JobsRead => "jobs:read",
            ZScope::JobsWrite => "jobs:write",
            ZScope::WorksRead => "works:read",
            ZScope::WorksWrite => "works:write",
            ZScope::ArtifactsRead => "artifacts:read",
            ZScope::ArtifactsWrite => "artifacts:write",
            ZScope::KanbanRead => "kanban:read",
            ZScope::KanbanWrite => "kanban:write",
            ZScope::EventsRead => "events:read",
            ZScope::PeersWrite => "peers:write",
        }
    }

    /// The inverse of [`ZScope::as_str`]. Returns `None` for a scope this crate does not know,
    /// which is what a *newer* deployment granting a *newer* scope looks like from here.
    pub fn from_str(s: &str) -> Option<ZScope> {
        ZScope::ALL.into_iter().find(|scope| scope.as_str() == s)
    }
}

impl std::fmt::Display for ZScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who the connection is authorized as, and what it may do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZMe {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    /// The grant, as the wire spells it. Strings rather than [`ZScope`] because a deployment may
    /// grant a scope this crate predates, and a `me` call that fails to deserialize is a worse
    /// answer than one naming a scope the caller does not recognize. See [`ZMe::known_scopes`].
    #[serde(default)]
    pub scopes: Vec<String>,
    /// The account's billing plan, as the deployment names it. Absent on a deployment that does not
    /// meter, or one that predates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Credit balance, formatted by the deployment rather than parsed here — the unit and the
    /// precision are its business, and a node only ever displays this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<String>,
}

impl ZMe {
    /// The granted scopes this crate knows, unknown ones dropped.
    pub fn known_scopes(&self) -> Vec<ZScope> {
        self.scopes.iter().filter_map(|s| ZScope::from_str(s)).collect()
    }

    /// Whether the grant contains `scope`. Cheaper and more honest than `known_scopes().contains`:
    /// it compares wire spellings, so it is unaffected by scopes this crate predates.
    pub fn has(&self, scope: ZScope) -> bool {
        self.scopes.iter().any(|s| s == scope.as_str())
    }
}

/// A project: the account-level folder a session, board, or job belongs to. `ZSession.project_id`
/// names one of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZProject {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The account's undeletable fallback project. Resources created without a project land here,
    /// and exactly one project per account has this set.
    #[serde(default)]
    pub is_default: bool,
}

/// What [`AttaccaApi::create_project`] takes. A project is a folder, not a workspace: nothing runs
/// in it, so there is nothing to configure beyond its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewProject {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A partial project edit: an omitted field is left alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZProjectUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Three-valued, which is why it is doubly wrapped: omitted leaves the description as it is, an
    /// explicit `null` clears it, and a string replaces it. A single `Option` would collapse the
    /// first two together and leave a node no way to remove a description it once set.
    #[serde(default, deserialize_with = "deserialize_some", skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
}

/// Forces an explicit JSON `null` to deserialize as `Some(None)` rather than collapsing into the
/// same `None` a missing key produces.
fn deserialize_some<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZAgent {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewAgent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZSession {
    pub id: String,
    /// A session created without a title reads back under Attacca's placeholder, not as absent —
    /// the title agent replaces it once the session has a first message to name it from, so a
    /// title seen here is only final for a session that has already taken a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub running: bool,
    /// The session's own system instructions, appended to its agent's for every turn. See
    /// [`ZNewSession::preamble`]. Absent on a deployment that predates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
}

/// What [`AttaccaApi::create_session_with`] takes: everything [`AttaccaApi::create_session`]'s three
/// arguments carry, plus the options added since. A struct rather than more arguments because the
/// generated request struct has no per-field default — a new *argument* on an existing tool is a
/// decode error for every node built before it, while a new field on a struct whose fields are all
/// `#[serde(default)]` is not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewSession {
    pub agent_id: String,
    /// Prefer leaving this unset. Attacca titles a session from its first message, so an untitled
    /// session gets a real name the moment it is used, in the language that message was written in.
    /// Set it only for a session a human will go looking for under a name the node already knows —
    /// a title given here is permanent, and opts the session out of that auto-titling for good.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Omit to file the session under the account's default project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// System instructions for this session alone, appended to the agent's own preamble on every
    /// turn — the agent keeps its identity, tools and skills, and this narrows what it is doing
    /// here. Fixed for the session's lifetime; a node wanting different instructions opens a
    /// different session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
}

/// What [`AttaccaApi::send_message_with`] takes: a message, and optionally the agent that answers
/// it. A struct for the reason [`ZNewSession`] is one — a new field defaults, a new argument on an
/// existing tool is a decode error for every node built before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewMessage {
    pub session_id: String,
    pub message: String,
    /// The agent that answers this one message, in place of the session's own — the per-message
    /// override the web app's agent picker sends. Omit to keep the session's agent. A voice node
    /// uses it so that whichever session a person picked, the answer comes from an agent written
    /// to be read aloud.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// The window [`AttaccaApi::session_history`] reads. Every field defaults, so the whole timeline is
/// `ZHistoryQuery::default()`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZHistoryQuery {
    /// Return only entries past this cursor, exclusive — the `cursor` of the last entry already
    /// seen, from either this tool or `turn_events`. Omit for the whole timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<i64>,
    /// At most this many entries, taken oldest-first from `after`. Omit for everything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// What one session has cost so far: [`AttaccaApi::session_usage`]'s answer.
///
/// Every field is optional and defaults, so a deployment reports what it actually meters and a node
/// displays what it is given. `Default` is a legitimate response — it means "metered nothing" —
/// which is why absence is spelled per-field rather than by the call failing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZUsage {
    /// The model the session's turns ran on, as the deployment names it. A session that has taken
    /// no turn has none yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Tokens currently in context — what the next turn starts from, not a running total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
    /// Credits this session has consumed. A string for the same reason as [`ZMe::credits`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits_used: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZSessionFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Where a job is in its life. Derived by Attacca from the job's session rather than set by the
/// caller, so this is a thing to read and not a thing to write — [`ZJobUpdate`] has no state field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZJobState {
    Backlog,
    Processing,
    Done,
    /// The agent asked a question and is waiting on an answer — send one with `send_message` to the
    /// job's `session_id`.
    Ambiguous,
    Failed,
    Recurring,
    /// A plan-mode job that has produced a plan and is waiting for it to be approved.
    AwaitingPlan,
}

/// One item of autonomous work: a message handed to an agent that runs it to completion on its own
/// session, reporting long results as artifacts. The unit a node reaches for when it wants something
/// done rather than a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZJob {
    pub id: String,
    /// Titled in the background from the opening message, so a job read back immediately still
    /// carries Attacca's placeholder.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub state: ZJobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The session driving the job. Pass it to `send_message` to answer a question, or to
    /// `turn_events` to watch the work happen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// A one-line "what the agent is doing right now", so a node can show progress without opening
    /// the event stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    /// Why the job ended, set only in [`ZJobState::Failed`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// A planning job: it drafts a thread's plan (its [`ZGoal`]) rather than doing the work itself.
    /// The job [`AttaccaApi::create_thread`] starts is one.
    #[serde(default)]
    pub planning: bool,
    /// The thread this job belongs to. Absent for a job that stands alone, and on a deployment that
    /// predates threads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// What [`AttaccaApi::create_job`] takes. Creating a job starts it: the first turn is enqueued
/// before this call returns, so the job comes back already `Processing`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewJob {
    /// What the agent is being asked to do. Also what the job is titled from.
    pub message: String,
    /// Omit to run the job on the account's Main Agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Omit to file the job under the account's default project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// IANA name, e.g. `Europe/Berlin`. What the agent answers "what time is it" with; omitting it
    /// leaves the deployment's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// Draft a plan instead of doing the work here. Prefer [`AttaccaApi::create_thread`], which
    /// starts such a job with a thread for the plan to live in.
    #[serde(default)]
    pub planning: bool,
    /// Investigate, hand back a plan, and wait for approval before executing — all within this one
    /// job. Distinct from `planning`, which hands off to a thread's plan. This is the web composer's
    /// "Plan"; leaving it unset is its "Chat".
    #[serde(default)]
    pub plan_mode: bool,
    /// Fact-check the job's chat: after each reply a background checker rates its factual claims and
    /// reports the low-confidence ones back to the agent.
    #[serde(default)]
    pub factcheck: bool,
    /// Attachments for the opening message, the same way `send_message` takes them. Text datums are
    /// folded into `message`; files land in the account's workspace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<Datum>,
}

/// A partial job edit: an omitted field is left alone. State is derived and cannot be set here.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZJobUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZJobFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Where a work is in its life. Like [`ZJobState`], derived rather than set — but two of these are
/// gates rather than progress: [`ZWorkState::AwaitingGoalApproval`] and
/// [`ZWorkState::AwaitingPlanApproval`] are where the loop stops and waits to be let through, with
/// `approve_work_goal` and `approve_work_plan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZWorkState {
    Draft,
    CheckingRequirements,
    /// Gate 1: the goal is drafted and needs approving before planning starts.
    AwaitingGoalApproval,
    Planning,
    /// Gate 2: the task graph is drafted and needs approving before anything executes.
    AwaitingPlanApproval,
    Executing,
    /// A phase finished that was set to stop for review. `continue_work` resumes.
    Halted,
    Verifying,
    Done,
    Failed,
    Cancelled,
}

/// A larger unit than a job: a goal that is planned into a graph of tasks, each executed in its own
/// git worktree and merged into a shared integration branch. A work pauses twice for approval, which
/// is why creating one is not enough to make it run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZWork {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub state: ZWorkState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The planning conversation. This is an ordinary session id: `send_message` to argue with the
    /// plan, `turn_events` to watch it being drawn up, `work_message` to do the former without
    /// having to read this field first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_session_id: Option<String>,
    /// The measurable success criterion, settled at gate 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_goal: Option<String>,
    /// Blockers found up front — a missing toolchain, a login the agent cannot perform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements_report: Option<String>,
    /// The git branch the task worktrees merge into. Set once execution starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// What [`AttaccaApi::create_work`] takes. Creating a work starts its planning turns; it then stops
/// at `awaiting_goal_approval` and stays there until `approve_work_goal` is called.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewWork {
    /// The goal, in prose. The work's title is derived from its first line.
    pub message: String,
    /// Omit to plan the work with the account's Main Agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Omit to file the work under the account's default project. A work's tasks run against the
    /// project's checkout, so this decides what the work is allowed to change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// A partial work edit: an omitted field is left alone. State is derived and cannot be set here —
/// use the approval and stop/continue tools to move a work along.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZWorkUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZWorkFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Where one task in a work's graph has got to. `creating`/`verifying`/`recording` name the agent
/// role currently holding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZTaskState {
    Pending,
    /// An upstream task failed, so this one cannot start.
    Blocked,
    /// The agent asked a question and is waiting on an answer in the task's own session.
    AwaitingInput,
    Creating,
    SelfVerifying,
    Verifying,
    Recording,
    Merging,
    Done,
    Failed,
    /// The merge into the integration branch hit a conflict.
    Conflicted,
}

/// One node of a work's task graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZTask {
    pub id: String,
    pub work_id: String,
    pub title: String,
    pub description: String,
    pub state: ZTaskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_id: Option<String>,
    /// The session doing the work. An ordinary session id, so `turn_events` follows one task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_session_id: Option<String>,
    /// The task's own git branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// `[{ "text": String, "met": bool }]` — the checks the task has to satisfy. Passed through as
    /// JSON rather than typed, because it is the planner's shape and not the protocol's.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub micro_measurables: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// An edge: `downstream_task_id` may not start until `upstream_task_id` is done. Two tasks with no
/// edge between them run in parallel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZTaskDep {
    pub upstream_task_id: String,
    pub downstream_task_id: String,
}

/// A work's plan, as a graph: [`AttaccaApi::work_tasks`]'s answer. Empty until the work has been
/// through `planning`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZWorkTasks {
    #[serde(default)]
    pub tasks: Vec<ZTask>,
    #[serde(default)]
    pub deps: Vec<ZTaskDep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZTurnStatus {
    pub session_id: String,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cursor: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZDeltaKind {
    Assistant,
    Reasoning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZSessionEvent {
    /// The event's stable id. An event arriving with an id already held under another `seq` has
    /// moved (a message re-sequenced to where the turn received it): drop the old entry. Absent on a
    /// deployment that predates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub seq: i64,
    pub cursor: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    /// RFC 3339. Absent on a deployment that predates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// How far the answer reached the person when they interrupted. Sent with `cancel_turn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ZDelivered {
    /// The `chat_agent` event the node was delivering, as the `cursor` of the `Event` frame it
    /// received. `None`: the answer still streaming as `Delta` frames, not yet an event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<i64>,
    /// How much of that answer was delivered, in Unicode scalar values (`char`s).
    pub chars: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ZTurnFrame {
    Event { cursor: i64, event: ZSessionEvent },
    Delta { kind: ZDeltaKind, text: String },
    /// This turn was stopped, not finished. Sent before the `Status { running: false }` that
    /// ends it, and also sent when a delivery point cut an already-finished turn.
    Cancelled,
    Status { running: bool },
}

/// A sibling node's iroh address, as [`AttaccaApi::peer_lookup`] answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ZPeerAddr {
    pub node_id: String,
    /// The node path, `system/program/node` — the same string it was looked up by.
    pub path: String,
    /// iroh EndpointId — an ed25519 public key. The peer proves its identity with this.
    pub endpoint_id: String,
    /// Hole-punching candidate addresses.
    #[serde(default)]
    pub addrs: Vec<String>,
    /// The relay the deployment operates. Carried here rather than hardcoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    pub online: bool,
}

/// One entry of [`AttaccaApi::peer_list`]'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ZPeerEntry {
    pub node_id: String,
    /// The node path, `system/program/node` (`zyris::NodeAddress::path`). **It is a label, not a
    /// trust anchor.**
    ///
    /// Do not key a peer-key pin on the value this field comes back with, and do not key one on
    /// `node_id` either. Both are issued by the server, and the server is precisely the party a pin
    /// exists to constrain: it can present a substituted peer as a **new node**, which then arrives
    /// as "never seen before," passes any check, and gets pinned — working every time and leaving
    /// no trace.
    ///
    /// A path looks user-chosen but is not. Every segment is a slug the server derived and
    /// deduplicated: the system from what the approving person picked (pre-filled from the
    /// enrolling machine's own unverified hostname), the program from what the program called
    /// itself, and the node from what the connection asked for. A node's path is freed once its
    /// connection is gone past the resume grace, so a *different* connection can take it. "Same
    /// path, different key" is ordinary there, which is exactly the event a pin is supposed to
    /// catch.
    ///
    /// So the anchor cannot come from the server at all. It comes from a person: the peer's key
    /// fingerprint is confirmed once, out of band, and the pin binds to the key that was confirmed.
    /// The path's job is to say which peer the user meant, and nothing more.
    pub path: String,
    pub endpoint_id: String,
    pub online: bool,
    /// What the entry is. `Server` is attacca's own endpoint, which offers chat artifacts
    /// (`request_file`, an agent's send). A node accepts offers from either kind; a node UI should not
    /// show a `Server` entry as one of the person's machines, and nothing should be pinned under its
    /// path. Absent from a server that predates the field, which only ever listed nodes.
    #[serde(default)]
    pub kind: ZPeerKind,
}

/// What a [`ZPeerEntry`] is: one of the account's nodes, or attacca's own endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZPeerKind {
    #[default]
    Node,
    Server,
}

/// [`AttaccaApi::request_file`]'s answer: the offer attacca made over the peer link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ZFileOffered {
    pub transfer_id: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// What [`AttaccaApi::send_chat`] takes: a message into a session, as the web chat sends one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZChatMessage {
    pub session_id: String,
    pub message: String,
    /// The agent that answers this one message, in place of the session's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Let Attacca's router decide whether this is a reply here or a job to hand off. A routed
    /// message runs as a job, inside a thread when it needs a plan, and the answer says where.
    #[serde(default)]
    pub route: bool,
    /// Investigate, hand back a plan, and wait for approval before executing. Never routed.
    #[serde(default)]
    pub plan_mode: bool,
}

/// Where a routed message went, in [`ZTurnAck::routed`]. `job_id` is always set; the rest are set
/// when the router opened a thread for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZRouted {
    pub job_id: String,
    /// The job's own session — `turn_events` on it follows the work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
}

/// [`AttaccaApi::send_chat`]'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZTurnAck {
    /// A turn started on the session the message was sent to. False exactly when it was routed.
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routed: Option<ZRouted>,
}

/// A thread: a line of work made of jobs, with at most one live plan ([`ZGoal`]). A thread has no
/// session and no state of its own — talk to its jobs, and read its progress off its goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZThread {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The thread's agent. A member job may name its own. Absent once that agent is deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// One job row of a [`ZThreadSummary`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZThreadJobRow {
    pub id: String,
    pub title: String,
    pub state: ZJobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The job is verifying another job's result.
    #[serde(default)]
    pub is_verifying: bool,
    /// The job is waiting on a person.
    #[serde(default)]
    pub needs_user: bool,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<String>,
}

/// One entry of [`AttaccaApi::list_threads`]: the thread, with its plan's progress rolled up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZThreadSummary {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub state: ZGoalState,
    /// How a `closed` plan came to rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<ZGoalResting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    /// The goal text, once drafted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(default)]
    pub auto_handover: bool,
    #[serde(default)]
    pub milestone_total: u32,
    #[serde(default)]
    pub milestone_done: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_milestone_title: Option<String>,
    #[serde(default)]
    pub job_count: u32,
    #[serde(default)]
    pub running_job_count: u32,
    /// Human tasks still open: things only a person can do before the plan goes on.
    #[serde(default)]
    pub human_blocked_count: u32,
    /// Jobs waiting on an answer. `awaiting_question_session_id` is where to send one.
    #[serde(default)]
    pub awaiting_question_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awaiting_question_session_id: Option<String>,
    #[serde(default)]
    pub jobs: Vec<ZThreadJobRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// A file pinned to a thread. A resident artifact is handed to every job the thread starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZThreadArtifact {
    pub thread_id: String,
    pub file_id: String,
    pub resident: bool,
    pub position: i32,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// [`AttaccaApi::get_thread`]'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZThreadDetail {
    pub thread: ZThread,
    #[serde(default)]
    pub artifacts: Vec<ZThreadArtifact>,
    #[serde(default)]
    pub jobs: Vec<ZJob>,
}

/// What [`AttaccaApi::create_thread`] takes — the web composer's "Thread".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewThread {
    /// What the thread is for. Starts the planning job; without it only the thread is created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Defaults to `message`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Omit for the account's Main Agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Omit for the account's default project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Attachments for the planning job. Refused without a `message`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<Datum>,
}

/// A partial thread edit: an omitted field is left alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZThreadUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZThreadFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

/// What [`AttaccaApi::thread_follow_up`] takes: a new job in an existing thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZThreadFollowUp {
    pub message: String,
    /// The job this follows on from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_job_id: Option<String>,
    /// Omit for the thread's agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub plan_mode: bool,
    #[serde(default)]
    pub factcheck: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<Datum>,
}

/// [`AttaccaApi::thread_follow_up`]'s answer: the job started, and the next step recorded for it
/// when the thread has a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZFollowedUp {
    pub job: ZJob,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step: Option<ZNextStep>,
}

/// [`AttaccaApi::job_follow_up`]'s answer: the thread the job now belongs to — opened for it if it
/// stood alone — and the follow-up job started in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZJobFollowedUp {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub job: ZJob,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step: Option<ZNextStep>,
}

/// Where a thread's plan is. Derived, like [`ZJobState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZGoalState {
    /// The planning job is drafting the goal and its milestones.
    Planning,
    /// Next steps are ready and wait to be handed over — by hand, or by auto-handover.
    AwaitingHandover,
    Running,
    Closed,
    Failed,
}

/// How a closed plan came to rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZGoalResting {
    Complete,
    Failed,
}

/// A thread's plan: a measurable goal, milestones toward it, and next steps that become jobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZGoal {
    pub id: String,
    pub thread_id: String,
    pub thread_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// The goal text. Read it before [`AttaccaApi::approve_goal`] — it is what is being approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// RFC 3339. Absent until approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_approved_at: Option<String>,
    pub state: ZGoalState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<ZGoalResting>,
    /// The planning job's session: `send_message` to steer the plan, `turn_events` to watch it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_session_id: Option<String>,
    /// Whether a handed-over step is verified by default.
    #[serde(default)]
    pub verification_default: bool,
    /// Hand ready next steps over without waiting for a person.
    #[serde(default)]
    pub auto_handover: bool,
    /// How many verify-and-retry loops the plan may run.
    #[serde(default)]
    pub max_loops: i32,
    #[serde(default)]
    pub loop_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// A partial goal edit: an omitted field is left alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZGoalUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_default: Option<bool>,
    /// Prefer [`AttaccaApi::set_auto_handover`], which also advances the plan when turning it on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_handover: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_loops: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZMilestoneState {
    Pending,
    Active,
    Done,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZMilestone {
    pub id: String,
    pub goal_id: String,
    pub position: i32,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub state: ZMilestoneState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewMilestone {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Omit to append.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

/// A partial milestone edit: an omitted field is left alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZMilestoneUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<ZMilestoneState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZNextStepState {
    /// Drafted, not yet a job. Only a proposed step can be handed over.
    Proposed,
    /// Running as `job_id`.
    HandedOver,
    Blocked,
    Done,
    Dropped,
}

/// One unit of a plan, which becomes a job when handed over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNextStep {
    pub id: String,
    pub goal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_id: Option<String>,
    pub position: i32,
    pub title: String,
    /// What the job is told to do.
    pub body: String,
    /// What has to hold for the step to count as done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<String>,
    /// A verification job checks the result against `acceptance`.
    #[serde(default)]
    pub verification: bool,
    /// Steps that must be done first.
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub state: ZNextStepState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handed_over_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewNextStep {
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_id: Option<String>,
    /// Omit for the goal's `verification_default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

/// A partial next-step edit: an omitted field is left alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNextStepUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
    /// Replaces the whole list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZVerificationVerdict {
    Pass,
    ChangeRequest,
    NewNextStep,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZVerificationOutcome {
    Accepted,
    Dismissed,
}

/// A verification job's judgement of a handed-over step, waiting for a person to accept or dismiss.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZVerification {
    pub id: String,
    pub goal_id: String,
    pub next_step_id: String,
    /// The job whose result was checked.
    pub work_job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_job_id: Option<String>,
    /// Absent while the verification job is still running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<ZVerificationVerdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_request: Option<String>,
    /// The step a `new_next_step` verdict proposes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_next_step: Option<ZNewNextStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ZVerificationOutcome>,
    /// The plan's loop cap was reached, so this verdict does not loop again on its own.
    #[serde(default)]
    pub capped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZHumanTaskState {
    Open,
    Done,
    Dismissed,
}

/// Something only a person can do — a login, a purchase, a decision — that the plan waits on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZHumanTask {
    pub id: String,
    pub goal_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_id: Option<String>,
    /// A check Attacca can run to see whether the task is done, for `recheck_human_task`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_probe_ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_probe_output: Option<String>,
    pub state: ZHumanTaskState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZNewHumanTask {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
}

/// How [`AttaccaApi::resolve_human_task`] closes a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZHumanTaskResolution {
    Done,
    Dismissed,
}

/// [`AttaccaApi::recheck_human_task`]'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZProbeReport {
    /// `resolved`, `still_blocked`, `could_not_run`, `no_probe` or `already_resolved`. A string
    /// rather than an enum so a deployment may add one.
    pub status: String,
    pub ran: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default)]
    pub output: String,
    pub task: ZHumanTask,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step_state: Option<ZNextStepState>,
}

/// [`AttaccaApi::get_goal`]'s answer: the whole plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZGoalDetail {
    pub goal: ZGoal,
    #[serde(default)]
    pub milestones: Vec<ZMilestone>,
    #[serde(default)]
    pub next_steps: Vec<ZNextStep>,
    #[serde(default)]
    pub human_tasks: Vec<ZHumanTask>,
    #[serde(default)]
    pub verifications: Vec<ZVerification>,
}

/// Why auto-handover stopped advancing the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZAutoHalt {
    BlockedOnPerson,
    VerificationFailed,
    CreditsRefused,
    WaitingOnDependencies,
    InFlight,
    Idle,
}

/// What turning auto-handover on did straight away.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZAdvanceReport {
    /// Verifications accepted.
    #[serde(default)]
    pub accepted: Vec<String>,
    /// Next steps handed over.
    #[serde(default)]
    pub handed_over: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub halted: Option<ZAutoHalt>,
}

/// [`AttaccaApi::set_auto_handover`]'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZAutoHandover {
    pub goal: ZGoal,
    /// Absent when auto-handover was turned off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ZAdvanceReport>,
}

#[zyris::capability(name = "attacca_api", version = 1)]
pub trait AttaccaApi {
    /// Identify the account this connection is authorized as, and the scopes it was granted.
    /// Requires no scope of its own, so it answers even for a node granted nothing.
    async fn me(&self) -> zyris::Result<ZMe>;

    /// List the caller's agents.
    async fn list_agents(&self) -> zyris::Result<Vec<ZAgent>>;

    /// Create an agent.
    async fn create_agent(&self, agent: ZNewAgent) -> zyris::Result<ZAgent>;

    /// List the caller's projects: the default first, then the rest oldest-first. The default is
    /// created on demand, so this never comes back empty.
    async fn list_projects(&self) -> zyris::Result<Vec<ZProject>>;

    /// Read one project.
    async fn get_project(&self, project_id: String) -> zyris::Result<ZProject>;

    /// Create a project.
    async fn create_project(&self, project: ZNewProject) -> zyris::Result<ZProject>;

    /// Edit a project's name or description. Omitted fields are left alone.
    async fn update_project(
        &self,
        project_id: String,
        update: ZProjectUpdate,
    ) -> zyris::Result<ZProject>;

    /// Delete a project. Refused while it still holds sessions, boards or jobs, and refused outright
    /// for the account's default project — move or delete the contents first.
    async fn delete_project(&self, project_id: String) -> zyris::Result<()>;

    /// List the caller's sessions.
    async fn list_sessions(&self, filter: ZSessionFilter) -> zyris::Result<Vec<ZSession>>;

    /// Create a session. Kept for nodes built before `create_session_with`, which is the same call
    /// with room for the options added since.
    ///
    /// Legacy: Attacca's own UI no longer opens bare sessions. To start a conversation, prefer
    /// `create_job` (or `create_thread` for something that needs a plan).
    ///
    /// Pass `title` as null unless a human will go looking for this session under a name the node
    /// already knows: Attacca titles a session from its first message, and a title given here is
    /// permanent and suppresses that.
    async fn create_session(
        &self,
        agent_id: String,
        title: Option<String>,
        project_id: Option<String>,
    ) -> zyris::Result<ZSession>;

    /// Legacy, as `create_session` is; still the way to get a session with a `preamble`.
    ///
    /// Create a session, with options — most usefully a `preamble`, which gives this session its own
    /// system instructions on top of its agent's. Leave `title` unset unless the node has a name
    /// worth pinning; Attacca titles the session from its first message otherwise.
    async fn create_session_with(&self, session: ZNewSession) -> zyris::Result<ZSession>;

    /// A session's durable timeline, oldest-first: the same events `turn_events` streams, read back
    /// as a list, so a node that was not connected when they happened can still see them. Mind the
    /// one difference in `after`: omitting it here means the whole history, where in `turn_events`
    /// it means live frames only.
    async fn session_history(
        &self,
        session_id: String,
        query: ZHistoryQuery,
    ) -> zyris::Result<Vec<ZSessionEvent>>;

    /// What a session has cost so far: model, token counts, credits. Requires `sessions:read`.
    ///
    /// Added within version 1, so a node built against this crate may find it absent from an older
    /// deployment's announcement and get `capability_not_announced` back. Treat that as "this
    /// deployment does not meter" and carry on — it is not a connection-level failure.
    async fn session_usage(&self, session_id: String) -> zyris::Result<ZUsage>;

    /// Post a message, starting a turn. Stream results via `turn_events`.
    async fn send_message(
        &self,
        session_id: String,
        message: String,
        data: Vec<Datum>,
    ) -> zyris::Result<()>;

    /// [`AttaccaApi::send_message`], with the options added since — most usefully an agent that
    /// answers this message in place of the session's own. Carries no data: a message with files
    /// goes through `send_message`.
    ///
    /// Added within version 1, so an older deployment answers `capability_not_announced` or an
    /// unknown-tool error; a node falls back to `send_message` and the session's agent.
    async fn send_message_with(&self, message: ZNewMessage) -> zyris::Result<()>;

    /// Post a message the way the web chat does: with `route`, Attacca's router may run it as a job
    /// (inside a thread when it needs a plan) instead of a turn here, and the answer says where it
    /// went — follow `routed.job_session_id` with `turn_events`. `sessions:write`.
    ///
    /// Added within version 1; an older deployment answers `capability_not_announced`, and a node
    /// falls back to `send_message_with`.
    async fn send_chat(&self, message: ZChatMessage) -> zyris::Result<ZTurnAck>;

    /// Stop the running turn on a session, and keep only what the person got.
    ///
    /// Whatever answer text streamed before the stop is kept. With `delivered`, the stored answer is
    /// cut to what the node actually delivered: `cursor: None` cuts the answer still streaming;
    /// `cursor: Some(c)` cuts that `chat_agent` event and empties every later answer in its turn (a
    /// finished turn is cut too). A cursor that is unknown, stale, not a `chat_agent` event, or
    /// `cursor: None` with no turn running, is refused with `invalid_params`. `None` just stops.
    async fn cancel_turn(
        &self,
        session_id: String,
        delivered: Option<ZDelivered>,
    ) -> zyris::Result<()>;

    /// List the caller's jobs, newest first.
    async fn list_jobs(&self, filter: ZJobFilter) -> zyris::Result<Vec<ZJob>>;

    /// Read one job.
    async fn get_job(&self, job_id: String) -> zyris::Result<ZJob>;

    /// Queue a job and start it. The returned job is already running; watch it with `turn_events` on
    /// its `session_id`, or poll `get_job` for its state.
    async fn create_job(&self, job: ZNewJob) -> zyris::Result<ZJob>;

    /// Edit a job's title, description or project. Omitted fields are left alone.
    async fn update_job(&self, job_id: String, update: ZJobUpdate) -> zyris::Result<ZJob>;

    /// Delete a job.
    async fn delete_job(&self, job_id: String) -> zyris::Result<()>;

    /// Follow up on a finished job: the follow-up runs as a new job in the original's thread, which
    /// is opened for it if the original stood alone. A blank `message` is refused. `jobs:write`.
    async fn job_follow_up(
        &self,
        job_id: String,
        message: String,
        title: Option<String>,
    ) -> zyris::Result<ZJobFollowedUp>;

    // Threads and their plans (goals) are jobs grouped and planned, so they are scoped as jobs:
    // `jobs:read` to read, `jobs:write` for everything else. Added within version 1.

    /// List the caller's threads, most recently active first.
    async fn list_threads(&self, filter: ZThreadFilter) -> zyris::Result<Vec<ZThreadSummary>>;

    /// Read one thread, with its artifacts and jobs.
    async fn get_thread(&self, thread_id: String) -> zyris::Result<ZThreadDetail>;

    /// Open a thread — the web composer's "Thread". With a `message` it starts a planning job that
    /// drafts the thread's goal; the plan then waits at `approve_goal`.
    async fn create_thread(&self, thread: ZNewThread) -> zyris::Result<ZThread>;

    /// Edit a thread's title, description or project. Omitted fields are left alone.
    async fn update_thread(&self, thread_id: String, update: ZThreadUpdate) -> zyris::Result<ZThread>;

    /// Delete a thread.
    async fn delete_thread(&self, thread_id: String) -> zyris::Result<()>;

    /// Start a new job in a thread. Refused once the thread's plan has come to rest.
    async fn thread_follow_up(
        &self,
        thread_id: String,
        follow_up: ZThreadFollowUp,
    ) -> zyris::Result<ZFollowedUp>;

    /// Pin a file to a thread. A `resident` one is handed to every job the thread starts.
    async fn add_thread_artifact(
        &self,
        thread_id: String,
        file_id: String,
        resident: bool,
    ) -> zyris::Result<ZThreadArtifact>;

    /// Unpin a file from a thread. The file itself is kept.
    async fn remove_thread_artifact(&self, thread_id: String, file_id: String) -> zyris::Result<()>;

    /// Give a thread a plan of its own when it has none, optionally seeded with the goal text.
    async fn create_goal(&self, thread_id: String, goal: Option<String>) -> zyris::Result<ZGoal>;

    /// Read a plan whole: goal, milestones, next steps, human tasks, verifications.
    async fn get_goal(&self, goal_id: String) -> zyris::Result<ZGoalDetail>;

    /// Edit a plan's settings. Omitted fields are left alone.
    async fn update_goal(&self, goal_id: String, update: ZGoalUpdate) -> zyris::Result<ZGoal>;

    /// Delete a plan. The thread and its jobs are kept.
    async fn delete_goal(&self, goal_id: String) -> zyris::Result<()>;

    /// Replace the goal text. `None` clears it.
    async fn set_goal(&self, goal_id: String, goal: Option<String>) -> zyris::Result<ZGoal>;

    /// Approve the goal the planner drafted, letting the plan go on to hand over next steps. Read
    /// `goal` first — that is what is being approved; `send_message` to `planner_session_id` is
    /// how to ask for changes instead.
    async fn approve_goal(&self, goal_id: String) -> zyris::Result<ZGoal>;

    async fn create_milestone(
        &self,
        goal_id: String,
        milestone: ZNewMilestone,
    ) -> zyris::Result<ZMilestone>;

    async fn update_milestone(
        &self,
        goal_id: String,
        milestone_id: String,
        update: ZMilestoneUpdate,
    ) -> zyris::Result<ZMilestone>;

    async fn delete_milestone(&self, goal_id: String, milestone_id: String) -> zyris::Result<()>;

    async fn create_next_step(
        &self,
        goal_id: String,
        step: ZNewNextStep,
    ) -> zyris::Result<ZNextStep>;

    async fn update_next_step(
        &self,
        goal_id: String,
        next_step_id: String,
        update: ZNextStepUpdate,
    ) -> zyris::Result<ZNextStep>;

    async fn delete_next_step(&self, goal_id: String, next_step_id: String) -> zyris::Result<()>;

    /// Turn a `proposed` next step into a job. `verification` overrides the step's own flag.
    /// Refused for a step that is not `proposed`.
    async fn hand_over_next_step(
        &self,
        goal_id: String,
        next_step_id: String,
        verification: Option<bool>,
    ) -> zyris::Result<ZNextStep>;

    /// Turn auto-handover on or off. Turning it on advances the plan at once, as far as it can go.
    async fn set_auto_handover(&self, goal_id: String, enabled: bool) -> zyris::Result<ZAutoHandover>;

    /// Accept a verification's verdict.
    async fn accept_verification(
        &self,
        goal_id: String,
        verification_id: String,
    ) -> zyris::Result<ZVerification>;

    /// Dismiss a verification's verdict.
    async fn dismiss_verification(
        &self,
        goal_id: String,
        verification_id: String,
    ) -> zyris::Result<ZVerification>;

    async fn create_human_task(
        &self,
        goal_id: String,
        task: ZNewHumanTask,
    ) -> zyris::Result<ZHumanTask>;

    /// Close a human task as done or dismissed.
    async fn resolve_human_task(
        &self,
        goal_id: String,
        task_id: String,
        resolution: ZHumanTaskResolution,
    ) -> zyris::Result<ZHumanTask>;

    /// Run a human task's probe now, resolving the task if it passes.
    async fn recheck_human_task(
        &self,
        goal_id: String,
        task_id: String,
    ) -> zyris::Result<ZProbeReport>;

    /// Leave a note on a human task for the agent to read. `None` clears it.
    async fn set_human_task_note(
        &self,
        goal_id: String,
        task_id: String,
        note: Option<String>,
    ) -> zyris::Result<ZHumanTask>;

    // Works are legacy: Attacca no longer starts them from its UI, and a planned piece of work is
    // now a thread (`create_thread`). The tools stay for the works that exist.

    /// List the caller's works, newest first.
    async fn list_works(&self, filter: ZWorkFilter) -> zyris::Result<Vec<ZWork>>;

    /// Read one work.
    async fn get_work(&self, work_id: String) -> zyris::Result<ZWork>;

    /// Create a work and start planning it. It will settle at `awaiting_goal_approval` and go no
    /// further on its own — a node that creates a work and walks away has created something that
    /// never runs. See `approve_work_goal`.
    async fn create_work(&self, work: ZNewWork) -> zyris::Result<ZWork>;

    /// Edit a work's title, description or project. Omitted fields are left alone.
    async fn update_work(&self, work_id: String, update: ZWorkUpdate) -> zyris::Result<ZWork>;

    /// Delete a work.
    async fn delete_work(&self, work_id: String) -> zyris::Result<()>;

    /// Let a work through gate 1: accept the goal it drafted and start planning the task graph.
    /// Read `final_goal` and `requirements_report` off the work first — that is what is being
    /// approved, and `work_message` is how to ask for changes instead.
    async fn approve_work_goal(&self, work_id: String) -> zyris::Result<ZWork>;

    /// Let a work through gate 2: accept the task graph and begin executing it. Read it first with
    /// `work_tasks`.
    async fn approve_work_plan(&self, work_id: String) -> zyris::Result<ZWork>;

    /// A work's task graph. Empty before planning has run.
    async fn work_tasks(&self, work_id: String) -> zyris::Result<ZWorkTasks>;

    /// Stop a running work, and every task under it.
    async fn stop_work(&self, work_id: String) -> zyris::Result<()>;

    /// Resume a stopped, halted or failed work from where it left off: the tasks that already
    /// finished are kept, the rest are re-run.
    async fn continue_work(&self, work_id: String) -> zyris::Result<ZWork>;

    /// Say something to a work's planner — to steer the goal at gate 1 or the plan at gate 2.
    /// Exactly `send_message` against the work's `planner_session_id`, spelled so a node does not
    /// have to read the work first, and streamed back the same way.
    async fn work_message(
        &self,
        work_id: String,
        message: String,
        data: Vec<Datum>,
    ) -> zyris::Result<()>;

    /// Live turn feed with cursor resume: the head carries the current running flag and
    /// last cursor; items mirror LiveFrame (durable events with cursor, deltas, `cancelled`,
    /// status).
    #[zyris(uni_stream)]
    async fn turn_events(
        &self,
        session_id: String,
        after: Option<i64>,
    ) -> zyris::Result<Streaming<ZTurnStatus, ZTurnFrame>>;

    /// Publish this node's own iroh address. Call again whenever the address changes. `peers:write`.
    ///
    /// `endpoint_id` **keeps whatever value was published first.** A request to overwrite it with a
    /// different one is rejected — so the server never becomes a place that quietly swaps the key.
    async fn peer_publish(&self, endpoint_id: String, addrs: Vec<String>) -> zyris::Result<()>;

    /// Ask for another node's address on the same account. `peers:write`.
    ///
    /// **The lookup key is the node path** — `laptop/zyris-code/myrepo`, a
    /// `zyris::NodeAddress::path()`. It is how the caller names the peer it meant, and nothing
    /// more. See [`ZPeerEntry::path`] for why it cannot also be what a peer-key pin binds to: the
    /// anchor is a fingerprint a person confirmed out of band, not any name this server issues.
    ///
    /// A path is unique among live nodes, so there is at most one answer. A path no live node holds
    /// is refused, never answered with a near match.
    async fn peer_lookup(&self, path: String) -> zyris::Result<ZPeerAddr>;

    /// List the nodes on the same account. Used to decide whether an incoming connection may be
    /// accepted. `peers:write`.
    async fn peer_list(&self) -> zyris::Result<Vec<ZPeerEntry>>;

    /// attacca's own iroh relay, the one relay a zyris endpoint should use — build the endpoint with
    /// `zyris_p2p::endpoint::private_endpoint(secret, relay)`. `None` when this deployment runs none,
    /// in which case peers connect directly or not at all. `peers:write`.
    async fn peer_relay(&self) -> zyris::Result<Option<String>>;

    /// Have attacca offer a chat file to the calling node over the peer link. attacca dials this
    /// node's published peer address, offers the file, and the node pulls it into its inbox the way it
    /// receives from any peer; this returns once the offer was taken. Refused for a connection that is
    /// not a node, or a node that has not published a peer address. `files:read`.
    async fn request_file(&self, file_id: String) -> zyris::Result<ZFileOffered>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peer_entry_without_kind_is_a_node() {
        let entry: ZPeerEntry = serde_json::from_value(serde_json::json!({
            "node_id": "n", "path": "a/b/c", "endpoint_id": "e", "online": true
        }))
        .unwrap();
        assert_eq!(entry.kind, ZPeerKind::Node);
    }

    #[test]
    fn a_server_peer_entry_round_trips() {
        let entry = ZPeerEntry {
            node_id: "s".into(),
            path: "attacca/server/pod-1".into(),
            endpoint_id: "e".into(),
            online: true,
            kind: ZPeerKind::Server,
        };
        let wire = serde_json::to_value(&entry).unwrap();
        assert_eq!(wire["kind"], "server");
        assert_eq!(serde_json::from_value::<ZPeerEntry>(wire).unwrap(), entry);
    }

    #[test]
    fn a_session_event_round_trips_its_id() {
        let event = ZSessionEvent {
            id: Some("7d7c3f4e-0000-4000-8000-000000000001".into()),
            seq: 5,
            cursor: 9,
            kind: "chat_user".into(),
            payload: serde_json::json!({ "content": "hi" }),
            created_at: None,
        };
        let wire = serde_json::to_value(&event).unwrap();
        assert_eq!(wire["id"], "7d7c3f4e-0000-4000-8000-000000000001");
        let back: ZSessionEvent = serde_json::from_value(wire).unwrap();
        assert_eq!(back.id.as_deref(), Some("7d7c3f4e-0000-4000-8000-000000000001"));
    }

    #[test]
    fn a_session_event_from_an_older_server_decodes_without_an_id() {
        let back: ZSessionEvent = serde_json::from_value(serde_json::json!({
            "seq": 1, "cursor": 1, "kind": "chat_user", "payload": { "content": "hi" }
        }))
        .unwrap();
        assert_eq!(back.id, None);
    }
}
