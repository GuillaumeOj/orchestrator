use std::path::Path;

use governor::{AgentId, EntryId, PressureState, Tier, TurnKind};
use jiff::Timestamp;
use orch_store::TaskFlow;
use serde_json::json;

use super::*;
use crate::store::TeamSeat;

/// Wed 29 Oct 2025 09:26:40 UTC. Every time below is relative to it, and
/// clock times are formatted in UTC: the tests hold at any date and zone.
const NOW: u64 = 1_761_730_000_000;
const MIN: u64 = 60_000;

#[derive(Default)]
struct Fx {
    agents: BTreeMap<String, AgentRecord>,
    inbox: Vec<InboxItem>,
    live: BTreeSet<String>,
    queue: Vec<QueueEntry>,
    paused: BTreeSet<String>,
    ask: Option<AskFacts>,
    gov: GovFacts,
    runs: Vec<RunFacts>,
    tasks: Vec<Task>,
    messages: Vec<Message>,
    projects: BTreeMap<PathBuf, String>,
    done_today: u32,
}

impl Fx {
    fn world(&self) -> World {
        let tz = TimeZone::UTC;
        let open: Vec<InboxItem> = self.inbox.iter().filter(|i| i.open()).cloned().collect();
        let decisions = crate::decision::build(&crate::decision::Ctx {
            items: &open,
            agents: &self.agents,
            tails: &BTreeMap::new(),
            ask: self.ask.as_ref(),
            runs: &self.runs,
            projects: &self.projects,
            startup_timeout_ms: 30_000,
        });
        compute(
            &Inputs {
                agents: &self.agents,
                inbox: &self.inbox,
                live: &self.live,
                queue: &self.queue,
                paused: &self.paused,
                ask: self.ask.as_ref(),
                gov: &self.gov,
                runs: &self.runs,
                tasks: &self.tasks,
                messages: &self.messages,
                projects: &self.projects,
                startup_timeout_ms: 30_000,
                live_cap: 3,
                done_today: self.done_today,
                tz: &tz,
                decisions: &decisions,
            },
            NOW,
        )
    }

    fn rec(&self, id: &str) -> AgentAttention {
        self.world().agents[id].clone()
    }

    /// A solo agent in `state`, live unless created, suspended or archived.
    fn agent(&mut self, id: &str, state: AgentState) -> &mut AgentRecord {
        let mut a = crate::runtime::tests::test_agent(Path::new("/repo"));
        a.id = id.into();
        a.tracker.state = state;
        a.created_at_ms = NOW - 60 * MIN;
        a.last_event_at_ms = Some(NOW - 5 * MIN);
        if !matches!(state, AgentState::Created | AgentState::Suspended | AgentState::Archived | AgentState::Killed) {
            self.live.insert(id.into());
        }
        self.agents.insert(id.into(), a);
        self.agents.get_mut(id).unwrap()
    }

    /// A team member of run-1 on `node`, working on `task`.
    fn member(&mut self, id: &str, node: &str, state: AgentState, task: Option<u64>) -> &mut AgentRecord {
        self.ensure_run(RunState::Executing);
        let a = self.agent(id, state);
        a.team = Some(TeamSeat {
            run_id: "run-1".into(),
            node: node.into(),
            feature_id: "f1".into(),
            project_id: "shop".into(),
            task,
        });
        a
    }

    fn lead(&mut self, state: AgentState) -> &mut AgentRecord {
        let a = self.member("lead-1", "lead", state, None);
        a.role.orch_tools = vec!["merge".into()];
        a
    }

    fn ensure_run(&mut self, phase: RunState) {
        if self.runs.is_empty() {
            self.runs.push(RunFacts {
                run: TeamRun {
                    id: "run-1".into(),
                    team_id: "t".into(),
                    team_scope: Default::default(),
                    team_version: 1,
                    feature_id: "f1".into(),
                    status: phase,
                    resume_status: None,
                    pr_url: None,
                    started_at_ms: NOW - 120 * MIN,
                    ended_at_ms: None,
                },
                project: "shop".into(),
                feature: "checkout".into(),
                phase_since_ms: NOW - 30 * MIN,
            });
        }
    }

    fn phase(&mut self, phase: RunState) {
        self.ensure_run(phase);
        self.runs[0].run.status = phase;
    }

    fn item(&mut self, agent: &str, kind: &str, payload: Value, age_min: u64) -> u64 {
        let id = self.inbox.len() as u64 + 1;
        let mut it = InboxItem::new(id, kind, Some(agent), format!("{agent}: {kind}"));
        it.payload = payload;
        it.created_at_ms = NOW - age_min * MIN;
        self.inbox.push(it);
        id
    }

    fn entry(&mut self, agent: &str, reason: QueueReason, eta_min: Option<u64>) {
        let n = self.queue.len() as u64 + 1;
        self.queue.push(QueueEntry {
            id: EntryId(n),
            kind: TurnKind::Task,
            agent: AgentId(agent.into()),
            allocation: "default".into(),
            tier: Tier::Normal,
            enqueued_at: ts(NOW - 6 * MIN),
            effective_tier: Tier::Normal,
            reason,
            estimated_start: eta_min.map(|m| ts(NOW + m * MIN)),
            estimated_cost_pct: 0.5,
            estimated_cost_5h_pct: None,
            binding: Window::Weekly,
            self_started: false,
            ceiling: None,
            needs_spawn: false,
        });
    }

    fn task(&mut self, id: u64, node: &str, status: TaskStatus, deps: &[u64]) -> &mut Task {
        let mut t = Task::new("shop", "local", &format!("task {id}"), NOW - 40 * MIN);
        t.id = id;
        t.team_run_id = Some("run-1".into());
        t.feature_id = Some("f1".into());
        t.node = Some(node.into());
        t.status = status;
        t.depends_on = deps.to_vec();
        t.updated_at_ms = NOW - 20 * MIN;
        self.tasks.push(t);
        self.tasks.last_mut().unwrap()
    }

    fn message(&mut self, from: (&str, Option<&str>), to: &str, kind: &str, age_min: u64) {
        let id = self.messages.len() as u64 + 1;
        self.messages.push(Message {
            id,
            team_run_id: "run-1".into(),
            from_node: from.0.into(),
            from_agent: from.1.map(str::to_string),
            to_node: to.into(),
            to_agent: None,
            kind: kind.into(),
            body: "…".into(),
            r#ref: Value::Null,
            created_at_ms: NOW - age_min * MIN,
            delivered_at_ms: None,
            read_at_ms: None,
        });
    }
}

fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millisecond(ms as i64).unwrap()
}

/// (level, status, kind, ref, reason) of a record.
fn shape(r: &AgentAttention) -> (Level, Status, Option<WaitKind>, Option<String>, String) {
    let w = r.waiting_on.as_ref();
    (r.level, r.status, w.map(|w| w.kind), w.and_then(|w| w.r#ref.clone()), r.reason.clone())
}

fn s(v: &str) -> Option<String> {
    Some(v.into())
}

// ------------------------------------------------------------ rule 1: error

#[test]
fn rule_1_error() {
    let mut f = Fx::default();
    f.agent("a", AgentState::Error).tracker.last_error = Some("model_not_found".into());
    assert_eq!(shape(&f.rec("a")), (Level::Error, Status::Error, None, None, "model_not_found".into()));
    // A startup that never happened: kind startup, the decision as ref (D3).
    f.agent("b", AgentState::Starting);
    let id = f.item("b", "startup", json!({}), 2);
    assert_eq!(
        shape(&f.rec("b")),
        (
            Level::Error,
            Status::Error,
            Some(WaitKind::Startup),
            Some(decision_id("startup", id)),
            "no session after 30 s".into()
        )
    );
    f.agent("c", AgentState::Idle);
    f.item("c", "startup", json!({"failed": "resumed", "exit_status": 1}), 1);
    assert_eq!(f.rec("c").reason, "resumed start exited (status 1)");
    // An API error alert; an ordinary alert does not put the agent in error.
    f.agent("d", AgentState::Idle);
    f.inbox.push({
        let mut it = InboxItem::new(99, "alert", Some("d"), "d: API error 529".into());
        it.payload = json!({"alert": "api_error"});
        it.created_at_ms = NOW - MIN;
        it
    });
    assert_eq!(shape(&f.rec("d")), (Level::Error, Status::Error, None, None, "API error 529".into()));
    f.agent("e", AgentState::Idle);
    f.item("e", "alert", json!({}), 1);
    assert_eq!(f.rec("e").level, Level::Idle);
    // The since of an error item is its creation.
    assert_eq!(f.rec("b").since_ms, NOW - 2 * MIN);
    assert!(f.rec("b").since_stored);
}

#[test]
fn an_error_alert_from_before_the_running_turn_is_history() {
    let mut f = Fx::default();
    let a = f.agent("a", AgentState::Working);
    a.tracker.turn_started_ms = Some(NOW - MIN);
    f.item("a", "alert", json!({"action": "watchdog"}), 10);
    assert_eq!(f.rec("a").level, Level::Running);
    f.agents.get_mut("a").unwrap().tracker.state = AgentState::Idle;
    assert_eq!(f.rec("a").level, Level::Error);
}

// ------------------------------------------------------------ rule 2: needs you

#[test]
fn rule_2_needs_you_by_kind() {
    let mut f = Fx::default();
    f.agent("p", AgentState::WaitingForYou);
    let id = f.item("p", "permission", json!({"tool": "Bash", "tool_input": {"command": "pnpm add zod\nmore"}}), 2);
    let r = f.rec("p");
    assert_eq!(
        shape(&r),
        (
            Level::NeedsYou,
            Status::WaitingForYou,
            Some(WaitKind::User),
            Some(format!("inbox-{id}")),
            "permission: Bash pnpm add zod".into()
        )
    );
    assert_eq!(r.since_ms, NOW - 2 * MIN);
    // Parked.
    f.agent("pk", AgentState::Idle);
    f.item("pk", "permission", json!({"tool": "Edit", "tool_input": {"file_path": "src/a.rs"}}), 22);
    f.inbox.last_mut().unwrap().status = InboxStatus::Parked;
    assert_eq!(f.rec("pk").reason, "permission (parked): Edit src/a.rs");
    // An MCP tool: server and tool.
    f.agent("m", AgentState::WaitingForYou);
    f.item("m", "permission", json!({"tool": "mcp__penpot__execute_code", "tool_input": {}}), 1);
    assert_eq!(f.rec("m").reason, "permission: penpot execute_code");
    // Questions.
    f.agent("q", AgentState::Idle);
    let q = f.item("q", "question", json!({"question": "Which auth provider?\nDetails…"}), 4);
    assert_eq!(shape(&f.rec("q")).3, Some(format!("q-{q}")));
    assert_eq!(f.rec("q").reason, "question: Which auth provider?");
    f.agent("q3", AgentState::Idle);
    f.item("q3", "question", json!({"questions": [{"header": "Auth"}, {}, {}]}), 4);
    assert_eq!(f.rec("q3").reason, "3 questions: Auth");
    // Gates name the run's feature; status at gate.
    f.lead(AgentState::Idle);
    let g = f.item("lead-1", "gate", json!({"action": "spec_approval"}), 12);
    assert_eq!(
        shape(&f.rec("lead-1")),
        (
            Level::NeedsYou,
            Status::WaitingGate,
            Some(WaitKind::User),
            Some(format!("gate-{g}")),
            "spec gate · checkout".into()
        )
    );
    f.inbox.last_mut().unwrap().payload = json!({"action": "push_pr"});
    assert_eq!(f.rec("lead-1").reason, "PR gate · checkout");
    f.inbox.last_mut().unwrap().payload = json!({"action": "restart_gate", "detail": "git status not clean"});
    assert_eq!(f.rec("lead-1").reason, "restart gate · git status not clean");
    // Trust dialog and a prompt only in the pane.
    f.agent("t", AgentState::Starting).trust_dialog = true;
    assert_eq!(shape(&f.rec("t")).3, s("trust-t"));
    assert_eq!(f.rec("t").reason, "trust dialog in its pane");
    f.agent("w", AgentState::WaitingForYou);
    assert_eq!(shape(&f.rec("w")).3, s("wait-w"));
    assert_eq!(f.rec("w").reason, "a prompt in its pane");
    // The Governor's ask about an agent.
    f.agent("k", AgentState::Idle);
    f.ask = Some(AskFacts {
        id: 7,
        agent: s("k"),
        created_ms: NOW - MIN,
        cost_pp: Some(2.14),
        reason: "x".into(),
        effect: String::new(),
        deadline_ms: NOW,
        default_apply: false,
    });
    assert_eq!(
        shape(&f.rec("k")),
        (
            Level::NeedsYou,
            Status::WaitingForYou,
            Some(WaitKind::User),
            s("ask-7"),
            "run now? ≈ +2.1 pp of the week".into()
        )
    );
}

#[test]
fn rule_2_ranks_permission_question_gate_ask_trust_prompt_oldest_first() {
    let mut f = Fx::default();
    f.agent("a", AgentState::WaitingForYou).trust_dialog = true;
    assert_eq!(shape(&f.rec("a")).3, s("trust-a"), "trust beats the pane prompt");
    f.ask = Some(AskFacts {
        id: 1,
        agent: s("a"),
        created_ms: NOW,
        cost_pp: None,
        reason: "Run a now?".into(),
        effect: String::new(),
        deadline_ms: NOW,
        default_apply: false,
    });
    assert_eq!(shape(&f.rec("a")).3, s("ask-1"), "ask beats trust");
    let g = f.item("a", "gate", json!({"action": "spec_approval"}), 1);
    assert_eq!(shape(&f.rec("a")).3, Some(format!("gate-{g}")), "gate beats ask");
    let q = f.item("a", "question", json!({"question": "?"}), 1);
    assert_eq!(shape(&f.rec("a")).3, Some(format!("q-{q}")), "question beats gate");
    let p_new = f.item("a", "permission", json!({"tool": "Bash"}), 1);
    let p_old = f.item("a", "permission", json!({"tool": "Bash"}), 9);
    assert_ne!(p_new, p_old);
    assert_eq!(shape(&f.rec("a")).3, Some(format!("inbox-{p_old}")), "the oldest permission first");
}

// ------------------------------------------------------------ rules 3 and 4

#[test]
fn rule_3_limit() {
    let mut f = Fx {
        gov: GovFacts {
            exhausted: Some(Window::FiveHour),
            five_hour_resets_ms: Some(NOW + 6 * 60 * MIN + 33 * MIN + 20_000),
            ..Default::default()
        },
        ..Default::default()
    };
    f.agent("a", AgentState::BlockedByLimit);
    assert_eq!(
        shape(&f.rec("a")),
        (
            Level::Waiting,
            Status::BlockedByLimit,
            Some(WaitKind::Limit),
            s("five_hour"),
            "5h window full · resets 16:00".into()
        )
    );
    // The weekly reset, five days away: the weekday.
    f.gov = GovFacts {
        exhausted: Some(Window::Weekly),
        week_resets_ms: Some(NOW + 5 * 24 * 60 * MIN - 26 * MIN),
        ..Default::default()
    };
    assert_eq!(f.rec("a").reason, "week full · resets Mon 09:00");
}

#[test]
fn rule_4_queue_entries_with_position_and_eta() {
    let mut f = Fx::default();
    f.gov.live_autonomous = 3;
    f.agent("x", AgentState::Idle);
    f.agent("a", AgentState::Idle);
    f.entry("x", QueueReason::BehindQueue, None);
    f.entry("a", QueueReason::LiveCap { state: PressureState::Green, cap: 3 }, Some(54));
    let r = f.rec("a");
    assert_eq!(
        shape(&r),
        (Level::Queued, Status::Queued, Some(WaitKind::Governor), s("a"), "live cap 3/3 · 2nd in line · ≈10:20".into())
    );
    assert_eq!(r.since_ms, NOW - 6 * MIN);
    assert_eq!(f.rec("x").reason, "behind higher-priority work · next");
    // Not started: no process; no position.
    f.agent("n", AgentState::Created);
    f.entry("n", QueueReason::LiveCap { state: PressureState::Red, cap: 1 }, Some(54));
    assert_eq!(
        shape(&f.rec("n")),
        (Level::Queued, Status::NotStarted, Some(WaitKind::Governor), s("n"), "live cap 3/1 (Red) · ≈10:20".into())
    );
    // Created with no entry yet.
    f.agent("c", AgentState::Created);
    assert_eq!(shape(&f.rec("c")).1, Status::NotStarted);
    assert_eq!(f.rec("c").reason, "waiting for a slot");
}

#[test]
fn rule_4_paused_and_reasons_with_their_own_time() {
    let mut f = Fx::default();
    f.agent("a", AgentState::Idle);
    f.entry("a", QueueReason::PausedAll, None);
    assert_eq!(shape(&f.rec("a")).1, Status::Paused);
    assert_eq!(f.rec("a").reason, "Pause all");
    f.queue[0].reason = QueueReason::PausedAgent;
    assert_eq!(f.rec("a").reason, "by you");
    // Paused with no queued work.
    f.queue.clear();
    f.paused.insert("a".into());
    assert_eq!(
        (f.rec("a").level, f.rec("a").status, f.rec("a").reason),
        (Level::Queued, Status::Paused, "by you".into())
    );
    // A pause never hides a running turn.
    f.agents.get_mut("a").unwrap().tracker.state = AgentState::Working;
    assert_eq!(f.rec("a").level, Level::Running);
    // Own-time reasons: no position, no ETA.
    f.paused.clear();
    f.agents.get_mut("a").unwrap().tracker.state = AgentState::Idle;
    f.entry("a", QueueReason::ScheduledOffHours { starts_at: ts(NOW + 9 * 60 * MIN - 26 * MIN) }, Some(5));
    assert_eq!(f.rec("a").reason, "off-hours · starts 18:00");
    f.queue[0].reason = QueueReason::ModelWindow {
        model: "opus".into(),
        resets_at: ts(NOW + 5 * 24 * 60 * MIN - 26 * MIN),
        exhausted: true,
    };
    assert_eq!(f.rec("a").reason, "opus week full · starts Mon 09:00");
    f.queue[0].reason = QueueReason::HardCap;
    f.gov.hard_caps.insert("default".into(), (12.0, 12.0));
    assert_eq!(f.rec("a").reason, "default cap reached · 12 of 12 pp · next · ≈09:31");
    f.queue[0].reason = QueueReason::FiveHourGuard;
    f.gov.five_hour_pct = Some(86.4);
    assert_eq!(f.rec("a").reason, "5h guard · 5h window at 86 % · next · ≈09:31");
}

// ------------------------------------------------------------ rules 5 and 6

#[test]
fn rule_5_startup_then_rule_6_running() {
    let mut f = Fx::default();
    f.agent("a", AgentState::Starting).spawned_at_ms = Some(NOW - 12_000);
    assert_eq!(
        shape(&f.rec("a")),
        (Level::Waiting, Status::Working, Some(WaitKind::Startup), None, "starting · times out after 30 s".into())
    );
    assert_eq!(f.rec("a").since_ms, NOW - 12_000);
    // Past the timeout and no startup item yet: running ("starting").
    f.agents.get_mut("a").unwrap().spawned_at_ms = Some(NOW - 40_000);
    assert_eq!((f.rec("a").level, f.rec("a").activity.clone()), (Level::Running, s("starting")));
    let a = f.agent("w", AgentState::Working);
    a.tracker.turn_started_ms = Some(NOW - 3 * MIN);
    a.tracker.current_tool = s("Bash");
    a.tracker.tool_brief = s("cargo test");
    let r = f.rec("w");
    assert_eq!(
        (r.level, r.status, r.waiting_on.clone(), r.reason.as_str()),
        (Level::Running, Status::Working, None, "")
    );
    assert_eq!((r.activity.as_deref(), r.since_ms), (Some("Bash cargo test"), NOW - 3 * MIN));
    f.agents.get_mut("w").unwrap().tracker.current_tool = None;
    assert_eq!(f.rec("w").activity.as_deref(), Some("writing"));
}

#[test]
fn rule_6_a_long_mcp_call_is_an_external_wait() {
    let mut f = Fx::default();
    let a = f.agent("a", AgentState::Working);
    a.tracker.current_tool = s("mcp__penpot__execute_code");
    a.tracker.tool_started_ms = Some(NOW - 4 * MIN);
    assert_eq!(f.rec("a").level, Level::Running);
    assert_eq!(f.rec("a").activity.as_deref(), Some("penpot execute_code"));
    f.agents.get_mut("a").unwrap().tracker.tool_started_ms = Some(NOW - 7 * MIN);
    assert_eq!(
        shape(&f.rec("a")),
        (
            Level::Waiting,
            Status::Working,
            Some(WaitKind::External),
            s("mcp__penpot__execute_code"),
            "waiting on penpot execute_code".into()
        )
    );
    assert_eq!(f.rec("a").since_ms, NOW - 7 * MIN);
    // The orchestrator's own tools are not external.
    f.agents.get_mut("a").unwrap().tracker.current_tool = s("mcp__orch__commit");
    assert_eq!(f.rec("a").level, Level::Running);
}

// ------------------------------------------------------------ rule 7: waiting on an agent

#[test]
fn rule_7_review_plan_message_lead() {
    let mut f = Fx::default();
    f.lead(AgentState::Idle);
    f.member("back-1", "back", AgentState::Idle, Some(4));
    f.member("front-1", "front", AgentState::Working, Some(5));
    f.task(4, "back", TaskStatus::Review, &[]);
    f.task(5, "front", TaskStatus::InProgress, &[]);
    // A review asked from the lead.
    f.message(("back", Some("back-1")), "lead", "review_request", 3);
    assert_eq!(
        shape(&f.rec("back-1")),
        (Level::Waiting, Status::Idle, Some(WaitKind::Agent), s("lead-1"), "→ lead-1 · review of T-4".into())
    );
    assert_eq!(f.rec("back-1").since_ms, NOW - 3 * MIN);
    // Answered: no wait on the lead any more.
    f.message(("lead", Some("lead-1")), "back", "review", 1);
    assert_ne!(shape(&f.rec("back-1")).2, Some(WaitKind::Agent));
    // A plan the lead has not decided.
    f.tasks[0].status = TaskStatus::InProgress;
    f.tasks[0].flow = TaskFlow { plan_first: true, plan: s("do x"), ..Default::default() };
    assert_eq!(f.rec("back-1").reason, "→ lead-1 · plan for T-4");
    f.tasks[0].flow.plan_approved = true;
    // A message to front, unanswered.
    f.message(("back", Some("back-1")), "front", "message", 12);
    let r = f.rec("back-1");
    assert_eq!((shape(&r).3, r.reason.as_str(), r.since_ms), (s("front-1"), "→ front-1 · message", NOW - 12 * MIN));
    // A lead with tasks in progress: → the members working them.
    f.tasks[1].agent_id = s("front-1");
    let r = f.rec("lead-1");
    assert_eq!(
        shape(&r),
        (Level::Waiting, Status::Idle, Some(WaitKind::Agent), s("back-1"), "→ back-1, front-1 · T-4, T-5".into())
    );
}

#[test]
fn rule_7_order_review_before_plan_before_message() {
    let mut f = Fx::default();
    f.lead(AgentState::Idle);
    f.member("back-1", "back", AgentState::Idle, Some(4));
    f.task(4, "back", TaskStatus::Review, &[]).flow = TaskFlow { plan_first: true, plan: s("p"), ..Default::default() };
    f.message(("back", Some("back-1")), "lead", "review_request", 3);
    assert_eq!(f.rec("back-1").reason, "→ lead-1 · review of T-4");
    f.tasks[0].status = TaskStatus::InProgress;
    assert_eq!(f.rec("back-1").reason, "→ lead-1 · plan for T-4");
    f.tasks[0].flow.plan = None;
    assert_eq!(f.rec("back-1").reason, "→ lead-1 · review", "a review asked outside Review is a message");
    // A message sent before its last turn is not a wait.
    f.agents.get_mut("back-1").unwrap().tracker.turn_started_ms = Some(NOW - MIN);
    assert_eq!(f.rec("back-1").level, Level::Idle);
}

#[test]
fn rule_7_lead_list_past_two() {
    assert_eq!(
        copy::lead_waits(&["a".into(), "b".into(), "c".into()], &["T-1".into(), "T-2".into(), "T-3".into()]),
        "→ a, b +1 more · T-1, T-2 +1 more"
    );
}

// ------------------------------------------------------------ rules 8 to 10

#[test]
fn rule_8_blocked_by_a_task() {
    let mut f = Fx::default();
    f.member("back-1", "back", AgentState::Working, Some(2));
    f.member("front-1", "front", AgentState::Idle, None);
    f.task(2, "back", TaskStatus::InProgress, &[]).agent_id = s("back-1");
    f.task(3, "front", TaskStatus::Ready, &[2]);
    let r = f.rec("front-1");
    assert_eq!(
        shape(&r),
        (Level::Waiting, Status::Idle, Some(WaitKind::Task), s("T-2"), "blocked by T-2 (back-1, in progress)".into())
    );
    // The dependency done: nothing blocks it.
    f.tasks[0].status = TaskStatus::Done;
    assert_eq!(f.rec("front-1").level, Level::Idle);
}

#[test]
fn rule_9_done_and_rule_10_idle() {
    let mut f = Fx::default();
    f.member("back-1", "back", AgentState::Idle, None);
    f.task(2, "back", TaskStatus::Done, &[]);
    assert_eq!(shape(&f.rec("back-1")).0, Level::Done);
    f.task(3, "back", TaskStatus::Ready, &[]);
    assert_eq!(shape(&f.rec("back-1")).0, Level::Idle, "a task left open");
    f.phase(RunState::Done);
    assert_eq!(shape(&f.rec("back-1")).1, Status::Done, "the run is done");
    f.agent("arch", AgentState::Archived);
    assert_eq!((f.rec("arch").level, f.rec("arch").live), (Level::Done, false));
    f.agent("solo", AgentState::Idle);
    assert_eq!(shape(&f.rec("solo")), (Level::Idle, Status::Idle, None, None, String::new()));
    f.agent("sus", AgentState::Suspended);
    assert_eq!(shape(&f.rec("sus")).1, Status::Suspended);
    f.agent("k", AgentState::Killed);
    assert_eq!(shape(&f.rec("k")).1, Status::Suspended);
}

// ------------------------------------------------------------ precedence

#[test]
fn precedence_between_adjacent_rules() {
    let mut f = Fx::default();
    // 1 > 2: an error beats a permission.
    f.agent("a", AgentState::Error);
    f.item("a", "permission", json!({"tool": "Bash"}), 1);
    assert_eq!(f.rec("a").level, Level::Error);
    // 2 > 3: a question beats a limit.
    f.agent("b", AgentState::BlockedByLimit);
    f.item("b", "question", json!({"question": "?"}), 1);
    assert_eq!(f.rec("b").level, Level::NeedsYou);
    // 3 > 4: a limit beats a queue entry.
    f.agent("c", AgentState::BlockedByLimit);
    f.entry("c", QueueReason::BehindQueue, None);
    assert_eq!(f.rec("c").status, Status::BlockedByLimit);
    // 4 > 7: queued work beats a wait on an agent.
    f.lead(AgentState::Idle);
    f.member("d", "back", AgentState::Idle, Some(9));
    f.task(9, "back", TaskStatus::InProgress, &[]);
    f.message(("back", Some("d")), "front", "message", 2);
    assert_eq!(f.rec("d").level, Level::Waiting);
    f.entry("d", QueueReason::BehindQueue, None);
    assert_eq!(f.rec("d").level, Level::Queued);
    // 5/6 > 4: a running turn with its next turn queued is running.
    f.agent("e", AgentState::Working);
    f.entry("e", QueueReason::BehindQueue, None);
    assert_eq!(f.rec("e").level, Level::Running);
    // A held tool call (state queued) is queued.
    f.agents.get_mut("e").unwrap().tracker.state = AgentState::Queued;
    assert_eq!(f.rec("e").level, Level::Queued);
    // 7 > 8: a wait on an agent beats a blocked next task.
    f.member("g", "front", AgentState::Idle, Some(10));
    f.task(10, "front", TaskStatus::Review, &[]);
    f.task(11, "front", TaskStatus::Ready, &[10]);
    f.message(("front", Some("g")), "lead", "review_request", 1);
    assert_eq!(shape(&f.rec("g")).2, Some(WaitKind::Agent));
    // 8 > 9 is structural (a blocked task is not done); 9 > 10: done beats idle.
}

#[test]
fn needs_you_iff_waiting_on_user() {
    let mut f = Fx::default();
    for (i, st) in
        [AgentState::WaitingForYou, AgentState::Idle, AgentState::Working, AgentState::Error, AgentState::Created]
            .into_iter()
            .enumerate()
    {
        f.agent(&format!("a{i}"), st);
    }
    f.item("a1", "permission", json!({}), 1);
    f.item("a3", "startup", json!({}), 1);
    f.agent("t", AgentState::Idle).trust_dialog = true;
    for r in f.world().agents.values() {
        assert_eq!(
            r.level == Level::NeedsYou,
            r.waiting_on.as_ref().is_some_and(|w| w.kind == WaitKind::User),
            "{r:?}"
        );
        if r.level == Level::Running || r.level == Level::Idle || r.level == Level::Done {
            assert!(r.waiting_on.is_none(), "{r:?}");
        }
    }
}

// ------------------------------------------------------------ runs and counts

#[test]
fn run_record_level_blocker_and_counts() {
    let mut f = Fx::default();
    f.lead(AgentState::Idle);
    f.member("back-1", "back", AgentState::Working, Some(1));
    f.member("front-1", "front", AgentState::WaitingForYou, Some(2));
    f.task(1, "back", TaskStatus::InProgress, &[]).agent_id = s("back-1");
    f.task(2, "front", TaskStatus::InProgress, &[]).agent_id = s("front-1");
    let p = f.item("front-1", "permission", json!({"tool": "Bash", "tool_input": {"command": "pnpm add zod"}}), 2);
    let w = f.world();
    let r = &w.runs["run-1"];
    assert_eq!((r.level, r.phase, r.phase_since_ms), (Level::NeedsYou, RunState::Executing, NOW - 30 * MIN));
    assert_eq!(r.members, ["back-1", "front-1", "lead-1"]);
    let b = r.blocker.as_ref().unwrap();
    assert_eq!(
        (b.agent.as_str(), b.waiting_on.kind, b.reason.as_str()),
        ("front-1", WaitKind::User, "permission: Bash pnpm add zod")
    );
    assert_eq!(b.waiting_on.r#ref, Some(format!("inbox-{p}")));
    assert_eq!(
        (r.counts.needs_you, r.counts.decisions, r.counts.running, r.counts.waiting, r.counts.live),
        (1, 1, 1, 1, 3)
    );
    assert_eq!(r.counts.live_cap, None);
    // A wait inside the run is no blocker; a gate phase makes the run need you.
    f.inbox.clear();
    f.agents.get_mut("front-1").unwrap().tracker.state = AgentState::Working;
    assert_eq!(f.world().runs["run-1"].blocker, None, "the lead waits on members: internal");
    assert_eq!(f.world().runs["run-1"].level, Level::Running);
    f.phase(RunState::PrGate);
    assert_eq!(f.world().runs["run-1"].level, Level::NeedsYou);
}

#[test]
fn counts_agree_for_the_demo_fixture() {
    // spec §6.1: 1 permission, 1 question, 1 queued, 1 not started, 2 running.
    let mut f = Fx::default();
    f.projects.insert("/repo".into(), "blog".into());
    f.agent("perm", AgentState::WaitingForYou);
    f.item("perm", "permission", json!({"tool": "Bash"}), 1);
    f.agent("ques", AgentState::Idle);
    f.item("ques", "question", json!({"question": "?"}), 1);
    f.agent("q", AgentState::Idle);
    f.entry("q", QueueReason::BehindQueue, None);
    f.agent("ns", AgentState::Created);
    f.agent("r1", AgentState::Working);
    f.agent("r2", AgentState::Working);
    f.agent("arch", AgentState::Archived);
    f.done_today = 5;
    let w = f.world();
    let g = &w.counts.global;
    assert_eq!((g.needs_you, g.decisions, g.queued, g.running, g.idle, g.error), (2, 2, 2, 2, 0, 0));
    assert_eq!((g.live, g.live_cap, g.done_today), (5, Some(3), 5), "archived and not-started agents are not live");
    let p = &w.counts.projects["blog"];
    assert_eq!((p.needs_you, p.decisions, p.queued, p.live, p.live_cap), (2, 2, 2, 5, None));
    // Decisions without an agent count globally; the Governor ask too.
    f.inbox.push(InboxItem::new(50, "budget", None, "reserve used up".into()));
    f.inbox.push(InboxItem::new(51, "info", None, "digest".into()));
    f.ask = Some(AskFacts {
        id: 1,
        agent: None,
        created_ms: NOW,
        cost_pp: None,
        reason: "x".into(),
        effect: String::new(),
        deadline_ms: NOW,
        default_apply: false,
    });
    let w = f.world();
    assert_eq!((w.counts.global.decisions, w.counts.projects["blog"].decisions), (4, 2));
    // Synthesized decisions: a pane prompt with no item, an agent in error.
    f.agent("pane", AgentState::WaitingForYou);
    f.agent("err", AgentState::Error);
    assert_eq!(f.world().counts.global.decisions, 6);
}

// ------------------------------------------------------------ copy and time

#[test]
fn clock_times_and_positions() {
    let tz = TimeZone::UTC;
    assert_eq!(clock(NOW + 54 * MIN, NOW, &tz), "10:20");
    assert_eq!(clock(NOW + 5 * 24 * 60 * MIN, NOW, &tz), "Mon 09:26");
    assert_eq!(clock(NOW + 10 * 24 * 60 * MIN, NOW, &tz), "8 Nov 09:26");
    assert_eq!(clock(NOW - 24 * 60 * MIN, NOW, &tz), "28 Oct 09:26", "the past never reads as a weekday");
    let fixed = TimeZone::fixed(jiff::tz::offset(9));
    assert_eq!(clock(NOW, NOW, &fixed), "18:26");
    let pos: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22].iter().map(|n| copy::position(*n)).collect();
    assert_eq!(
        pos,
        [
            "next",
            "2nd in line",
            "3rd in line",
            "4th in line",
            "11th in line",
            "12th in line",
            "13th in line",
            "21st in line",
            "22nd in line"
        ]
    );
    assert_eq!(copy::cut("  \nabcdef", 4), "abc…");
}

#[test]
fn records_serialize_as_the_contract() {
    let mut f = Fx::default();
    f.member("front-1", "front", AgentState::WaitingForYou, Some(4));
    f.task(4, "front", TaskStatus::InProgress, &[]);
    f.item("front-1", "permission", json!({"tool": "Bash", "tool_input": {"command": "pnpm add zod"}}), 2);
    let w = f.world();
    let v = serde_json::to_value(&w.agents["front-1"]).unwrap();
    assert_eq!(
        v,
        json!({
            "agent": "front-1", "project": "shop", "run": "run-1", "node": "front", "task": 4,
            "level": "needs_you", "status": "waiting_for_you",
            "waiting_on": {"kind": "user", "ref": "inbox-1", "since_ms": NOW - 2 * MIN},
            "reason": "permission: Bash pnpm add zod", "live": true, "since_ms": NOW - 2 * MIN,
        })
    );
    let r = serde_json::to_value(&w.runs["run-1"]).unwrap();
    assert_eq!(r["phase"], "executing");
    assert_eq!(r["blocker"]["waiting_on"]["kind"], "user");
    assert!(r["counts"].get("live_cap").is_none());
    assert_eq!(serde_json::to_value(&w.counts).unwrap()["global"]["live_cap"], 3);
}

/// NFR-1b: 30 agents and 10 runs in under 1 ms (release; debug is looser).
/// `cargo test -p agent-runtime --release compute_bench -- --nocapture` prints it.
#[test]
fn compute_bench_30_agents_10_runs() {
    let mut f = Fx::default();
    f.ensure_run(RunState::Executing);
    let base = f.runs[0].clone();
    f.runs = (0..10)
        .map(|n| {
            let mut r = base.clone();
            r.run.id = format!("run-{n}");
            r
        })
        .collect();
    for n in 0..30u64 {
        let id = format!("a{n}");
        let state = [AgentState::Working, AgentState::Idle, AgentState::WaitingForYou][n as usize % 3];
        let a = f.agent(&id, state);
        a.team = Some(TeamSeat {
            run_id: format!("run-{}", n % 10),
            node: format!("n{}", n % 3),
            feature_id: "f1".into(),
            project_id: "shop".into(),
            task: Some(n),
        });
        let run = format!("run-{}", n % 10);
        let t = f.task(n, &format!("n{}", n % 3), TaskStatus::InProgress, &[n.saturating_sub(1)]);
        t.team_run_id = Some(run.clone());
        if n % 3 == 2 {
            f.item(&id, "permission", json!({"tool": "Bash", "tool_input": {"command": "ls"}}), 1);
        }
        if n % 5 == 0 {
            f.entry(&id, QueueReason::BehindQueue, Some(10));
        }
        for k in 0..3 {
            f.message((&format!("n{}", n % 3), Some(&id)), &format!("n{}", (n + k) % 3), "message", k);
            f.messages.last_mut().unwrap().team_run_id = run.clone();
        }
    }
    let _ = f.world(); // warm up
    let n = 200;
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(f.world());
    }
    let per = t.elapsed() / n;
    println!("attention::compute, 30 agents + 10 runs: {per:?} per call");
    let limit =
        if cfg!(debug_assertions) { std::time::Duration::from_millis(10) } else { std::time::Duration::from_millis(1) };
    assert!(per < limit, "{per:?}");
}
