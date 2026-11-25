//! Performance overlay and log (spec 12.4 / 14.4 measuring tool): FPS, frame time, CPU time per
//! frame, simulation and bot search time, and the input processing delay — the time from
//! reading a key event to the simulation frame that consumes it. With the overlay on, one CSV
//! row per second is appended to `<data dir>/perf/perf-<start>.csv`.

use eframe::egui::{self, Color32, RichText};
use std::collections::VecDeque;
use std::io::Write;
use std::time::Instant;

const WINDOW: usize = 240;

#[derive(Default)]
pub struct Perf {
    last_frame: Option<Instant>,
    dts: VecDeque<f64>,
    cpu: VecDeque<f64>,
    sim: VecDeque<f64>,
    lat: VecDeque<f64>,
    bot_ms: f64,
    stalls: u64,
    read_at: Option<Instant>,
    log: Option<std::fs::File>,
    last_log: Option<Instant>,
}

fn push(q: &mut VecDeque<f64>, v: f64) {
    if q.len() == WINDOW {
        q.pop_front();
    }
    q.push_back(v);
}

fn avg(q: &VecDeque<f64>) -> f64 {
    if q.is_empty() {
        0.0
    } else {
        q.iter().sum::<f64>() / q.len() as f64
    }
}

fn max(q: &VecDeque<f64>) -> f64 {
    q.iter().copied().fold(0.0, f64::max)
}

impl Perf {
    /// Call once at the start of every UI frame.
    pub fn begin_frame(&mut self, cpu_secs: Option<f32>) {
        let now = Instant::now();
        if let Some(l) = self.last_frame {
            push(&mut self.dts, (now - l).as_secs_f64() * 1000.0);
        }
        self.last_frame = Some(now);
        if let Some(c) = cpu_secs {
            push(&mut self.cpu, c as f64 * 1000.0);
        }
    }

    /// Game key events were read this frame.
    pub fn inputs_read(&mut self) {
        if self.read_at.is_none() {
            self.read_at = Some(Instant::now());
        }
    }

    /// Simulation frames ran (`steps` > 0) taking `ms`; pending inputs are now consumed.
    pub fn sim_ran(&mut self, steps: u32, ms: f64) {
        if steps == 0 {
            return;
        }
        push(&mut self.sim, ms / steps as f64);
        if let Some(t) = self.read_at.take() {
            push(&mut self.lat, t.elapsed().as_secs_f64() * 1000.0);
        }
    }

    pub fn bot(&mut self, think_ms: f64, stalls: u64) {
        self.bot_ms = think_ms;
        self.stalls = stalls;
    }

    pub fn fps(&self) -> f64 {
        let a = avg(&self.dts);
        if a > 0.0 {
            1000.0 / a
        } else {
            0.0
        }
    }

    pub fn overlay(&self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::unique("perf_overlay"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-8.0, -8.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let line = |ui: &mut egui::Ui, t: String, warn: bool| {
                        let c = if warn {
                            Color32::from_rgb(240, 120, 100)
                        } else {
                            Color32::LIGHT_GRAY
                        };
                        ui.add(
                            egui::Label::new(RichText::new(t).monospace().size(11.0).color(c))
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    };
                    let fps = self.fps();
                    line(ui, format!("FPS        {fps:6.1}"), fps < 59.0);
                    line(
                        ui,
                        format!(
                            "frame ms   {:6.2} (max {:.1})",
                            avg(&self.dts),
                            max(&self.dts)
                        ),
                        false,
                    );
                    line(ui, format!("cpu ms     {:6.2}", avg(&self.cpu)), false);
                    line(ui, format!("sim ms/f   {:6.3}", avg(&self.sim)), false);
                    line(
                        ui,
                        format!("bot ms     {:6.2} (wait {})", self.bot_ms, self.stalls),
                        false,
                    );
                    let la = avg(&self.lat);
                    line(
                        ui,
                        format!("input ms   {la:6.2} (max {:.1})", max(&self.lat)),
                        max(&self.lat) > 16.0,
                    );
                });
            });
    }

    /// Appends one CSV row per second while enabled.
    pub fn log(&mut self, dir: &std::path::Path, screen: &str) {
        let now = Instant::now();
        if self.last_log.is_some_and(|l| (now - l).as_secs_f64() < 1.0) {
            return;
        }
        self.last_log = Some(now);
        if self.log.is_none() {
            let _ = std::fs::create_dir_all(dir);
            let name = format!(
                "perf-{}.csv",
                stacklab_store::now_utc().replace([':', '-'], "")
            );
            let path = dir.join(name);
            if let Ok(mut f) = std::fs::File::create(&path) {
                let _ = writeln!(
                    f,
                    "utc,screen,fps,frame_ms_avg,frame_ms_max,cpu_ms_avg,sim_ms_per_frame,bot_ms,bot_waits,input_ms_avg,input_ms_max"
                );
                self.log = Some(f);
            }
        }
        let row = format!(
            "{},{},{:.1},{:.2},{:.2},{:.2},{:.3},{:.2},{},{:.2},{:.2}",
            stacklab_store::now_utc(),
            screen,
            self.fps(),
            avg(&self.dts),
            max(&self.dts),
            avg(&self.cpu),
            avg(&self.sim),
            self.bot_ms,
            self.stalls,
            avg(&self.lat),
            max(&self.lat)
        );
        if let Some(f) = self.log.as_mut() {
            let _ = writeln!(f, "{row}");
        }
    }

    pub fn stop_log(&mut self) {
        self.log = None;
    }
}
