//! UI-09 (statistics · settings), the 40-line mode and the independent skill test, plus the
//! shared keyboard reader that maps user bindings to engine keys and commands.

use crate::draw::{field, mini_piece, FieldView};
use crate::{data_dir, Request, Screen, StackApp};
use eframe::egui::{self, Color32, RichText, Vec2};
use stacklab_engine::game::{InputEvent, Mode};
use stacklab_engine::replay::Recorder;
use stacklab_training::independent::{compare, due, IndependentResult, IndependentTest};
use stacklab_training::problem::Problem;
use stacklab_training::session::AttemptResult;
use stacklab_training::settings::{self as cfg, Action, Settings, SettingsError};

pub const SPRINT_LINES: u32 = 40;

pub struct SprintPlay {
    pub rec: Recorder,
    pub sim_time: f64,
    pub pending: Vec<InputEvent>,
    pub saved: bool,
    /// Wall-clock time when the 3-second countdown started (0 = not yet).
    pub countdown_from: f64,
}

/// Persisted under app_state "independent".
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct IndependentState {
    pub history: Vec<IndependentResult>,
    /// Problem ids served by earlier tests (never served again).
    pub seen: Vec<String>,
}

fn fmt_ticks(t: u64) -> String {
    let ms = t * 1000 / 60;
    format!("{}:{:02}.{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
}

impl StackApp {
    /// User bindings resolved to egui keys.
    fn key_actions(&self) -> Vec<(egui::Key, Action)> {
        self.settings
            .bindings
            .iter()
            .filter_map(|(a, k)| egui::Key::from_name(k).map(|k| (k, *a)))
            .collect()
    }

    /// Reads this frame's key events: game keys become timed engine inputs in `pending`,
    /// pause toggles `self.paused`; other pressed commands are returned.
    pub(crate) fn read_keys(
        &mut self,
        ui: &egui::Ui,
        base: u64,
        pending: &mut Vec<InputEvent>,
    ) -> Vec<Action> {
        let map = self.key_actions();
        let mut commands = Vec::new();
        let mut toggle = false;
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
                    for (_, a) in map.iter().filter(|(ek, _)| ek == key) {
                        match a {
                            Action::Game(k) => {
                                let t = base + (pending.len() as u64).min(9);
                                pending.push(InputEvent {
                                    time: t,
                                    key: *k,
                                    down: *pressed,
                                });
                            }
                            Action::Pause if *pressed => toggle = true,
                            other if *pressed => commands.push(*other),
                            _ => {}
                        }
                    }
                }
            }
        });
        if toggle {
            self.paused = !self.paused;
        }
        if self.paused {
            pending.clear();
        }
        commands
    }

    fn save_settings(&mut self) {
        if let Some(st) = &self.store {
            let _ = st.set_state(
                "settings",
                &serde_json::to_string(&self.settings).unwrap_or_default(),
            );
        }
    }

    fn action_name(&self, a: Action) -> String {
        self.s.get(&format!("action_{}", a.id()))
    }

    // ----- settings (UI-09 입력 설정) ------------------------------------------------------

    pub(crate) fn settings_screen(&mut self, ui: &mut egui::Ui) -> bool {
        let mut back = false;
        // capture the next key for a pending rebind
        if let Some((action, slot)) = self.rebinding {
            let pressed = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => Some(*key),
                    _ => None,
                })
            });
            if let Some(k) = pressed {
                self.rebinding = None;
                match self.settings.bind(action, slot, k.name()) {
                    Ok(()) => {
                        self.settings_msg = None;
                        self.save_settings();
                    }
                    Err(SettingsError::DuplicateKey { key, actions }) => {
                        self.settings_msg = Some(self.s.fmt(
                            "key_duplicate",
                            &[("k", key), ("a", self.action_name(actions.0))],
                        ));
                    }
                    Err(_) => {}
                }
            }
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(self.s.get("back_to_dashboard")).clicked() {
                    back = true;
                }
                ui.heading(self.s.get("settings"));
            });
            ui.label(RichText::new(self.s.get("handling")).strong());
            ui.label(RichText::new(self.s.get("handling_note")).weak());
            let mut h = self.settings.handling;
            let mut changed = false;
            let mut slider = |ui: &mut egui::Ui, label: String, v: &mut u32, r: (u32, u32)| {
                let mut f = cfg::frames(*v);
                let resp = ui.add(
                    egui::Slider::new(&mut f, cfg::frames(r.0)..=cfg::frames(r.1))
                        .step_by(0.1)
                        .fixed_decimals(1)
                        .suffix(" F")
                        .text(label),
                );
                if resp.changed() {
                    *v = cfg::sub_frames(f);
                    changed = true;
                }
            };
            slider(ui, "DAS".into(), &mut h.das, cfg::DAS_RANGE);
            slider(ui, "ARR".into(), &mut h.arr, cfg::ARR_RANGE);
            slider(ui, "DCD".into(), &mut h.dcd, cfg::DCD_RANGE);
            let mut sdf = self.settings.sdf_display();
            let sdf_resp = ui.add(
                egui::Slider::new(&mut sdf, cfg::SDF_RANGE.0..=cfg::SDF_RANGE.1)
                    .text("SDF")
                    .custom_formatter(|v, _| {
                        if v >= 41.0 {
                            "∞".into()
                        } else {
                            format!("{v:.0}X")
                        }
                    }),
            );
            if changed || sdf_resp.changed() {
                self.settings.handling.das = h.das;
                self.settings.handling.arr = h.arr;
                self.settings.handling.dcd = h.dcd;
                self.settings.set_sdf_display(sdf);
                if self.settings.validate().is_ok() {
                    self.save_settings();
                }
            }
            ui.add_space(10.0);
            ui.label(RichText::new(self.s.get("key_bindings")).strong());
            ui.label(RichText::new(self.s.get("key_bindings_note")).weak());
            egui::Grid::new("keys").striped(true).show(ui, |ui| {
                for a in Action::ALL {
                    ui.label(self.action_name(a));
                    let keys: Vec<String> = self
                        .settings
                        .keys_for(a)
                        .iter()
                        .map(|k| k.to_string())
                        .collect();
                    for (slot, k) in keys.iter().enumerate() {
                        let waiting = self.rebinding == Some((a, slot));
                        let label = if waiting {
                            self.s.get("press_key")
                        } else {
                            k.clone()
                        };
                        if ui.button(label).clicked() {
                            self.rebinding = Some((a, slot));
                        }
                    }
                    if keys.len() < 2 {
                        let waiting = self.rebinding == Some((a, keys.len()));
                        let label = if waiting {
                            self.s.get("press_key")
                        } else {
                            "+".to_string()
                        };
                        if ui.button(label).clicked() {
                            self.rebinding = Some((a, keys.len()));
                        }
                    } else if ui.small_button("−").clicked() {
                        let last = keys[1].clone();
                        self.settings
                            .bindings
                            .retain(|(b, k)| !(*b == a && *k == last));
                        self.save_settings();
                    }
                    ui.end_row();
                }
            });
            if let Some(m) = &self.settings_msg {
                ui.colored_label(Color32::from_rgb(240, 120, 100), m);
            }
            ui.horizontal(|ui| {
                if self.rebinding.is_some() && ui.button(self.s.get("cancel")).clicked() {
                    self.rebinding = None;
                }
                if ui.button(self.s.get("reset_defaults")).clicked() {
                    let overlay = self.settings.perf_overlay;
                    self.settings = Settings {
                        perf_overlay: overlay,
                        ..Settings::default()
                    };
                    self.settings_msg = None;
                    self.save_settings();
                }
            });
            ui.add_space(10.0);
            if ui
                .checkbox(&mut self.settings.perf_overlay, self.s.get("perf_overlay"))
                .changed()
            {
                self.save_settings();
            }
        });
        back
    }

    // ----- 40-line mode (spec 4.1) -------------------------------------------------------

    pub(crate) fn new_sprint(&mut self) -> SprintPlay {
        let seed = self.rng.below(u64::MAX >> 1);
        SprintPlay {
            rec: Recorder::new(
                self.rs.clone(),
                seed,
                self.settings.handling,
                Mode::Sprint {
                    lines: SPRINT_LINES,
                },
                None,
            ),
            sim_time: 0.0,
            pending: Vec::new(),
            saved: false,
            countdown_from: 0.0,
        }
    }

    pub(crate) fn sprint_screen(&mut self, ui: &mut egui::Ui, mut sp: Box<SprintPlay>) -> Screen {
        let over = sp.rec.game.is_over();
        let mut restart = false;
        if !over {
            let base = sp.rec.game.now_sub();
            let mut pending = std::mem::take(&mut sp.pending);
            for c in self.read_keys(ui, base, &mut pending) {
                if c == Action::Restart {
                    restart = true;
                }
            }
            sp.pending = pending;
            let now = ui.input(|i| i.time);
            if sp.countdown_from == 0.0 {
                sp.countdown_from = now;
            }
            let counting = now - sp.countdown_from < 3.0;
            if counting {
                sp.pending.clear();
            }
            if sp.sim_time == 0.0 || self.paused || counting {
                sp.sim_time = now;
            }
            let mut steps = 0;
            while sp.sim_time + 1.0 / 60.0 <= now && steps < 8 && !sp.rec.game.is_over() {
                let inputs = std::mem::take(&mut sp.pending);
                sp.rec.step(&inputs);
                let _ = sp.rec.game.events();
                sp.sim_time += 1.0 / 60.0;
                steps += 1;
            }
            if steps == 8 {
                sp.sim_time = now;
            }
            ui.ctx().request_repaint();
        }
        let g = &sp.rec.game;
        let stats = g.stats();
        if g.is_over() && !sp.saved {
            sp.saved = true;
            if let Some(st) = &self.store {
                let log = sp.rec.finish();
                let _ = st.save_sprint(
                    stats.finish_tick,
                    stats.pieces,
                    stats.inputs,
                    &log.to_json(),
                    &self.rs.meta.version,
                );
            }
        }
        let ticks = stats.finish_tick.unwrap_or(g.tick());
        let secs = (ticks as f64 / 60.0).max(1e-9);
        let mut back = false;
        egui::Panel::left("sprint_info")
            .exact_size(260.0)
            .show(ui, |ui| {
                ui.heading(self.s.get("sprint"));
                ui.label(self.s.fmt(
                    "sprint_lines",
                    &[
                        ("n", stats.lines.min(SPRINT_LINES).to_string()),
                        ("m", SPRINT_LINES.to_string()),
                    ],
                ));
                ui.label(RichText::new(fmt_ticks(ticks)).size(26.0).monospace());
                ui.label(format!("PPS {:.2}", stats.pieces as f64 / secs));
                ui.label(self.s.fmt(
                    "sprint_inputs",
                    &[
                        ("i", stats.inputs.to_string()),
                        (
                            "k",
                            format!("{:.2}", stats.inputs as f64 / stats.pieces.max(1) as f64),
                        ),
                    ],
                ));
                if self.paused {
                    ui.colored_label(Color32::YELLOW, self.s.get("paused"));
                }
                let left = 3.0 - (ui.input(|i| i.time) - sp.countdown_from);
                if sp.countdown_from > 0.0 && left > 0.0 {
                    ui.label(RichText::new(format!("{}", left.ceil() as u32)).size(40.0));
                }
                ui.add_space(8.0);
                if g.is_over() {
                    let t = if stats.finish_tick.is_some() {
                        self.s.get("sprint_done")
                    } else {
                        self.s.get("sprint_failed")
                    };
                    ui.label(RichText::new(t).strong());
                }
                if ui.button(self.s.get("restart_btn")).clicked() {
                    restart = true;
                }
                if ui.button(self.s.get("back_to_dashboard")).clicked() {
                    back = true;
                }
                ui.add_space(12.0);
                ui.label(self.s.get("hold"));
                let (resp, painter) =
                    ui.allocate_painter(Vec2::new(120.0, 70.0), egui::Sense::hover());
                if let Some(h) = g.hold_piece() {
                    mini_piece(&painter, resp.rect.center(), 16.0, h, false);
                }
            });
        egui::Panel::right("sprint_next")
            .exact_size(140.0)
            .show(ui, |ui| {
                ui.label("NEXT");
                for p in g.next_pieces() {
                    let (resp, painter) =
                        ui.allocate_painter(Vec2::new(120.0, 60.0), egui::Sense::hover());
                    mini_piece(&painter, resp.rect.center(), 14.0, p, false);
                }
            });
        egui::CentralPanel::default().show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            field(
                ui.painter(),
                rect.shrink(8.0),
                &FieldView {
                    board: g.board(),
                    active: g.active_piece(),
                    ghost: g.ghost(),
                    targets: vec![],
                    colored: vec![],
                    colors: Some(g.colors()),
                },
            );
        });
        if back {
            self.paused = false;
            return Screen::Dashboard;
        }
        if restart {
            self.paused = false;
            return Screen::Sprint(Box::new(self.new_sprint()));
        }
        Screen::Sprint(sp)
    }

    // ----- independent skill test (spec 6.4) ---------------------------------------------

    pub(crate) fn independent_offer(&mut self, ui: &mut egui::Ui) {
        let completed = self.st.set.number.saturating_sub(1);
        if due(
            &self.indep_state.history,
            completed,
            self.policy.cycle.independent_test_every_sets,
        ) {
            ui.horizontal(|ui| {
                ui.label(self.s.get("indep_offer"));
                if ui.button(self.s.get("indep_start")).clicked() {
                    self.start_independent();
                }
            });
        }
    }

    pub(crate) fn start_independent(&mut self) {
        self.indep = Some(IndependentTest::new(
            self.indep_state.history.len() as u32 + 1,
        ));
        self.request_independent();
    }

    pub(crate) fn request_independent(&mut self) {
        let Some((m, seed)) = self.indep.as_ref().and_then(|t| t.next()) else {
            return;
        };
        let req = Request::Independent(
            m,
            seed,
            Box::new(self.st.profile.loadout.clone()),
            self.indep_state.seen.clone(),
        );
        if self.worker.0.send(req).is_ok() {
            self.screen = Screen::Loading;
        }
    }

    pub(crate) fn record_independent(&mut self, p: &Problem, r: &AttemptResult) {
        let Some(t) = self.indep.as_mut() else {
            return;
        };
        t.record(p, r);
        self.indep_state.seen.push(p.id.clone());
        if t.is_complete() {
            let res = t.finish(
                self.st.set.number.saturating_sub(1),
                stacklab_store::now_utc(),
            );
            self.indep_state.history.push(res);
            self.indep = None;
            self.stats_tab = 3;
        }
        if let Some(st) = &self.store {
            let _ = st.set_state(
                "independent",
                &serde_json::to_string(&self.indep_state).unwrap_or_default(),
            );
        }
    }

    // ----- statistics (UI-09) ------------------------------------------------------------

    pub(crate) fn stats_screen(&mut self, ui: &mut egui::Ui) -> bool {
        let mut back = false;
        ui.horizontal(|ui| {
            if ui.button(self.s.get("back_to_dashboard")).clicked() {
                back = true;
            }
            ui.heading(self.s.get("stats"));
        });
        ui.horizontal(|ui| {
            for (i, k) in [
                "tab_skills",
                "tab_matches",
                "tab_sprint",
                "tab_independent",
                "tab_tl",
                "tab_backup",
            ]
            .iter()
            .enumerate()
            {
                if ui
                    .selectable_label(self.stats_tab == i as u8, self.s.get(k))
                    .clicked()
                {
                    self.stats_tab = i as u8;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| match self.stats_tab {
            0 => self.stats_skills(ui),
            1 => self.stats_matches(ui),
            2 => self.stats_sprint(ui),
            3 => self.stats_independent(ui),
            4 => self.stats_tl(ui),
            _ => self.backup_section(ui),
        });
        back
    }

    fn stats_skills(&mut self, ui: &mut egui::Ui) {
        let rows = self
            .store
            .as_ref()
            .and_then(|s| s.module_stats().ok())
            .unwrap_or_default();
        egui::Grid::new("skill_stats").striped(true).show(ui, |ui| {
            for h in [
                "col_module",
                "col_rating",
                "col_attempts",
                "col_success",
                "col_valid",
                "col_finesse",
            ] {
                ui.label(RichText::new(self.s.get(h)).strong());
            }
            ui.end_row();
            for (m, r) in &self.st.profile.skills {
                ui.label(format!("{} {}", m.letter(), self.module_name(*m)));
                ui.label(format!("{:.0} ± {:.0}", r.rating, r.rd));
                let row = rows.iter().find(|x| x.0 == m.letter().to_string());
                let (n, s, v, fs, fj) = row.map_or((0, 0, 0, 0, 0), |x| (x.1, x.2, x.3, x.4, x.5));
                ui.label(n.to_string());
                ui.label(if n > 0 {
                    format!("{:.0}%", 100.0 * s as f64 / n as f64)
                } else {
                    "-".into()
                });
                ui.label(v.to_string());
                ui.label(if fj > 0 {
                    format!("{:.0}%", 100.0 * fs as f64 / fj as f64)
                } else {
                    "-".into()
                });
                ui.end_row();
            }
        });
    }

    fn stats_matches(&mut self, ui: &mut egui::Ui) {
        let ms = self
            .store
            .as_ref()
            .and_then(|s| s.list_matches(100).ok())
            .unwrap_or_default();
        ui.label(self.s.fmt(
            "match_rating_line",
            &[
                ("r", format!("{:.0}", self.st.matchp.rating.rating)),
                ("d", format!("{:.0}", self.st.matchp.rating.rd)),
                ("n", self.st.matchp.evaluations.to_string()),
            ],
        ));
        if ms.is_empty() {
            ui.label(RichText::new(self.s.get("no_records")).weak());
            return;
        }
        egui::Grid::new("matches").striped(true).show(ui, |ui| {
            for m in &ms {
                ui.label(&m.created_at);
                ui.label(if m.kind == "evaluation" {
                    self.s.get("kind_evaluation")
                } else {
                    self.s.get("kind_mini")
                });
                ui.label(format!("AI {} ({:.0})", m.bot_preset_id, m.bot_rating));
                ui.label(match m.result {
                    Some(true) => self.s.get("result_win"),
                    Some(false) => self.s.get("result_loss"),
                    None => self.s.get("status_in_progress"),
                });
                ui.label(m.rating_delta.map_or(String::new(), |d| format!("{d:+.0}")));
                ui.end_row();
            }
        });
    }

    fn stats_sprint(&mut self, ui: &mut egui::Ui) {
        if ui.button(self.s.get("sprint_start")).clicked() {
            self.screen = Screen::Sprint(Box::new(self.new_sprint()));
        }
        let rs = self
            .store
            .as_ref()
            .and_then(|s| s.sprint_records(50).ok())
            .unwrap_or_default();
        if rs.is_empty() {
            ui.label(RichText::new(self.s.get("no_records")).weak());
            return;
        }
        egui::Grid::new("sprints").striped(true).show(ui, |ui| {
            for r in &rs {
                ui.label(&r.created_at);
                match r.finish_ticks {
                    Some(t) => {
                        ui.label(RichText::new(fmt_ticks(t)).monospace());
                        ui.label(format!("PPS {:.2}", r.pieces as f64 / (t as f64 / 60.0)));
                    }
                    None => {
                        ui.label(self.s.get("sprint_failed"));
                        ui.label("");
                    }
                }
                ui.label(self.s.fmt(
                    "sprint_inputs",
                    &[
                        ("i", r.inputs.to_string()),
                        (
                            "k",
                            format!("{:.2}", r.inputs as f64 / r.pieces.max(1) as f64),
                        ),
                    ],
                ));
                ui.end_row();
            }
        });
    }

    fn stats_independent(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(self.s.get("indep_note")).weak());
        if ui.button(self.s.get("indep_start")).clicked() {
            self.start_independent();
        }
        let h = self.indep_state.history.clone();
        if h.is_empty() {
            ui.label(RichText::new(self.s.get("no_records")).weak());
            return;
        }
        for (i, r) in h.iter().enumerate().rev() {
            ui.separator();
            ui.label(
                RichText::new(self.s.fmt(
                    "indep_result",
                    &[
                        ("n", r.number.to_string()),
                        ("d", r.created_at.clone()),
                        ("p", format!("{:.0}", 100.0 * r.success_rate)),
                        ("t", format!("{:.0}", 100.0 * r.mean_time_share)),
                    ],
                ))
                .strong(),
            );
            if i > 0 {
                let (pp, per) = compare(&h[i - 1], r);
                ui.label(self.s.fmt("indep_delta", &[("p", format!("{pp:+.0}"))]));
                let line: Vec<String> = per
                    .iter()
                    .map(|(m, d)| format!("{} {d:+}", m.letter()))
                    .collect();
                ui.label(RichText::new(line.join(" · ")).weak());
            }
            let line: Vec<String> = r
                .per_module
                .iter()
                .map(|(m, (n, s))| format!("{} {s}/{n}", m.letter()))
                .collect();
            ui.label(line.join(" · "));
        }
    }

    fn stats_tl(&mut self, ui: &mut egui::Ui) {
        self.match_rating_panel(ui);
        ui.separator();
        let anchors = self.st.matchp.anchors.clone();
        if anchors.is_empty() {
            ui.label(RichText::new(self.s.get("no_records")).weak());
        }
        egui::Grid::new("tl").striped(true).show(ui, |ui| {
            for a in anchors.iter().rev() {
                ui.label(&a.entered_at);
                ui.label(format!("TL {:.0} ± {:.0}", a.glicko, a.rd));
                ui.label(self.s.fmt(
                    "tl_offset",
                    &[
                        ("a", format!("{:.0}", a.app_rating_at_entry)),
                        ("o", format!("{:+.0}", a.offset)),
                    ],
                ));
                ui.end_row();
            }
        });
    }

    // ----- backup / restore (spec 10.1) ----------------------------------------------------

    fn backup_dir() -> std::path::PathBuf {
        data_dir().join("backups")
    }

    fn backup_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(self.s.get("backup_note")).weak());
        ui.label(
            RichText::new(Self::backup_dir().display().to_string())
                .monospace()
                .weak(),
        );
        if ui.button(self.s.get("backup_create")).clicked() {
            self.backup_msg = Some(match self.make_backup() {
                Ok(p) => self.s.fmt("backup_done", &[("f", p)]),
                Err(e) => self.s.fmt("backup_failed", &[("e", e)]),
            });
        }
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(Self::backup_dir())
            .map(|d| {
                d.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files.reverse();
        for f in files.iter().take(20) {
            ui.horizontal(|ui| {
                ui.label(
                    f.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
                if self.restore_confirm.as_ref() == Some(f) {
                    ui.colored_label(Color32::YELLOW, self.s.get("restore_warn"));
                    if ui.button(self.s.get("restore_confirm")).clicked() {
                        self.restore_confirm = None;
                        self.backup_msg = Some(match self.restore_backup(f) {
                            Ok(()) => self.s.get("restore_done"),
                            Err(e) => self.s.fmt("restore_failed", &[("e", e)]),
                        });
                    }
                    if ui.button(self.s.get("cancel")).clicked() {
                        self.restore_confirm = None;
                    }
                } else if ui.button(self.s.get("restore")).clicked() {
                    self.restore_confirm = Some(f.clone());
                }
            });
        }
        if let Some(m) = &self.backup_msg {
            ui.label(m);
        }
    }

    fn make_backup(&mut self) -> Result<String, String> {
        self.persist();
        let st = self.store.as_ref().ok_or("no database")?;
        let json = st.backup_json().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(Self::backup_dir()).map_err(|e| e.to_string())?;
        let name = format!(
            "stacklab-{}.json",
            stacklab_store::now_utc().replace([':', '-'], "")
        );
        let path = Self::backup_dir().join(&name);
        std::fs::write(&path, json).map_err(|e| e.to_string())?;
        Ok(name)
    }

    /// Atomic restore, then reload everything that lives in memory.
    fn restore_backup(&mut self, path: &std::path::Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let st = self.store.as_mut().ok_or("no database")?;
        st.restore_json(&text).map_err(|e| e.to_string())?;
        let ckpt = st.read_checkpoint().ok().flatten();
        self.st = ckpt
            .as_ref()
            .and_then(|c| serde_json::from_str(&c.training).ok())
            .unwrap_or_else(|| {
                stacklab_training::flow::TrainingState::new(&self.policy, self.rng.below(1 << 40))
            });
        self.current_problem = ckpt.as_ref().and_then(|c| c.current_problem.clone());
        self.st.profile.loadout = st.load_loadout().ok().flatten();
        self.settings = st
            .get_state("settings")
            .ok()
            .flatten()
            .and_then(|j| serde_json::from_str::<Settings>(&j).ok())
            .filter(|s| s.validate().is_ok())
            .unwrap_or_default();
        self.indep_state = st
            .get_state("independent")
            .ok()
            .flatten()
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        self.indep = None;
        Ok(())
    }
}
