//! StackLab desktop app (STEP 3 skeleton): dashboard (UI-01, minimal), problem training (UI-03)
//! and problem result / rewind (UI-04). Matches, openers, settings and statistics come later.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod draw;
mod strings;

use draw::{field, mini_piece, FieldView};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Vec2};
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
            });
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
            worker: spawn_worker(rs, policy.clone(), cc.egui_ctx.clone()),
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
            self.saved = SavedCycle {
                cycle: Cycle::new(plan_cycle(&self.profile, &self.policy, &mut self.rng)),
                diagnosis: false,
                number: self.saved.number + 1,
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
            self.s.fmt(
                "cycle_progress",
                &[
                    ("c", ((self.saved.number - 1) % 3 + 1).to_string()),
                    ("n", n.to_string()),
                ],
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
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new(label).size(20.0)))
                .clicked()
            {
                self.request_next();
            }
            ui.label(self.progress_text());
            if ui.button(self.s.get("openers")).clicked() {
                self.screen = Screen::Openers;
            }
        });
        ui.label(RichText::new(self.s.get("next_match_soon")).weak());
        if let Some(r) = &self.last_report {
            ui.label(r);
        }
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
        if std::env::var("STACKLAB_AUTOSCREEN").is_ok_and(|v| v == "openers")
            && self.autoshot.as_ref().is_some_and(|a| a.1 == 1)
        {
            self.screen = Screen::Openers;
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
