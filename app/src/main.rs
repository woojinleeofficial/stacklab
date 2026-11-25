//! StackLab desktop app (STEP 3 skeleton): dashboard (UI-01, minimal), problem training (UI-03)
//! and problem result / rewind (UI-04). Matches, openers, settings and statistics come later.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod draw;
mod matchplay;
mod strings;

use draw::{field, mini_piece, FieldView};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Vec2};
use matchplay::{MatchKind, MatchPlay};
use stacklab_engine::game::{Handling, InputEvent, Key};
use stacklab_engine::rng::Rng;
use stacklab_engine::{Ruleset, Spin};
use stacklab_store::Store;
use stacklab_training::adaptive::{
    anchor_bucket, diagnosis_plan, plan_cycle, Cycle, SkillProfile, SlotResult,
};
use stacklab_training::policy::Policy;
use stacklab_training::problem::{Goal, Module, Problem, WitnessStep};
use stacklab_training::service::ProblemService;
use stacklab_training::session::{AttemptResult, AttemptSession, FailReason, Status};
use stacklab_training::set::{
    opener_built, select_opponent, MatchProfile, MiniStatus, Observations, Phase, RoundRecord,
    SetState,
};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use strings::Strings;

/// Requests to the background problem worker (generation can take up to seconds, e.g. PC).
enum Request {
    Slot(Module, Box<SkillProfile>, u64),
    Anchor(Module, u64),
}

fn spawn_worker(
    rs: Arc<Ruleset>,
    policy: Policy,
    ctx: egui::Context,
) -> (Sender<Request>, Receiver<Option<Problem>>) {
    let (tx, rx) = channel::<Request>();
    let (otx, orx) = channel();
    std::thread::spawn(move || {
        let mut svc = ProblemService::new(rs, policy);
        for req in rx {
            let p = match req {
                Request::Slot(m, profile, seed) => {
                    let mut rng = Rng::new(seed);
                    svc.next_for(&profile, m, &mut rng).map(|(p, _)| p)
                }
                Request::Anchor(m, k) => svc.anchor(m, k),
            };
            if otx.send(p).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    (tx, orx)
}

/// Default key map (TETR.IO-like defaults). Settings screen arrives in STEP 7.
fn keymap() -> Vec<(egui::Key, Key)> {
    vec![
        (egui::Key::ArrowLeft, Key::Left),
        (egui::Key::ArrowRight, Key::Right),
        (egui::Key::ArrowDown, Key::SoftDrop),
        (egui::Key::Space, Key::HardDrop),
        (egui::Key::ArrowUp, Key::Cw),
        (egui::Key::X, Key::Cw),
        (egui::Key::Z, Key::Ccw),
        (egui::Key::A, Key::Flip),
        (egui::Key::C, Key::Hold),
    ]
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SavedCycle {
    cycle: Cycle,
    diagnosis: bool,
    number: u32,
    #[serde(default = "SetState::new_first")]
    set: SetState,
}

/// UI-06 data of the last evaluation.
#[derive(Clone)]
struct EvalReport {
    wins: u32,
    losses: u32,
    before: Option<(f64, f64)>,
    after: Option<(f64, f64)>,
    weaknesses: Vec<&'static str>,
}

struct Playing {
    session: AttemptSession,
    first: Option<AttemptResult>,
    hint: Option<WitnessStep>,
    answer: Vec<WitnessStep>,
    sim_time: f64,
    pending: Vec<InputEvent>,
    finished: bool,
}

enum Screen {
    Dashboard,
    Loading,
    Play(Box<Playing>),
    Openers,
    Match(Box<MatchPlay>),
    /// UI-08: event id and whether the confirmation step (2nd step) is shown.
    Recommend(String, bool),
}

struct StackApp {
    s: Strings,
    policy: Policy,
    store: Option<Store>,
    profile: SkillProfile,
    saved: SavedCycle,
    rng: Rng,
    worker: (Sender<Request>, Receiver<Option<Problem>>),
    screen: Screen,
    handling: Handling,
    last_report: Option<String>,
    autoshot: Option<(String, u32)>,
    library: stacklab_opener::library::Library,
    coverage: std::collections::BTreeMap<String, stacklab_opener::library::CoverageRecord>,
    pick: (String, String),
    opener_msg: Option<String>,
    matchp: MatchProfile,
    bots: stacklab_bot::Calibration,
    tl_input: (f64, f64),
    tl_msg: Option<String>,
    eval_report: Option<EvalReport>,
    rs: Arc<Ruleset>,
    rec_msg: Option<String>,
}

fn data_dir() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("StackLab")
}

fn install_korean_font(ctx: &egui::Context) {
    let candidates = [
        "C:\\Windows\\Fonts\\malgun.ttf",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansKR-Regular.ttf",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            ctx.add_font(egui::epaint::text::FontInsert::new(
                "korean",
                egui::FontData::from_owned(bytes),
                vec![
                    egui::epaint::text::InsertFontFamily {
                        family: egui::FontFamily::Proportional,
                        priority: egui::epaint::text::FontPriority::Lowest,
                    },
                    egui::epaint::text::InsertFontFamily {
                        family: egui::FontFamily::Monospace,
                        priority: egui::epaint::text::FontPriority::Lowest,
                    },
                ],
            ));
            return;
        }
    }
}

impl StackApp {
    fn new(cc: &eframe::CreationContext<'_>) -> StackApp {
        install_korean_font(&cc.egui_ctx);
        let policy = Policy::bundled();
        let rs = Arc::new(Ruleset::tl_s2());
        let store = std::fs::create_dir_all(data_dir())
            .ok()
            .and_then(|_| Store::open(&data_dir().join("stacklab.db")).ok());
        let load = |k: &str| store.as_ref().and_then(|s| s.get_state(k).ok().flatten());
        let profile = load("profile")
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_else(|| SkillProfile::new(&policy));
        let saved = load("cycle")
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or(SavedCycle {
                cycle: Cycle::new(diagnosis_plan(1)),
                diagnosis: true,
                number: 0,
                set: SetState::new_first(),
            });
        let matchp = load("match")
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_else(|| MatchProfile::new(&policy));
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let autoshot = std::env::var("STACKLAB_AUTOSHOT").ok().map(|p| (p, 0));
        let mut profile = profile;
        if let Some(lo) = store.as_ref().and_then(|s| s.load_loadout().ok().flatten()) {
            profile.loadout = Some(lo);
        }
        let pick = profile
            .loadout
            .as_ref()
            .map_or(("tki".to_string(), "free-transition".to_string()), |l| {
                (l.primary.clone(), l.fallback.clone())
            });
        StackApp {
            s: Strings::korean(),
            worker: spawn_worker(rs.clone(), policy.clone(), cc.egui_ctx.clone()),
            rs,
            matchp,
            bots: stacklab_bot::calibration(),
            tl_input: (1500.0, 150.0),
            tl_msg: None,
            eval_report: None,
            rec_msg: None,
            policy,
            store,
            profile,
            saved,
            rng: Rng::new(seed),
            screen: Screen::Dashboard,
            handling: Handling::default(),
            last_report: None,
            autoshot,
            library: stacklab_opener::library::Library::bundled(),
            coverage: stacklab_opener::library::coverage_table(),
            pick,
            opener_msg: None,
        }
    }

    fn persist(&self) {
        if let Some(st) = &self.store {
            let _ = st.set_state(
                "profile",
                &serde_json::to_string(&self.profile).unwrap_or_default(),
            );
            let _ = st.set_state(
                "cycle",
                &serde_json::to_string(&self.saved).unwrap_or_default(),
            );
            let _ = st.set_state(
                "match",
                &serde_json::to_string(&self.matchp).unwrap_or_default(),
            );
        }
    }

    fn request_next(&mut self) {
        let Some(m) = self.saved.cycle.next_module() else {
            return;
        };
        let req = if self.saved.diagnosis {
            Request::Anchor(m, self.rng.below(10))
        } else {
            Request::Slot(
                m,
                Box::new(self.profile.clone()),
                self.rng.below(u64::MAX >> 1),
            )
        };
        if self.worker.0.send(req).is_ok() {
            self.screen = Screen::Loading;
        }
    }

    fn module_name(&self, m: Module) -> String {
        self.s.get(&format!("module_{}", m.letter()))
    }

    fn goal_text(&self, p: &Problem) -> String {
        match &p.goal {
            Goal::Finesse { optimal_inputs, .. } => self
                .s
                .fmt("goal_finesse", &[("n", optimal_inputs.to_string())]),
            Goal::Stacking {
                pieces,
                max_holes,
                max_height,
                well,
            } => {
                let mut t = self.s.fmt(
                    "goal_stacking",
                    &[
                        ("n", pieces.to_string()),
                        ("h", max_holes.to_string()),
                        ("t", max_height.to_string()),
                    ],
                );
                if let Some(w) = well {
                    t += &self
                        .s
                        .fmt("goal_stacking_well", &[("w", (w + 1).to_string())]);
                }
                t
            }
            Goal::Spin {
                piece,
                spin,
                lines,
                within_pieces,
            } => {
                let pname = piece.map_or("All-".to_string(), |p| format!("{}-", p.to_char()));
                let sname = match spin {
                    Spin::Full => "Spin ",
                    Spin::Mini => "Spin Mini ",
                    Spin::None => "",
                };
                let lname = ["", "Single", "Double", "Triple", "Quad"][(*lines as usize).min(4)];
                self.s.fmt(
                    "goal_spin",
                    &[
                        ("p", pname),
                        ("s", sname.into()),
                        ("l", lname.into()),
                        ("n", within_pieces.to_string()),
                    ],
                )
            }
            Goal::PerfectClear { within_pieces } => {
                self.s.fmt("goal_pc", &[("n", within_pieces.to_string())])
            }
            Goal::OpenerBuild {
                opener,
                pieces,
                finisher,
                ..
            } => {
                let name = self
                    .library
                    .get(opener)
                    .map_or(opener.clone(), |d| d.name.clone());
                let f = finisher.map_or(String::new(), |(p, _, l)| {
                    let lname = ["", "Single", "Double", "Triple"][(l as usize).min(3)];
                    self.s.fmt(
                        "goal_opener_finisher",
                        &[("p", p.to_char().to_string()), ("l", lname.into())],
                    )
                });
                self.s.fmt(
                    "goal_opener",
                    &[("o", name), ("n", pieces.to_string()), ("f", f)],
                )
            }
            Goal::Attack {
                min_attack,
                within_pieces,
            } => self.s.fmt(
                "goal_attack",
                &[
                    ("n", within_pieces.to_string()),
                    ("a", min_attack.to_string()),
                ],
            ),
            Goal::Downstack { pieces, min_lines } => self.s.fmt(
                "goal_downstack",
                &[("n", pieces.to_string()), ("l", min_lines.to_string())],
            ),
            Goal::Placement { rated, .. } => {
                let mut t = self.s.get("goal_placement");
                if !rated {
                    t += &self.s.get("goal_unrated");
                }
                t
            }
        }
    }

    fn term_tip(&self, p: &Problem) -> Option<String> {
        Some(match &p.goal {
            Goal::Finesse { .. } => self.s.get("term_finesse"),
            Goal::Spin {
                spin: Spin::Mini, ..
            } => self.s.get("term_mini"),
            Goal::Spin { .. } => self.s.get("term_tspin"),
            Goal::PerfectClear { .. } => self.s.get("term_pc"),
            Goal::Attack { .. } => self.s.get("term_b2b"),
            Goal::OpenerBuild { .. } => self.s.get("term_opener"),
            _ => return None,
        })
    }

    fn reason_text(&self, r: &FailReason) -> String {
        match r {
            FailReason::WrongPlacement => self.s.get("reason_wrong_placement"),
            FailReason::TopOut => self.s.get("reason_top_out"),
            FailReason::NotAchieved => self.s.get("reason_not_achieved"),
            FailReason::TimeUp => self.s.get("reason_time_up"),
            FailReason::GoalNotMet {
                holes,
                height,
                well_blocked,
            } => {
                let w = if *well_blocked {
                    self.s.get("reason_well_blocked")
                } else {
                    String::new()
                };
                self.s.fmt(
                    "reason_goal_not_met",
                    &[
                        ("h", holes.to_string()),
                        ("t", height.to_string()),
                        ("w", w),
                    ],
                )
            }
        }
    }

    /// Records the slot with the first decided result and moves on (retries never add problems).
    fn finish_slot(&mut self, play: &Playing) {
        let p = &play.session.problem;
        let first = play.first.clone().unwrap_or_else(|| play.session.result());
        let share =
            (first.elapsed_frames as f64 / (p.time_limit_secs.max(1) as f64 * 60.0)).min(1.0);
        let bucket = if self.saved.diagnosis {
            anchor_bucket(p.module)
        } else {
            p.param_bucket.clone()
        };
        self.saved.cycle.record(SlotResult {
            module: p.module,
            bucket,
            estimate: p.difficulty,
            tags: p.subtype_tags.clone(),
            attempt: first,
            time_share: share,
        });
        if self.saved.cycle.is_complete() {
            let report = self.saved.cycle.close(&mut self.profile, &self.policy);
            self.last_report = Some(self.s.fmt(
                "cycle_done",
                &[
                    ("v", report.valid_attempts.to_string()),
                    ("s", report.successes.to_string()),
                ],
            ));
            let mut set = self.saved.set.clone();
            if self.saved.diagnosis {
                let _ = set.diagnosis_done();
            } else {
                let _ = set.cycle_done();
            }
            self.saved = SavedCycle {
                cycle: Cycle::new(plan_cycle(&self.profile, &self.policy, &mut self.rng)),
                diagnosis: false,
                number: self.saved.number + 1,
                set,
            };
        }
        self.persist();
    }

    fn progress_text(&self) -> String {
        let n = self.saved.cycle.results.len() + 1;
        if self.saved.diagnosis {
            self.s.fmt(
                "diagnosis_progress",
                &[
                    ("n", n.to_string()),
                    ("m", self.saved.cycle.plan.len().to_string()),
                ],
            )
        } else {
            let c = match self.saved.set.phase {
                Phase::Training(k) => k as u32,
                _ => 3,
            };
            self.s.fmt(
                "cycle_progress",
                &[("c", c.to_string()), ("n", n.to_string())],
            )
        }
    }

    fn dashboard(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.s.get("app_title"));
        ui.add_space(8.0);
        let label = if self.saved.cycle.results.is_empty() && self.saved.number == 0 {
            self.s.get("start_training")
        } else {
            self.s.get("continue_training")
        };
        let phase = self.saved.set.phase;
        ui.horizontal(|ui| {
            if matches!(phase, Phase::Diagnosis | Phase::Training(_))
                && ui
                    .add(egui::Button::new(RichText::new(label).size(20.0)))
                    .clicked()
            {
                self.request_next();
            }
            if matches!(phase, Phase::Diagnosis | Phase::Training(_)) {
                ui.label(self.progress_text());
            }
            if ui.button(self.s.get("openers")).clicked() {
                self.screen = Screen::Openers;
            }
        });
        self.match_actions(ui);
        if let Some(r) = &self.last_report {
            ui.label(r);
        }
        ui.separator();
        self.match_rating_panel(ui);
        ui.separator();
        ui.label(RichText::new(self.s.get("skill_ratings")).strong());
        egui::Grid::new("skills").striped(true).show(ui, |ui| {
            for (m, r) in &self.profile.skills {
                ui.label(format!("{} {}", m.letter(), self.module_name(*m)));
                let prov = if r.is_provisional(self.policy.rating.provisional_rd) {
                    format!(" ({})", self.s.get("provisional"))
                } else {
                    String::new()
                };
                ui.label(format!("{:.0} ± {:.0}{prov}", r.rating, r.rd));
                ui.end_row();
            }
        });
    }

    /// UI-07: opener management (primary / fallback, rules, coverage with scope, proficiency).
    fn openers_screen(&mut self, ui: &mut egui::Ui) -> bool {
        let mut back = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(self.s.get("back_to_dashboard")).clicked() {
                    back = true;
                }
                ui.heading(self.s.get("openers"));
            });
            ui.label(self.s.get("opener_rule"));
            if self.profile.loadout.is_none() {
                ui.label(RichText::new(self.s.get("opener_none")).weak());
            }
            let names: Vec<(String, String, bool)> = self
                .library
                .openers
                .iter()
                .map(|o| {
                    (
                        o.id.clone(),
                        o.name.clone(),
                        o.kind == stacklab_opener::library::Kind::Memorized,
                    )
                })
                .collect();
            ui.horizontal(|ui| {
                ui.label(self.s.get("opener_primary"));
                egui::ComboBox::from_id_salt("primary")
                    .selected_text(
                        names
                            .iter()
                            .find(|n| n.0 == self.pick.0)
                            .map_or(self.pick.0.clone(), |n| n.1.clone()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name, memorized) in &names {
                            if *memorized {
                                ui.selectable_value(&mut self.pick.0, id.clone(), name);
                            }
                        }
                    });
                ui.label(self.s.get("opener_fallback"));
                egui::ComboBox::from_id_salt("fallback")
                    .selected_text(
                        names
                            .iter()
                            .find(|n| n.0 == self.pick.1)
                            .map_or(self.pick.1.clone(), |n| n.1.clone()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name, _) in &names {
                            ui.selectable_value(&mut self.pick.1, id.clone(), name);
                        }
                    });
                if ui.button(self.s.get("opener_apply")).clicked() {
                    self.apply_loadout();
                }
            });
            if let Some(m) = &self.opener_msg {
                ui.label(m);
            }
            ui.horizontal(|ui| {
                if self.profile.loadout.is_some()
                    && self.store.is_some()
                    && ui.button(self.s.get("rec_reanalyze")).clicked()
                {
                    if let Some(id) = self.create_recommendation(false) {
                        self.screen = Screen::Recommend(id, false);
                    }
                }
                if let Some(id) = self.open_recommendation_id() {
                    if ui.button(self.s.get("rec_deferred_list")).clicked() {
                        self.screen = Screen::Recommend(id, false);
                    }
                }
            });
            ui.separator();
            let openers = self.library.openers.clone();
            for def in &openers {
                let role = match &self.profile.loadout {
                    Some(l) if l.primary == def.id => {
                        format!(" — {}", self.s.get("opener_primary"))
                    }
                    Some(l) if l.fallback == def.id => {
                        format!(" — {}", self.s.get("opener_fallback"))
                    }
                    _ => String::new(),
                };
                let kind = if def.kind == stacklab_opener::library::Kind::Memorized {
                    self.s.get("opener_kind_memorized")
                } else {
                    self.s.get("opener_kind_template")
                };
                egui::CollapsingHeader::new(
                    RichText::new(format!("{} ({kind}){role}", def.name)).strong(),
                )
                .default_open(true)
                .id_salt(&def.id)
                .show(ui, |ui| {
                    if let Some(c) = self.coverage.get(&def.id) {
                        let txt = if c.is_guaranteed() {
                            self.s.get("opener_guaranteed")
                        } else {
                            self.s.fmt(
                                "opener_coverage",
                                &[
                                    ("r", format!("{:.1}", 100.0 * c.rate())),
                                    ("lo", format!("{:.1}", 100.0 * c.ci_low)),
                                    ("hi", format!("{:.1}", 100.0 * c.ci_high)),
                                ],
                            )
                        };
                        ui.label(txt);
                        ui.label(
                            RichText::new(self.s.fmt("opener_scope", &[("s", c.scope.clone())]))
                                .weak(),
                        );
                    }
                    match self.profile.openers.get(&def.id) {
                        Some(r) => ui.label(self.s.fmt(
                            "opener_proficiency",
                            &[
                                ("r", format!("{:.0}", r.rating)),
                                ("d", format!("{:.0}", r.rd)),
                            ],
                        )),
                        None => ui.label(RichText::new(self.s.get("opener_unplayed")).weak()),
                    };
                    ui.label(
                        RichText::new(self.s.fmt("opener_source", &[("s", def.source.clone())]))
                            .weak(),
                    );
                    if !def.branches.is_empty() {
                        ui.label(self.s.get("opener_branches"));
                        ui.horizontal_wrapped(|ui| {
                            for b in def.branches.iter().filter(|b| !b.mirrored) {
                                ui.vertical(|ui| {
                                    let (resp, painter) = ui.allocate_painter(
                                        Vec2::new(110.0, 120.0),
                                        egui::Sense::hover(),
                                    );
                                    let rect =
                                        Rect::from_min_size(resp.rect.min, Vec2::new(110.0, 240.0));
                                    let clip = painter.with_clip_rect(resp.rect);
                                    // draw only the bottom 6 rows of the field, scaled into the box
                                    let shifted = rect.translate(Vec2::new(0.0, -120.0));
                                    field(
                                        &clip,
                                        shifted,
                                        &FieldView {
                                            board: &b.target,
                                            active: None,
                                            ghost: None,
                                            targets: vec![],
                                            colored: b.pieces.clone(),
                                        },
                                    );
                                    let (n, s) = self
                                        .profile
                                        .branch_stats
                                        .get(&format!("{}/{}", def.id, b.id))
                                        .copied()
                                        .unwrap_or((0, 0));
                                    ui.label(self.s.fmt(
                                        "opener_branch_stat",
                                        &[
                                            ("b", b.id.clone()),
                                            ("s", s.to_string()),
                                            ("n", n.to_string()),
                                        ],
                                    ));
                                });
                            }
                        });
                    }
                });
            }
        });
        back
    }

    fn apply_loadout(&mut self) {
        let (p, f) = self.pick.clone();
        let expected = self.profile.loadout.as_ref().map(|l| l.version);
        if let Err(e) = stacklab_opener::loadout::validate(&self.library, "TL-S2", &p, &f) {
            self.opener_msg = Some(e.to_string());
            return;
        }
        let saved = match self.store.as_mut() {
            Some(st) => st
                .set_loadout(&p, &f, expected, "manual", None)
                .map_err(|e| e.to_string()),
            None => Ok(stacklab_opener::loadout::OpenerLoadout {
                primary: p.clone(),
                fallback: f.clone(),
                version: expected.unwrap_or(0) + 1,
            }),
        };
        match saved {
            Ok(lo) => {
                self.opener_msg =
                    Some(self.s.fmt("opener_saved", &[("v", lo.version.to_string())]));
                self.profile.loadout = Some(lo);
                self.persist();
            }
            Err(e) => self.opener_msg = Some(e),
        }
    }

    /// Demo data for the headless UI-08 screenshot (STACKLAB_AUTOSCREEN=recommend-demo).
    fn seed_recommendation_demo(&mut self) {
        use stacklab_training::glicko::Rating;
        if self.profile.loadout.is_none() {
            if let Some(st) = self.store.as_mut() {
                if let Ok(lo) = st.set_loadout("tki", "free-transition", None, "demo", None) {
                    self.profile.loadout = Some(lo);
                }
            }
        }
        for (m, r) in [
            (Module::A, 1700.0),
            (Module::B, 1650.0),
            (Module::C, 1050.0),
            (Module::D, 1950.0),
            (Module::E, 1700.0),
            (Module::F, 1500.0),
            (Module::G, 1500.0),
            (Module::H, 1800.0),
        ] {
            self.profile.skills.insert(m, Rating::new(r, 60.0, 0.06));
        }
        for (o, r) in [
            ("tki", 1050.0),
            ("pco", 1900.0),
            ("free-transition", 1300.0),
            ("pc-first", 1850.0),
        ] {
            self.profile
                .openers
                .insert(o.into(), Rating::new(r, 60.0, 0.06));
        }
        self.profile
            .branch_stats
            .insert("pco/with-i".into(), (30, 27));
        self.profile.opener_valid_attempts = 30;
        self.profile.opener_observed_rounds = 6;
    }

    /// Spec 9.1: one automatic event after the first qualifying normal evaluation.
    fn maybe_create_recommendation(&mut self) {
        let exists = self
            .store
            .as_ref()
            .and_then(|s| s.has_automatic_recommendation().ok())
            .unwrap_or(true);
        if stacklab_training::recommend::automatic_trigger(
            &self.profile,
            &self.saved.set,
            &self.policy,
            exists,
        ) {
            self.create_recommendation(true);
        }
    }

    /// Creates (and analyses) a recommendation event; returns its id.
    fn create_recommendation(&mut self, automatic: bool) -> Option<String> {
        let version = self.profile.loadout.as_ref()?.version;
        let id = format!(
            "rec-{}-{}",
            if automatic { "auto" } else { "manual" },
            stacklab_store::now_utc()
        );
        let trigger = automatic.then(|| format!("set-{}", self.saved.set.number));
        let mut e =
            stacklab_training::recommend::new_event(&id, trigger.as_deref(), automatic, version);
        e.run(
            &self.profile,
            &self.library,
            &self.coverage,
            &self.policy,
            "TL-S2",
        )
        .ok()?;
        self.store.as_ref()?.create_recommendation(&e).ok()?;
        Some(id)
    }

    /// A READY or DEFERRED event the user can still decide on.
    fn open_recommendation_id(&self) -> Option<String> {
        use stacklab_training::recommend::RecStatus;
        let list = self.store.as_ref()?.list_recommendations().ok()?;
        list.into_iter()
            .rev()
            .find(|e| matches!(e.status, RecStatus::Ready | RecStatus::Deferred))
            .map(|e| e.event_id)
    }

    fn opener_name(&self, id: &str) -> String {
        self.library
            .get(id)
            .map_or(id.to_string(), |d| d.name.clone())
    }

    fn pair_column(
        &self,
        ui: &mut egui::Ui,
        title: &str,
        p: &stacklab_training::recommend::PairScore,
    ) {
        use stacklab_training::recommend::Reason;
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong());
            ui.label(
                RichText::new(self.s.fmt(
                    "rec_pair",
                    &[
                        ("p", self.opener_name(&p.primary)),
                        ("f", self.opener_name(&p.fallback)),
                    ],
                ))
                .size(18.0),
            );
            ui.label(self.s.fmt(
                "rec_score",
                &[
                    ("s", format!("{:.1}", p.total)),
                    ("lo", format!("{:.1}", p.low80)),
                    ("hi", format!("{:.1}", p.high80)),
                ],
            ));
            for c in &p.components {
                let k = self.s.get(&format!("rec_component_{}", c.key));
                let e = if c.estimated {
                    self.s.get("rec_estimated")
                } else {
                    String::new()
                };
                ui.label(self.s.fmt(
                    "rec_component_row",
                    &[
                        ("k", k),
                        ("v", format!("{:.1}", c.value)),
                        ("w", format!("{:.0}", c.weight)),
                        ("e", e),
                    ],
                ));
            }
            for r in &p.reasons {
                let t = match r {
                    Reason::Coverage { primary, union } => self.s.fmt(
                        "rec_reason_coverage",
                        &[
                            ("p", format!("{:.0}", 100.0 * primary)),
                            ("u", format!("{:.0}", 100.0 * union)),
                        ],
                    ),
                    Reason::Shortfall {
                        opener,
                        dimension,
                        required,
                        ability,
                    } => self.s.fmt(
                        "rec_reason_shortfall",
                        &[
                            ("o", self.opener_name(opener)),
                            ("d", self.s.get(&format!("dim_{dimension}"))),
                            ("r", required.to_string()),
                            ("a", ability.to_string()),
                        ],
                    ),
                    Reason::Observed { opener, rating, rd } => self.s.fmt(
                        "rec_reason_observed",
                        &[
                            ("o", self.opener_name(opener)),
                            ("r", format!("{rating:.0}")),
                            ("d", format!("{rd:.0}")),
                        ],
                    ),
                    Reason::Estimated { opener } => self
                        .s
                        .fmt("rec_reason_estimated", &[("o", self.opener_name(opener))]),
                    Reason::Learning { opener, cost } => self.s.fmt(
                        "rec_reason_learning",
                        &[("o", self.opener_name(opener)), ("c", cost.to_string())],
                    ),
                };
                ui.label(RichText::new(format!("• {t}")).weak());
            }
        });
    }

    /// UI-08 (spec 9.4): side-by-side comparison, then a confirmation step before applying.
    fn recommend_screen(&mut self, ui: &mut egui::Ui, id: &str, confirm: bool) -> Option<Screen> {
        use stacklab_training::recommend::{Analysis, RecStatus};
        let ev = self.store.as_ref()?.load_recommendation(id).ok()??;
        let mut next = None;
        ui.horizontal(|ui| {
            if ui.button(self.s.get("back_to_dashboard")).clicked() {
                next = Some(Screen::Dashboard);
            }
            ui.heading(self.s.get("rec_title"));
            ui.label(
                RichText::new(
                    self.s
                        .fmt("rec_status", &[("s", ev.status.as_str().to_string())]),
                )
                .weak(),
            );
        });
        if let Some(m) = &self.rec_msg {
            ui.label(m);
        }
        match &ev.analysis {
            Some(Analysis::Change {
                current,
                recommended,
            }) => {
                if !confirm {
                    ui.horizontal(|ui| {
                        self.pair_column(ui, &self.s.get("rec_current"), current);
                        ui.separator();
                        self.pair_column(ui, &self.s.get("rec_recommended"), recommended);
                    });
                    if matches!(ev.status, RecStatus::Ready | RecStatus::Deferred) {
                        ui.horizontal(|ui| {
                            if ui.button(self.s.get("rec_apply")).clicked() {
                                next = Some(Screen::Recommend(id.to_string(), true));
                            }
                            if ui.button(self.s.get("rec_keep")).clicked() {
                                self.set_rec_status(id, RecStatus::Declined);
                                next = Some(Screen::Dashboard);
                            }
                            if ev.status == RecStatus::Ready
                                && ui.button(self.s.get("rec_later")).clicked()
                            {
                                self.set_rec_status(id, RecStatus::Deferred);
                                next = Some(Screen::Dashboard);
                            }
                        });
                    }
                } else {
                    ui.label(RichText::new(self.s.get("rec_confirm_title")).strong());
                    let a = self.s.fmt(
                        "rec_pair",
                        &[
                            ("p", self.opener_name(&current.primary)),
                            ("f", self.opener_name(&current.fallback)),
                        ],
                    );
                    let b = self.s.fmt(
                        "rec_pair",
                        &[
                            ("p", self.opener_name(&recommended.primary)),
                            ("f", self.opener_name(&recommended.fallback)),
                        ],
                    );
                    ui.label(self.s.fmt("rec_before_after", &[("a", a), ("b", b)]));
                    ui.label(self.s.get("rec_keep_note"));
                    ui.horizontal(|ui| {
                        if ui.button(self.s.get("rec_confirm")).clicked() {
                            next = Some(self.confirm_recommendation(id));
                        }
                        if ui.button(self.s.get("rec_cancel")).clicked() {
                            next = Some(Screen::Recommend(id.to_string(), false));
                        }
                    });
                }
            }
            Some(Analysis::NoChange { current, .. }) => {
                ui.label(self.s.get("rec_nochange"));
                self.pair_column(ui, &self.s.get("rec_current"), current);
            }
            _ => {
                ui.label(self.s.get("rec_unavailable"));
            }
        }
        next
    }

    fn set_rec_status(&mut self, id: &str, to: stacklab_training::recommend::RecStatus) {
        let Some(st) = self.store.as_ref() else {
            return;
        };
        if let Ok(Some(mut e)) = st.load_recommendation(id) {
            if e.transition(to).is_ok() {
                let _ = st.save_recommendation(&e);
            }
        }
    }

    /// Step 2 "변경 확정": one DB transaction; on any failure the old loadout stays.
    fn confirm_recommendation(&mut self, id: &str) -> Screen {
        let lib = self.library.clone();
        let Some(st) = self.store.as_mut() else {
            return Screen::Dashboard;
        };
        match st.apply_recommendation(id, &lib, "TL-S2") {
            Ok(stacklab_store::ApplyOutcome::Applied(lo)) => {
                self.rec_msg = Some(self.s.fmt("rec_applied", &[("v", lo.version.to_string())]));
                self.opener_msg = self.rec_msg.clone();
                self.pick = (lo.primary.clone(), lo.fallback.clone());
                self.profile.loadout = Some(lo);
                self.persist();
                Screen::Openers
            }
            Ok(stacklab_store::ApplyOutcome::AlreadyApplied) => {
                self.rec_msg = Some(self.s.get("rec_already"));
                Screen::Openers
            }
            Ok(stacklab_store::ApplyOutcome::Conflict { .. }) => {
                self.rec_msg = Some(self.s.get("rec_conflict"));
                Screen::Recommend(id.to_string(), false)
            }
            Err(e) => {
                self.rec_msg = Some(self.s.fmt("rec_failed", &[("e", e.to_string())]));
                Screen::Recommend(id.to_string(), false)
            }
        }
    }

    fn match_rating_panel(&mut self, ui: &mut egui::Ui) {
        let r = self.matchp.rating;
        let key = if self.matchp.anchored() {
            "match_rating_tl"
        } else {
            "match_rating_app"
        };
        ui.label(RichText::new(self.s.get("match_rating")).strong());
        ui.label(self.s.fmt(
            key,
            &[
                ("r", format!("{:.0}", r.rating)),
                ("d", format!("{:.0}", r.rd)),
            ],
        ));
        ui.horizontal(|ui| {
            ui.label(self.s.get("tl_input"));
            ui.label(self.s.get("tl_glicko"));
            ui.add(
                egui::DragValue::new(&mut self.tl_input.0)
                    .range(0.0..=4000.0)
                    .speed(5.0),
            );
            ui.label(self.s.get("tl_rd"));
            ui.add(
                egui::DragValue::new(&mut self.tl_input.1)
                    .range(30.0..=350.0)
                    .speed(1.0),
            );
            if ui.button(self.s.get("tl_apply")).clicked() {
                let now = stacklab_store::now_utc();
                self.matchp.enter_anchor(
                    self.tl_input.0,
                    self.tl_input.1,
                    &now,
                    self.policy.tl_anchor.min_rd,
                );
                self.tl_msg = Some(
                    self.s
                        .fmt("tl_saved", &[("n", self.matchp.anchors.len().to_string())]),
                );
                self.persist();
            }
        });
        if let Some(m) = &self.tl_msg {
            ui.label(m);
        }
    }

    /// Phase-specific actions: mini matches, evaluation, result (UI-06).
    fn match_actions(&mut self, ui: &mut egui::Ui) {
        let set = self.saved.set.clone();
        match set.phase {
            Phase::Mini(k) => {
                ui.label(RichText::new(self.s.fmt("phase_mini", &[("k", k.to_string())])).strong());
                ui.horizontal(|ui| {
                    if ui.button(self.s.get("mini_start")).clicked() {
                        self.start_match(MatchKind::Mini(k));
                    }
                    if ui.button(self.s.get("mini_skip")).clicked() {
                        let _ = self.saved.set.skip_mini();
                        self.persist();
                    }
                    if ui.button(self.s.get("mini_defer")).clicked() {
                        let _ = self.saved.set.defer_mini();
                        self.persist();
                    }
                });
            }
            Phase::Evaluation => {
                ui.label(RichText::new(self.s.get("phase_eval")).strong());
                let (w, l) = set.score();
                ui.horizontal(|ui| {
                    let label = if set.eval_rounds.is_empty() {
                        self.s.get("eval_start")
                    } else {
                        self.s
                            .fmt("eval_resume", &[("w", w.to_string()), ("l", l.to_string())])
                    };
                    if ui.button(label).clicked() {
                        self.start_match(MatchKind::Evaluation);
                    }
                    if ui.button(self.s.get("eval_defer")).clicked() {
                        let _ = self.saved.set.defer_evaluation();
                        self.persist();
                    }
                    if ui.button(self.s.get("eval_close")).clicked() {
                        let _ = self.saved.set.close_without_evaluation();
                        self.maybe_create_recommendation();
                        self.eval_report = Some(EvalReport {
                            wins: w,
                            losses: l,
                            before: None,
                            after: None,
                            weaknesses: vec![],
                        });
                        self.persist();
                    }
                });
            }
            Phase::Result => {
                ui.label(
                    RichText::new(self.s.fmt("phase_result", &[("n", set.number.to_string())]))
                        .strong(),
                );
                if let Some(rep) = self.eval_report.clone() {
                    if set.closed_unrated {
                        ui.label(self.s.get("eval_unrated"));
                    } else {
                        let res = if rep.wins > rep.losses {
                            self.s.get("eval_won")
                        } else {
                            self.s.get("eval_lost")
                        };
                        ui.label(
                            RichText::new(self.s.fmt(
                                "eval_result",
                                &[
                                    ("w", rep.wins.to_string()),
                                    ("l", rep.losses.to_string()),
                                    ("res", res),
                                ],
                            ))
                            .size(20.0),
                        );
                        if let (Some(a), Some(b)) = (rep.before, rep.after) {
                            ui.label(self.s.fmt(
                                "match_delta",
                                &[
                                    ("a", format!("{:.0} ± {:.0}", a.0, a.1)),
                                    ("b", format!("{:.0} ± {:.0}", b.0, b.1)),
                                ],
                            ));
                        }
                        ui.label(RichText::new(self.s.get("weaknesses")).strong());
                        if rep.weaknesses.is_empty() {
                            ui.label(self.s.get("weakness_none"));
                        }
                        for w in &rep.weaknesses {
                            ui.label(format!(
                                "• {}",
                                self.s.get(&format!("weak_{}", w.replace('-', "_")))
                            ));
                        }
                    }
                }
                if let Some(id) = self.open_recommendation_id() {
                    if ui.button(self.s.get("rec_open")).clicked() {
                        self.screen = Screen::Recommend(id, false);
                    }
                }
                if ui.button(self.s.get("next_set")).clicked() {
                    if let Ok(n) = self.saved.set.next_set() {
                        self.saved.set = n;
                        self.eval_report = None;
                        self.persist();
                    }
                }
            }
            _ => {}
        }
        // deferred minis can still be played before the result
        if set.phase != Phase::Result {
            for (i, m) in set.minis.iter().enumerate() {
                if *m == MiniStatus::Deferred {
                    ui.horizontal(|ui| {
                        ui.label(self.s.get("deferred_minis"));
                        if ui.button(self.s.get("play_deferred")).clicked() {
                            self.start_match(MatchKind::DeferredMini(i));
                        }
                    });
                }
            }
        }
    }

    fn start_match(&mut self, kind: MatchKind) {
        let bots: Vec<(String, f64, f64)> = self
            .bots
            .bot
            .iter()
            .map(|b| {
                let (r, rd) = self.matchp.bot_rating(&b.id, b.rating, b.rd);
                (b.id.clone(), r, rd)
            })
            .collect();
        let band = (
            self.policy.matches.win_prob_band[0],
            self.policy.matches.win_prob_band[1],
        );
        let Some((id, p, in_band)) = select_opponent(&self.matchp, &bots, band) else {
            return;
        };
        let cal = self
            .bots
            .bot
            .iter()
            .find(|b| b.id == id)
            .cloned()
            .expect("selected bot");
        let (cal_rating, cal_rd) = self.matchp.bot_rating(&cal.id, cal.rating, cal.rd);
        let seed = self.rng.below(u64::MAX >> 1);
        let m = MatchPlay::new(
            kind,
            self.rs.clone(),
            self.handling,
            stacklab_bot::BotConfig::preset(&id),
            cal_rating,
            cal_rd,
            p,
            in_band,
            seed,
        );
        self.screen = Screen::Match(Box::new(m));
    }

    fn opener_shapes(&self) -> Vec<stacklab_engine::Board> {
        let Some(lo) = &self.profile.loadout else {
            return vec![];
        };
        [&lo.primary, &lo.fallback]
            .iter()
            .filter_map(|id| self.library.get(id))
            .flat_map(|d| d.branches.iter().map(|b| b.target.clone()))
            .collect()
    }

    /// Round finished: update the set, ratings and observed weaknesses.
    fn finish_round(&mut self, m: &MatchPlay) {
        let won = m.finished.unwrap_or(false);
        let mut obs = Observations::from_events(&m.user_events);
        let shapes = self.opener_shapes();
        if !shapes.is_empty() {
            obs.opener_built = Some(opener_built(&m.user_boards, &shapes));
        }
        self.profile
            .apply_match_observations(std::slice::from_ref(&obs));
        match m.kind {
            MatchKind::Mini(_) => {
                let _ = self.saved.set.mini_result(won);
            }
            MatchKind::DeferredMini(i) => {
                self.saved.set.deferred_mini_result(i, won);
            }
            MatchKind::Evaluation => {
                let rec = RoundRecord {
                    won,
                    void: false,
                    bot: m.bot_id.clone(),
                    bot_rating: m.bot_rating,
                    bot_rd: m.bot_rd,
                    calibration_version: self
                        .matchp
                        .calibration_version(self.bots.calibration_version),
                    observations: obs,
                };
                if let Ok(true) = self.saved.set.eval_round(rec) {
                    let (w, l) = self.saved.set.score();
                    let (before, after) = self
                        .matchp
                        .rate_evaluation(&self.saved.set.eval_rounds, &self.policy);
                    let weaknesses = self
                        .saved
                        .set
                        .eval_rounds
                        .iter()
                        .flat_map(|r| r.observations.weaknesses().into_iter().map(|x| x.1))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    self.eval_report = Some(EvalReport {
                        wins: w,
                        losses: l,
                        before: Some((before.rating, before.rd)),
                        after: Some((after.rating, after.rd)),
                        weaknesses,
                    });
                }
            }
        }
        self.persist();
    }

    fn match_screen(&mut self, ui: &mut egui::Ui, mut m: Box<MatchPlay>) -> Screen {
        let base = m.versus.players[0].now_sub();
        let map = keymap();
        ui.input(|i| {
            for e in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    ..
                } = e
                {
                    if *repeat {
                        continue;
                    }
                    if let Some((_, k)) = map.iter().find(|(ek, _)| ek == key) {
                        let t = base + (m.pending.len() as u64).min(9);
                        m.pending.push(InputEvent {
                            time: t,
                            key: *k,
                            down: *pressed,
                        });
                    }
                }
            }
        });
        let now = ui.input(|i| i.time);
        m.advance(now);
        ui.ctx().request_repaint();
        let (w, l) = self.saved.set.score();
        let header = match m.kind {
            MatchKind::Evaluation => self.s.fmt(
                "round_label",
                &[
                    ("n", (w + l + 1).to_string()),
                    ("w", w.to_string()),
                    ("l", l.to_string()),
                ],
            ),
            MatchKind::Mini(k) => self.s.fmt("mini_label", &[("k", k.to_string())]),
            MatchKind::DeferredMini(i) => self.s.fmt("mini_label", &[("k", (i + 1).to_string())]),
        };
        let opp = self.s.fmt(
            "opponent",
            &[
                ("b", m.bot_id.clone()),
                ("p", format!("{:.0}", 100.0 * m.expected)),
                (
                    "o",
                    if m.in_band {
                        String::new()
                    } else {
                        self.s.get("out_of_band")
                    },
                ),
            ],
        );
        let mut done = false;
        if let Some(won) = m.finished {
            egui::Panel::bottom("round_result").show(ui, |ui| {
                ui.label(
                    RichText::new(if won {
                        self.s.get("round_won")
                    } else {
                        self.s.get("round_lost")
                    })
                    .size(22.0),
                );
                if ui.button(self.s.get("continue_btn")).clicked() {
                    done = true;
                }
            });
        }
        let pending_tpl = self.s.get("pending_garbage");
        let you = self.s.get("you");
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(&opp);
            m.draw(ui, &header, &you, &|n| {
                pending_tpl.replace("{n}", &n.to_string())
            });
        });
        if done {
            self.finish_round(&m);
            return Screen::Dashboard;
        }
        Screen::Match(m)
    }

    fn collect_inputs(&self, ui: &egui::Ui, play: &mut Playing) -> Vec<&'static str> {
        let map = keymap();
        let mut commands = Vec::new();
        let base = play.session.game().now_sub();
        ui.input(|i| {
            for e in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    ..
                } = e
                {
                    if *repeat {
                        continue;
                    }
                    if let Some((_, k)) = map.iter().find(|(ek, _)| ek == key) {
                        let t = base + (play.pending.len() as u64).min(9);
                        play.pending.push(InputEvent {
                            time: t,
                            key: *k,
                            down: *pressed,
                        });
                    } else if *pressed && *key == egui::Key::R {
                        commands.push("restart");
                    } else if *pressed && *key == egui::Key::H {
                        commands.push("hint");
                    }
                }
            }
        });
        commands
    }

    fn play_screen(&mut self, ui: &mut egui::Ui, mut play: Box<Playing>) -> Screen {
        // ---- simulation at a fixed 60 Hz tick, decoupled from the display rate ----
        if !play.finished {
            for c in self.collect_inputs(ui, &mut play) {
                match c {
                    "restart" => play.session.restart(),
                    "hint" => play.hint = play.session.hint(),
                    _ => {}
                }
            }
            let now = ui.input(|i| i.time);
            if play.sim_time == 0.0 {
                play.sim_time = now;
            }
            let mut steps = 0;
            while play.sim_time + 1.0 / 60.0 <= now && steps < 8 {
                let inputs = std::mem::take(&mut play.pending);
                play.session.step(&inputs);
                play.sim_time += 1.0 / 60.0;
                steps += 1;
                if *play.session.status() != Status::Running {
                    play.finished = true;
                    if play.first.is_none() {
                        play.first = Some(play.session.result());
                    }
                    break;
                }
            }
            if steps == 8 {
                play.sim_time = now; // don't spiral after a stall
            }
            ui.ctx().request_repaint();
        }

        let p = play.session.problem.clone();
        let hold_piece = play.session.game().hold_piece();
        let next_pieces = play.session.game().next_pieces();
        let remaining = play.session.remaining_frames();
        let mut back = false;
        let mut next = false;
        egui::Panel::left("info").exact_size(260.0).show(ui, |ui| {
            ui.label(RichText::new(self.progress_text()).strong());
            ui.label(format!(
                "{} · D {:.0}",
                self.module_name(p.module),
                p.difficulty
            ));
            ui.add_space(6.0);
            ui.label(RichText::new(self.s.get("goal")).strong());
            let g = ui.label(self.goal_text(&p));
            if let Some(tip) = self.term_tip(&p) {
                g.on_hover_text(tip);
            }
            ui.add_space(6.0);
            ui.label(
                self.s
                    .fmt("time_left", &[("s", (remaining / 60).to_string())]),
            );
            ui.add_space(12.0);
            ui.label(self.s.get("hold"));
            let (resp, painter) = ui.allocate_painter(Vec2::new(120.0, 70.0), egui::Sense::hover());
            if let Some(h) = hold_piece {
                mini_piece(&painter, resp.rect.center(), 16.0, h, false);
            }
            ui.add_space(12.0);
            if ui.button(self.s.get("hint")).clicked() {
                play.hint = play.session.hint();
            }
            if ui.button(self.s.get("restart")).clicked() {
                play.session.restart();
                play.finished = false;
            }
            if ui.button(self.s.get("back_to_dashboard")).clicked() {
                back = true;
            }
        });
        egui::Panel::right("next").exact_size(140.0).show(ui, |ui| {
            ui.label(self.s.get("next"));
            let n = p.visible_next as usize;
            for piece in next_pieces.into_iter().take(n) {
                let (resp, painter) =
                    ui.allocate_painter(Vec2::new(120.0, 60.0), egui::Sense::hover());
                mini_piece(&painter, resp.rect.center(), 14.0, piece, false);
            }
        });
        if play.finished {
            egui::Panel::bottom("result").show(ui, |ui| {
                let r = play.session.result();
                let first = play.first.clone().unwrap_or(r.clone());
                let (txt, color) = if r.success {
                    (self.s.get("success"), Color32::from_rgb(90, 200, 120))
                } else {
                    (self.s.get("fail"), Color32::from_rgb(230, 90, 90))
                };
                ui.label(RichText::new(txt).size(22.0).color(color));
                if let Some(reason) = &r.reason {
                    ui.label(self.reason_text(reason));
                }
                if let (Some(fs), Goal::Finesse { optimal_inputs, .. }) =
                    (r.finesse_success, &p.goal)
                {
                    let used = r.inputs_per_piece.first().copied().unwrap_or(0);
                    ui.label(if fs {
                        self.s.get("finesse_ok")
                    } else {
                        self.s.fmt(
                            "finesse_bad",
                            &[("o", optimal_inputs.to_string()), ("u", used.to_string())],
                        )
                    });
                }
                ui.horizontal(|ui| {
                    if play.session.pieces_locked() > 0 && ui.button(self.s.get("rewind")).clicked()
                    {
                        play.session.rewind();
                        play.finished = false;
                        play.sim_time = 0.0;
                    }
                    if ui.button(self.s.get("retry")).clicked() {
                        play.session.restart();
                        play.finished = false;
                        play.sim_time = 0.0;
                    }
                    if ui.button(self.s.get("show_answer")).clicked() {
                        play.answer = play.session.show_answer();
                    }
                    if ui.button(self.s.get("next_problem")).clicked() {
                        next = true;
                    }
                });
                egui::CollapsingHeader::new(self.s.get("detail")).show(ui, |ui| {
                    ui.label(self.s.get("retry_note"));
                    ui.label(self.s.get("assisted_note"));
                    ui.label(format!(
                        "{} · {:?} · first: {}",
                        p.id,
                        p.subtype_tags,
                        if first.success { "✓" } else { "✗" }
                    ));
                });
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            let painter = ui.painter();
            let mut targets = Vec::new();
            if let Goal::Finesse { target, .. } = &p.goal {
                targets.push((target.state(), Color32::from_rgb(250, 250, 250)));
            }
            if let Some(h) = play.hint {
                targets.push((h.target.state(), Color32::from_rgb(255, 210, 80)));
            }
            for a in &play.answer {
                targets.push((a.target.state(), Color32::from_rgb(120, 220, 255)));
            }
            let game = play.session.game();
            field(
                painter,
                rect.shrink(8.0),
                &FieldView {
                    board: game.board(),
                    active: game.active_piece(),
                    ghost: game.ghost(),
                    targets,
                    colored: vec![],
                },
            );
        });
        if back {
            return Screen::Dashboard;
        }
        if next {
            self.finish_slot(&play);
            self.request_next();
            return std::mem::replace(&mut self.screen, Screen::Loading);
        }
        Screen::Play(play)
    }

    /// Debug aid for headless checks: STACKLAB_AUTOSHOT=out.ppm saves a screenshot after a
    /// few frames and closes the window (used to verify the UI in CI-like environments).
    fn take_autoshot(&mut self, ui: &egui::Ui) {
        let Some((path, frames)) = self.autoshot.as_mut() else {
            return;
        };
        *frames += 1;
        if *frames == 150 {
            let path = path.clone();
            let ctx = ui.ctx().clone();
            ui.ctx().request_screenshot(move |image| {
                let mut out =
                    format!("P6\n{} {}\n255\n", image.size[0], image.size[1]).into_bytes();
                for px in &image.pixels {
                    out.extend_from_slice(&[px.r(), px.g(), px.b()]);
                }
                let _ = std::fs::write(&path, out);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
            });
        }
        ui.ctx().request_repaint();
    }
}

impl eframe::App for StackApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Ok(p) = self.worker.1.try_recv() {
            self.screen = match p.and_then(|p| AttemptSession::new(&p, self.handling).ok()) {
                Some(session) => Screen::Play(Box::new(Playing {
                    session,
                    first: None,
                    hint: None,
                    answer: Vec::new(),
                    sim_time: 0.0,
                    pending: Vec::new(),
                    finished: false,
                })),
                None => Screen::Dashboard,
            };
        }
        if std::env::var("STACKLAB_AUTOPLAY").is_ok()
            && matches!(self.screen, Screen::Dashboard)
            && self.autoshot.as_ref().is_some_and(|a| a.1 == 1)
        {
            self.request_next();
        }
        if self.autoshot.as_ref().is_some_and(|a| a.1 == 1) {
            match std::env::var("STACKLAB_AUTOSCREEN").as_deref() {
                Ok("openers") => self.screen = Screen::Openers,
                Ok("match") => self.start_match(MatchKind::Mini(1)),
                // headless check only: a strong-PC demo profile in a throwaway HOME
                Ok("recommend-demo") => {
                    self.seed_recommendation_demo();
                    if let Some(id) = self.create_recommendation(false) {
                        self.screen = Screen::Recommend(id, false);
                    }
                }
                _ => {}
            }
        }
        let screen = std::mem::replace(&mut self.screen, Screen::Dashboard);
        self.screen = match screen {
            Screen::Dashboard => {
                egui::CentralPanel::default().show(ui, |ui| self.dashboard(ui));
                // the dashboard's "openers" button replaces self.screen during the call
                std::mem::replace(&mut self.screen, Screen::Dashboard)
            }
            Screen::Loading => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.centered_and_justified(|ui| ui.label(self.s.get("loading_problem")));
                });
                Screen::Loading
            }
            Screen::Play(play) => self.play_screen(ui, play),
            Screen::Match(m) => self.match_screen(ui, m),
            Screen::Recommend(id, confirm) => {
                let mut next = None;
                egui::CentralPanel::default()
                    .show(ui, |ui| next = self.recommend_screen(ui, &id, confirm));
                next.unwrap_or(Screen::Recommend(id, confirm))
            }
            Screen::Openers => {
                let mut back = false;
                egui::CentralPanel::default().show(ui, |ui| back = self.openers_screen(ui));
                if back {
                    Screen::Dashboard
                } else {
                    Screen::Openers
                }
            }
        };
        self.take_autoshot(ui);
    }
}

fn main() -> eframe::Result {
    // wgpu by default (DX12/Vulkan on Windows); STACKLAB_RENDERER=glow selects OpenGL, which
    // also works under Xvfb/llvmpipe for headless checks.
    let renderer = if std::env::var("STACKLAB_RENDERER").is_ok_and(|v| v == "glow") {
        eframe::Renderer::Glow
    } else {
        eframe::Renderer::Wgpu
    };
    let options = eframe::NativeOptions {
        renderer,
        viewport: egui::ViewportBuilder::default()
            .with_title("StackLab")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    let _ = (Pos2::ZERO, Rect::NOTHING);
    eframe::run_native(
        "StackLab",
        options,
        Box::new(|cc| Ok(Box::new(StackApp::new(cc)))),
    )
}
