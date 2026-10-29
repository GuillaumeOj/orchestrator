//! Presets (FR-306): installing and upgrading the starters, duplicating a
//! team or role, marking a team as a preset.
//!
//! An installed starter records its origin (starter id, starter version and
//! the fingerprint of what was installed). When the app's starters move to a
//! new [`STARTER_VERSION`], an installed copy whose fingerprint is unchanged
//! is replaced in place; a modified one is left alone and shows
//! `update_available`. Copies installed before origins existed (P2) are
//! recognised by the P2 fingerprints.

use anyhow::{Result, anyhow, bail};
use orch_protocol::now_ms;
use orch_team::presets::{STARTER_VERSION, V1_FINGERPRINTS, role_fingerprint, team_fingerprint};
use orch_team::{PermissionProfile, RoleDef, Scope, StarterOrigin, Team, valid_slug};
use serde_json::{Value, json};

use crate::runtime::Runtime;

fn v1_origin(kind: &str, id: &str) -> Option<StarterOrigin> {
    let key = format!("{kind}:{id}");
    V1_FINGERPRINTS.iter().find(|(k, _)| *k == key).map(|(_, fp)| StarterOrigin {
        starter: id.into(),
        version: 1,
        fingerprint: (*fp).into(),
    })
}

/// `teams_view`'s `preset`, `starter`, `update_available` of a team.
pub fn preset_fields(t: &Team) -> (bool, Option<String>, bool) {
    let starter = t.origin.as_ref().map(|o| o.starter.clone());
    let update = t.origin.as_ref().is_some_and(|o| o.version < STARTER_VERSION);
    (t.preset, starter, update)
}

/// What an installed copy is: unmodified since it was installed, or not.
fn unmodified_team(t: &Team) -> bool {
    t.origin.as_ref().is_some_and(|o| o.fingerprint == team_fingerprint(t))
}

/// A role the v0.11 auto-mode migration (or the user) moved from `gated` to
/// `autonomous` still counts as unmodified: that is the new default.
fn unmodified_role(r: &RoleDef) -> bool {
    let Some(o) = r.origin.as_ref() else { return false };
    if o.fingerprint == role_fingerprint(r) {
        return true;
    }
    let mut gated = r.clone();
    gated.permission_profile = PermissionProfile::Gated;
    r.permission_profile == PermissionProfile::Autonomous && o.fingerprint == role_fingerprint(&gated)
}

fn with_origin_role(mut r: RoleDef) -> RoleDef {
    r.origin =
        Some(StarterOrigin { starter: r.id.clone(), version: STARTER_VERSION, fingerprint: role_fingerprint(&r) });
    r
}

fn with_origin_team(mut t: Team) -> Team {
    t.origin =
        Some(StarterOrigin { starter: t.id.clone(), version: STARTER_VERSION, fingerprint: team_fingerprint(&t) });
    t.preset = true;
    t
}

/// A free id `<base>`, `<base>-2`… (≤ 40 chars) for which `taken` is false.
fn free_id(base: &str, taken: impl Fn(&str) -> Result<bool>) -> Result<String> {
    let base: String = base.chars().take(34).collect();
    for n in 1..1000 {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        if !taken(&id)? {
            return Ok(id);
        }
    }
    bail!("no free id for {base}")
}

impl Runtime {
    /// Installs the missing starters and upgrades the unmodified ones
    /// (FR-306). Returns how many were added or upgraded.
    pub fn install_presets(&self) -> Result<usize> {
        let now = now_ms();
        let mut n = 0;
        for r in orch_team::presets::starter_roles() {
            match self.db.role(&Scope::Global, &r.id)? {
                None => {
                    self.db.upsert_role(&with_origin_role(r), now)?;
                    n += 1;
                }
                Some(mut cur) => {
                    // A P2 copy: its origin is the P2 starter.
                    let legacy = cur.origin.is_none();
                    if legacy {
                        cur.origin = v1_origin("role", &cur.id);
                    }
                    let Some(o) = cur.origin.clone() else { continue };
                    if o.version < STARTER_VERSION && unmodified_role(&cur) {
                        let mut new = with_origin_role(r);
                        let same = role_fingerprint(&new) == role_fingerprint(&cur);
                        new.version = if same { cur.version } else { cur.version + 1 };
                        self.db.upsert_role(&new, now)?;
                        n += 1;
                    } else if legacy {
                        self.db.upsert_role(&cur, now)?; // record the origin, content unchanged
                    }
                }
            }
        }
        n += self.migrate_auto_mode(now)?;
        let roles = self.db.roles(None)?;
        for (_, t) in orch_team::presets::all() {
            match self.db.team(&Scope::Global, &t.id)? {
                None => {
                    self.db.save_team(&with_origin_team(t), &roles, now)?;
                    n += 1;
                }
                Some(mut cur) => {
                    let legacy = cur.origin.is_none();
                    if legacy {
                        cur.origin = v1_origin("team", &cur.id);
                        cur.preset = cur.origin.is_some();
                    }
                    let Some(o) = cur.origin.clone() else { continue };
                    if o.version < STARTER_VERSION && unmodified_team(&cur) {
                        let mut new = with_origin_team(t);
                        let same = team_fingerprint(&new) == team_fingerprint(&cur);
                        new.version = if same { cur.version } else { cur.version + 1 };
                        self.db.save_team(&new, &roles, now)?;
                        n += 1;
                    } else if legacy {
                        self.db.save_team(&cur, &roles, now)?;
                    }
                }
            }
        }
        Ok(n)
    }

    /// Once (v0.11): roles saved with the old default, `gated` (accept-edits,
    /// which stops on most commands), move to `autonomous` (auto mode), the
    /// new default. A role set to gated afterwards stays so. Runs after the
    /// starter upgrades, and a starter it moves still counts as unmodified.
    fn migrate_auto_mode(&self, now: u64) -> Result<usize> {
        const KEY: &str = "roles.auto_mode_v011";
        if self.db.setting(KEY)?.is_some() {
            return Ok(0);
        }
        let mut n = 0;
        let projects = self.db.projects(false)?;
        let scopes = std::iter::once(None).chain(projects.iter().map(|p| Some(p.id.as_str())));
        for scope in scopes {
            for mut r in self.db.roles(scope)? {
                if r.permission_profile == PermissionProfile::Gated {
                    r.permission_profile = PermissionProfile::Autonomous;
                    self.db.upsert_role(&r, now)?;
                    n += 1;
                }
            }
        }
        // Team agents already created get it at their next start (their
        // role is compiled again then).
        self.db.set_setting(KEY, &json!(now))?;
        Ok(n)
    }

    /// Re-copies the starter a team came from, when the team is unmodified
    /// since it was installed (a modified one is refused: duplicate it first).
    pub fn preset_update(&self, team_id: &str, scope: &Scope) -> Result<Team> {
        let cur = self.db.team(scope, team_id)?.ok_or_else(|| anyhow!("no team {team_id}"))?;
        let o = cur.origin.clone().ok_or_else(|| anyhow!("{team_id} is not a starter preset"))?;
        let (starter_roles, starter) = orch_team::presets::all()
            .into_iter()
            .find(|(_, t)| t.id == o.starter)
            .ok_or_else(|| anyhow!("the starter {} no longer exists", o.starter))?;
        if !unmodified_team(&cur) {
            bail!("{team_id} was modified since it was installed: duplicate it to keep your changes");
        }
        if o.version >= STARTER_VERSION {
            return Ok(cur);
        }
        // Its starter roles, where unmodified too.
        for r in starter_roles {
            if let Some(cr) = self.db.role(&Scope::Global, &r.id)?
                && cr.origin.as_ref().is_some_and(|ro| ro.version < STARTER_VERSION)
                && unmodified_role(&cr)
            {
                let mut new = with_origin_role(r);
                new.version = cr.version + 1;
                self.db.upsert_role(&new, now_ms())?;
            }
        }
        let mut new = with_origin_team(starter);
        new.id = cur.id.clone();
        new.scope = cur.scope.clone();
        new.version = cur.version + 1;
        self.db.save_team(&new, &self.db.roles(scope.project())?, now_ms())?;
        Ok(new)
    }

    /// FR-306 duplicate: a copy of `team` (in exactly `from`) into `to`, as
    /// `new_id` or `<id>-copy`, version 1. Roles it uses that `to` can't see
    /// are copied there too.
    pub fn team_duplicate(&self, team_id: &str, from: &Scope, to: &Scope, new_id: Option<&str>) -> Result<Value> {
        let src = self.db.team(from, team_id)?.ok_or_else(|| anyhow!("no team {team_id}"))?;
        let id = match new_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) if self.db.team(to, id)?.is_some() => bail!("a team {id} already exists there"),
            Some(id) => id.to_string(),
            None => free_id(&format!("{team_id}-copy"), |id| Ok(self.db.team(to, id)?.is_some()))?,
        };
        if !valid_slug(&id) {
            bail!("invalid team id {id:?}");
        }
        let visible_to = self.db.roles(to.project())?;
        let mut copied = vec![];
        for r in src.used_roles(&self.db.roles(from.project())?) {
            if visible_to.iter().any(|v| v.id == r.id) {
                continue;
            }
            let mut c = r.clone();
            (c.scope, c.version, c.origin) = (to.clone(), 1, None);
            self.db.upsert_role(&c, now_ms())?;
            copied.push(c.id);
        }
        let mut t = src.clone();
        t.id = id;
        t.name = format!("{} (copy)", src.name);
        (t.scope, t.version, t.preset, t.origin) = (to.clone(), 1, false, None);
        self.db.save_team(&t, &self.db.roles(to.project())?, now_ms())?;
        Ok(json!({"team": t, "roles_copied": copied}))
    }

    /// A copy of a starter exactly as the app ships it (not the installed,
    /// maybe edited copy) into `to`, as `new_id` or `<starter>-new`, version
    /// 1, not a preset. Its roles: one visible in `to` that says the same as
    /// the shipped one is used as is; a missing one is copied into `to`; an
    /// edited one is copied in as `<role>-<team id>` and the nodes use that.
    pub fn starter_copy(&self, starter: &str, to: &Scope, new_id: Option<&str>) -> Result<Value> {
        let (roles, team) = orch_team::presets::all()
            .into_iter()
            .find(|(_, t)| t.id == starter)
            .ok_or_else(|| anyhow!("no starter {starter}"))?;
        let id = match new_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) if self.db.team(to, id)?.is_some() => bail!("a team {id} already exists there"),
            Some(id) => id.to_string(),
            None => free_id(&format!("{starter}-new"), |id| Ok(self.db.team(to, id)?.is_some()))?,
        };
        if !valid_slug(&id) {
            bail!("invalid team id {id:?}");
        }
        let visible = self.db.roles(to.project())?;
        let mut renamed: std::collections::BTreeMap<String, String> = Default::default();
        let mut copied = vec![];
        for r in team.used_roles(&roles) {
            let rid = match visible.iter().find(|v| v.id == r.id) {
                Some(v) if role_fingerprint(v) == role_fingerprint(&r) => continue,
                Some(_) => {
                    let taken = |x: &str| Ok(visible.iter().any(|v| v.id == x) || self.db.role(to, x)?.is_some());
                    free_id(&format!("{}-{id}", r.id), taken)?
                }
                None => r.id.clone(),
            };
            let mut c = r.clone();
            (c.id, c.scope, c.version, c.origin) = (rid.clone(), to.clone(), 1, None);
            self.db.upsert_role(&c, now_ms())?;
            if rid != r.id {
                renamed.insert(r.id.clone(), rid.clone());
            }
            copied.push(rid);
        }
        let mut t = team;
        for n in &mut t.nodes {
            if let Some(nid) = renamed.get(&n.role_id) {
                n.role_id = nid.clone();
            }
        }
        t.id = id;
        (t.scope, t.version, t.preset, t.origin) = (to.clone(), 1, false, None);
        self.db.save_team(&t, &self.db.roles(to.project())?, now_ms())?;
        Ok(json!({"team": t, "roles_copied": copied}))
    }

    /// FR-306 duplicate of one role (`new_id` or `<id>-copy`, version 1).
    pub fn role_duplicate(&self, role_id: &str, from: &Scope, to: &Scope, new_id: Option<&str>) -> Result<RoleDef> {
        let src = self.db.role(from, role_id)?.ok_or_else(|| anyhow!("no role {role_id}"))?;
        let id = match new_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) if self.db.role(to, id)?.is_some() => bail!("a role {id} already exists there"),
            Some(id) => id.to_string(),
            None => free_id(&format!("{role_id}-copy"), |id| Ok(self.db.role(to, id)?.is_some()))?,
        };
        if !valid_slug(&id) {
            bail!("invalid role id {id:?}");
        }
        let mut r = src;
        r.id = id;
        r.name = format!("{} (copy)", r.name);
        (r.scope, r.version, r.origin) = (to.clone(), 1, None);
        self.db.upsert_role(&r, now_ms())?;
        Ok(r)
    }

    /// Shows or hides a team in the presets row (FR-306 "save as preset").
    pub fn team_set_preset(&self, team_id: &str, scope: &Scope, preset: bool) -> Result<Team> {
        let mut t = self.db.team(scope, team_id)?.ok_or_else(|| anyhow!("no team {team_id}"))?;
        t.preset = preset;
        self.db.save_team(&t, &self.db.roles(scope.project())?, now_ms())?;
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::test_runtime;

    /// The P2 install: the starters as P2 saved them (no origin, no preset flag).
    fn install_p2(rt: &Runtime, roles: &[RoleDef], teams: &[Team]) {
        for r in roles {
            rt.db.upsert_role(r, 1).unwrap();
        }
        for t in teams {
            rt.db.save_team(t, roles, 1).unwrap();
        }
    }

    /// Undoes the v4 models (the lead's Opus is older).
    fn without_v4_model(r: &mut RoleDef) {
        if r.id != "lead" {
            r.model = None;
        }
    }

    /// What P2 shipped, rebuilt: today's starters with the v2 changes undone.
    fn p2_starters() -> (Vec<RoleDef>, Vec<Team>) {
        let mut roles = orch_team::presets::starter_roles();
        for r in roles.iter_mut() {
            r.permission_profile = PermissionProfile::Gated;
            without_v4_model(r);
            if r.id == "reviewer" {
                r.prompt = "You are the reviewer. You review the branches you are asked to review and reply with \
                            concrete findings; you do not change them yourself."
                    .into();
            }
            if r.id == "ui" || r.id == "ux" {
                r.mcp_refs.clear();
                r.prompt = r.prompt.split("\n\nDesign hand-offs").next().unwrap().to_string();
            }
            if r.id == "lead" {
                r.prompt = r.prompt.split(" When the team has designers").next().unwrap().to_string();
            }
        }
        let teams = orch_team::presets::all()
            .into_iter()
            .filter(|(_, t)| t.id != "full-stack-reviewed")
            .map(|(_, mut t)| {
                t.edges.retain(|e| !(e.from == "ui" && e.to == "front"));
                t.preset = false;
                t
            })
            .collect();
        (roles, teams)
    }

    #[test]
    fn p2_rebuild_matches_the_recorded_fingerprints() {
        let (roles, teams) = p2_starters();
        for r in &roles {
            assert_eq!(Some(role_fingerprint(r)), v1_origin("role", &r.id).map(|o| o.fingerprint), "{}", r.id);
        }
        for t in &teams {
            assert_eq!(Some(team_fingerprint(t)), v1_origin("team", &t.id).map(|o| o.fingerprint), "{}", t.id);
        }
    }

    #[test]
    fn starter_copy_is_the_shipped_starter_even_after_edits() {
        let (_d, rt) = test_runtime(true);
        rt.install_presets().unwrap();
        // The user edited the installed ui role and the Design sprint team.
        let mut ui = rt.db.role(&Scope::Global, "ui").unwrap().unwrap();
        ui.prompt = "my own prompt".into();
        rt.db.upsert_role(&ui, 1).unwrap();
        let mut ds = rt.db.team(&Scope::Global, "design-sprint").unwrap().unwrap();
        ds.nodes[0].position = (5.0, 5.0);
        rt.db.save_team(&ds, &rt.db.roles(None).unwrap(), 1).unwrap();

        let v = rt.starter_copy("design-sprint", &Scope::Global, None).unwrap();
        let t: Team = serde_json::from_value(v["team"].clone()).unwrap();
        assert_eq!((t.id.as_str(), t.version, t.preset, t.origin.is_none()), ("design-sprint-new", 1, false, true));
        let (shipped_roles, shipped) =
            orch_team::presets::all().into_iter().find(|(_, t)| t.id == "design-sprint").unwrap();
        assert_eq!(t.nodes.iter().find(|n| n.id == shipped.nodes[0].id).unwrap().position, shipped.nodes[0].position);
        // The edited ui role is not used: the shipped one comes in under a new id.
        assert_eq!(v["roles_copied"], json!(["ui-design-sprint-new"]));
        let node_ui = t.nodes.iter().find(|n| n.role_id.starts_with("ui")).unwrap();
        assert_eq!(node_ui.role_id, "ui-design-sprint-new");
        let copy = rt.db.role(&Scope::Global, "ui-design-sprint-new").unwrap().unwrap();
        let shipped_ui = shipped_roles.iter().find(|r| r.id == "ui").unwrap();
        assert_eq!(copy.prompt, shipped_ui.prompt);
        assert_eq!(rt.db.role(&Scope::Global, "ui").unwrap().unwrap().prompt, "my own prompt", "left alone");

        // Into a project, with an id; the same id twice is refused.
        let p = Scope::Project("shop".into());
        let v = rt.starter_copy("solo-dev", &p, Some("mine")).unwrap();
        assert_eq!((v["team"]["id"].as_str(), v["roles_copied"].as_array().map(Vec::len)), (Some("mine"), Some(0)));
        assert!(rt.starter_copy("solo-dev", &p, Some("mine")).is_err());
        assert!(rt.starter_copy("nope", &p, None).is_err());
    }

    #[test]
    fn fresh_install_records_origins() {
        let (_d, rt) = test_runtime(true);
        assert_eq!(rt.install_presets().unwrap(), 11);
        assert_eq!(rt.install_presets().unwrap(), 0, "idempotent");
        let t = rt.db.team(&Scope::Global, "full-stack").unwrap().unwrap();
        assert_eq!(preset_fields(&t), (true, Some("full-stack".into()), false));
        assert_eq!(rt.db.role(&Scope::Global, "ui").unwrap().unwrap().mcp_refs, ["penpot"]);
    }

    #[test]
    fn unmodified_p2_starters_upgrade_in_place_modified_ones_get_a_hint() {
        let (_d, rt) = test_runtime(true);
        let (roles, mut teams) = p2_starters();
        // The user changed the Design sprint (moved a node) and nothing else.
        let ds = teams.iter_mut().find(|t| t.id == "design-sprint").unwrap();
        ds.nodes[0].position = (5.0, 5.0);
        install_p2(&rt, &roles, &teams);
        rt.install_presets().unwrap();

        let fs = rt.db.team(&Scope::Global, "full-stack").unwrap().unwrap();
        assert_eq!(fs.version, 2, "upgraded in place");
        assert!(orch_team::rules::can_handoff(&fs, "ui", "front"), "D2 edge arrived");
        assert_eq!(preset_fields(&fs), (true, Some("full-stack".into()), false));
        let ui = rt.db.role(&Scope::Global, "ui").unwrap().unwrap();
        assert_eq!((ui.version, ui.mcp_refs.as_slice()), (2, ["penpot".to_string()].as_slice()));
        let backend = rt.db.role(&Scope::Global, "backend").unwrap().unwrap();
        assert_eq!(
            (backend.version, backend.origin.unwrap().version),
            (2, STARTER_VERSION),
            "upgraded: auto mode is the v3 default"
        );

        let ds = rt.db.team(&Scope::Global, "design-sprint").unwrap().unwrap();
        assert_eq!((ds.version, ds.nodes[0].position), (1, (5.0, 5.0)), "a modified copy is left alone");
        assert_eq!(preset_fields(&ds), (true, Some("design-sprint".into()), true));
        assert!(rt.preset_update("design-sprint", &Scope::Global).unwrap_err().to_string().contains("modified"));
        let v = rt.teams_json(None).unwrap();
        let e = v["teams"].as_array().unwrap().iter().find(|t| t["team"]["id"] == "design-sprint").unwrap();
        assert_eq!(
            (e["preset"].as_bool(), e["starter"].as_str(), e["update_available"].as_bool()),
            (Some(true), Some("design-sprint"), Some(true))
        );

        // Saving from the canvas without the origin keeps it.
        let mut fs2 = fs.clone();
        fs2.origin = None;
        rt.save_team(fs2).unwrap();
        assert!(rt.db.team(&Scope::Global, "full-stack").unwrap().unwrap().origin.is_some());
    }

    /// The v3 starters: today's, without the v4 models.
    fn v3_roles() -> Vec<RoleDef> {
        let mut roles = orch_team::presets::starter_roles();
        roles.iter_mut().for_each(without_v4_model);
        roles
    }

    #[test]
    fn unmodified_v3_roles_upgrade_to_v4_modified_ones_do_not() {
        let (_d, rt) = test_runtime(true);
        for mut r in v3_roles() {
            if r.id == "frontend" {
                r.prompt = "my own prompt".into(); // the user's edit, after install
            }
            let fp = role_fingerprint(&v3_roles().into_iter().find(|v| v.id == r.id).unwrap());
            r.origin = Some(StarterOrigin { starter: r.id.clone(), version: 3, fingerprint: fp });
            rt.db.upsert_role(&r, 1).unwrap();
        }
        rt.install_presets().unwrap();
        let get = |id: &str| rt.db.role(&Scope::Global, id).unwrap().unwrap();
        let (backend, ui, lead) = (get("backend"), get("ui"), get("lead"));
        assert_eq!(
            (backend.model.as_deref(), backend.version, backend.origin.unwrap().version),
            (Some("sonnet"), 2, 4)
        );
        assert_eq!(ui.model.as_deref(), Some("opus"));
        assert_eq!(lead.version, 1, "lead unchanged: the version only goes up on a content change");
        let fe = get("frontend");
        assert_eq!((fe.model, fe.prompt.as_str(), fe.origin.unwrap().version), (None, "my own prompt", 3));
    }

    #[test]
    fn p2_starters_already_in_auto_mode_still_upgrade() {
        // A v0.11 install: main's migration moved the P2 starters to auto mode.
        let (_d, rt) = test_runtime(true);
        let (mut roles, teams) = p2_starters();
        for r in roles.iter_mut() {
            r.permission_profile = PermissionProfile::Autonomous;
        }
        install_p2(&rt, &roles, &teams);
        rt.db.set_setting("roles.auto_mode_v011", &json!(1)).unwrap();
        rt.install_presets().unwrap();
        let ui = rt.db.role(&Scope::Global, "ui").unwrap().unwrap();
        assert_eq!((ui.version, ui.mcp_refs.as_slice()), (2, ["penpot".to_string()].as_slice()));
        let backend = rt.db.role(&Scope::Global, "backend").unwrap().unwrap();
        assert_eq!(
            (backend.version, backend.permission_profile),
            (2, PermissionProfile::Autonomous),
            "the v4 model arrived"
        );
        assert_eq!(backend.origin.unwrap().version, STARTER_VERSION);
        let fs = rt.db.team(&Scope::Global, "full-stack").unwrap().unwrap();
        assert_eq!(preset_fields(&fs), (true, Some("full-stack".into()), false));
        assert!(rt.db.team(&Scope::Global, "full-stack-reviewed").unwrap().unwrap().preset);
    }

    #[test]
    fn preset_update_recopies_an_unmodified_old_starter() {
        let (_d, rt) = test_runtime(true);
        let (roles, teams) = p2_starters();
        install_p2(&rt, &roles, &teams);
        // A project copy recorded as v1 and unmodified (installed but not upgraded yet).
        let mut old = teams.iter().find(|t| t.id == "full-stack").unwrap().clone();
        old.scope = Scope::Project("p1".into());
        old.origin = v1_origin("team", "full-stack");
        rt.db.save_team(&old, &roles, 1).unwrap();
        let p1 = Scope::Project("p1".into());
        let new = rt.preset_update("full-stack", &p1).unwrap();
        assert_eq!((new.scope.clone(), new.version), (p1.clone(), 2));
        assert!(orch_team::rules::can_handoff(&new, "ui", "front"));
        assert_eq!(new.origin.as_ref().unwrap().version, STARTER_VERSION);
        assert_eq!(rt.preset_update("full-stack", &p1).unwrap().version, 2, "already current");
        assert!(rt.preset_update("nope", &p1).is_err());
    }

    #[test]
    fn duplicate_and_set_preset() {
        let (_d, rt) = test_runtime(true);
        rt.install_presets().unwrap();
        let p1 = Scope::Project("p1".into());
        // A starter into a project: the global roles are visible there, none copied.
        let v = rt.team_duplicate("full-stack", &Scope::Global, &p1, None).unwrap();
        assert_eq!(v["team"]["id"], "full-stack-copy");
        assert_eq!(v["team"]["version"], 1);
        assert_eq!(v["team"]["preset"], false);
        assert_eq!(v["team"]["name"], "Full-stack feature team (copy)");
        assert!(v["team"].get("origin").is_none());
        assert_eq!(v["roles_copied"], json!([]));
        assert_eq!(
            rt.team_duplicate("full-stack", &Scope::Global, &p1, None).unwrap()["team"]["id"],
            "full-stack-copy-2"
        );
        assert!(rt.team_duplicate("full-stack", &Scope::Global, &p1, Some("full-stack-copy")).is_err());
        assert!(rt.team_duplicate("full-stack", &Scope::Global, &p1, Some("Bad Id")).is_err());

        // A project team with a project role, copied to another project: the role goes too.
        let mut shop = RoleDef::new("shop", "Shop");
        shop.scope = p1.clone();
        rt.db.upsert_role(&shop, 1).unwrap();
        let mut t = rt.db.team(&p1, "full-stack-copy").unwrap().unwrap();
        t.nodes[1].role_id = "shop".into();
        rt.save_team(t).unwrap();
        let p2 = Scope::Project("p2".into());
        let v = rt.team_duplicate("full-stack-copy", &p1, &p2, Some("mine")).unwrap();
        assert_eq!(v["roles_copied"], json!(["shop"]));
        assert_eq!(rt.db.role(&p2, "shop").unwrap().unwrap().version, 1);

        let r = rt.role_duplicate("ui", &Scope::Global, &p2, None).unwrap();
        assert_eq!((r.id.as_str(), r.version, r.scope.clone(), r.origin.clone()), ("ui-copy", 1, p2.clone(), None));
        assert_eq!(r.mcp_refs, ["penpot"]);
        assert!(rt.role_duplicate("ghost", &Scope::Global, &p2, None).is_err());

        let t = rt.team_set_preset("mine", &p2, true).unwrap();
        assert!(t.preset);
        assert_eq!(rt.db.team(&p2, "mine").unwrap().unwrap().version, 1, "not a graph change");
        assert!(!rt.team_set_preset("mine", &p2, false).unwrap().preset);
    }
}
