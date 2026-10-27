//! Where the time goes when a parameter of a Jig is set on the page,
//! measured natively on the page's own host code: `WebHost` with the long
//! work a piece per frame (`set_stepwise`, as `page.rs` sets it), the
//! Parameters form committing once per drag (one thread), and the frames
//! the page runs: `pump` (where the form's `param.set` runs), `prepare`
//! with the page's 40 ms budget, and the shell drawn (as it was while the
//! scene is away, from the session once it is there).
//!
//! Per frame it records how long each of the three took and the work the
//! counters of regeneration, the kernel, mass properties and the
//! tessellator saw, grouped by what the frame was doing; so work done twice
//! for one change shows as two regenerations or the same bodies meshed
//! twice.
//!
//! `taskset -c 0 cargo run --release -p craft3d-web --example param_timing -- JIG NAME=VALUE [NAME=VALUE ...]`
//!
//! (`taskset -c 0`: one core, as the page has one thread.) Each
//! `NAME=VALUE` is one commit, in the parameter's unit, as the form sends
//! it (`ui.param`'s settle and clamp).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    run::main()
}

#[cfg(not(target_arch = "wasm32"))]
mod run {
    use craft3d_ui_egui::Shell;
    use craft3d_web::host::WebHost;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    /// The page's budget of work per frame (`page.rs`, FRAME_WORK_MS).
    const FRAME_WORK: Duration = Duration::from_millis(40);

    #[derive(Clone, Copy, Default)]
    struct Counts {
        regenerations: u64,
        rebuilt: u64,
        booleans: u64,
        mass_faces: u64,
        meshes: u64,
        triangles: u64,
    }

    fn counts() -> Counts {
        let regen = craft3d_regen::work::work();
        let kernel = craft3d_kernel::work::work();
        let (mass_faces, _) = craft3d_topo::mass_work();
        let (meshes, _, triangles) = craft3d_tess::tess_work();
        Counts {
            regenerations: regen.regenerations,
            rebuilt: regen.features_rebuilt,
            booleans: kernel.booleans,
            mass_faces,
            meshes,
            triangles,
        }
    }

    impl Counts {
        fn since(self, before: Counts) -> Counts {
            Counts {
                regenerations: self.regenerations - before.regenerations,
                rebuilt: self.rebuilt - before.rebuilt,
                booleans: self.booleans - before.booleans,
                mass_faces: self.mass_faces - before.mass_faces,
                meshes: self.meshes - before.meshes,
                triangles: self.triangles - before.triangles,
            }
        }
        fn add(&mut self, o: Counts) {
            self.regenerations += o.regenerations;
            self.rebuilt += o.rebuilt;
            self.booleans += o.booleans;
            self.mass_faces += o.mass_faces;
            self.meshes += o.meshes;
            self.triangles += o.triangles;
        }
        fn text(self) -> String {
            format!(
                "regen {:>2} rebuilt {:>4} bool {:>4} mass {:>5} meshed {:>4} tri {:>8}",
                self.regenerations,
                self.rebuilt,
                self.booleans,
                self.mass_faces,
                self.meshes,
                self.triangles
            )
        }
    }

    #[derive(Default)]
    struct Stage {
        frames: usize,
        ms: f64,
        longest: f64,
        work: Counts,
    }

    struct Frames {
        ctx: egui::Context,
        time: f64,
        stages: BTreeMap<String, Stage>,
        order: Vec<String>,
        frames: usize,
        verbose: bool,
    }

    impl Frames {
        fn note(&mut self, stage: String, ms: f64, work: Counts) {
            if !self.stages.contains_key(&stage) {
                self.order.push(stage.clone());
            }
            let s = self.stages.entry(stage).or_default();
            s.frames += 1;
            s.ms += ms;
            s.longest = s.longest.max(ms);
            s.work.add(work);
        }

        /// One frame as `page.rs` runs it; false once nothing is left.
        fn frame(&mut self, host: &mut WebHost) -> bool {
            self.frames += 1;
            self.time += 1.0 / 60.0;
            let c = counts();
            let t = Instant::now();
            let busy_before = host.shell.parameters_busy();
            host.pump();
            let pump = t.elapsed().as_secs_f64() * 1e3;
            let w = counts().since(c);
            if pump > 1.0 || w.regenerations > 0 || w.meshes > 0 {
                let what = if busy_before && !host.shell.parameters_busy() {
                    "pump: param.set"
                } else {
                    "pump"
                };
                self.note(what.to_string(), pump, w);
            }
            let c = counts();
            let t = Instant::now();
            let started = Instant::now();
            let busy = host.prepare(move || started.elapsed() > FRAME_WORK);
            let prepare = t.elapsed().as_secs_f64() * 1e3;
            let w = counts().since(c);
            let doing = host
                .shell
                .work()
                .map(|w| format!("{} {}", w.label, w.phase))
                .unwrap_or_else(|| {
                    if busy {
                        "checking".to_string()
                    } else {
                        "idle".to_string()
                    }
                });
            if busy || prepare > 1.0 {
                self.note(format!("prepare: {doing}"), prepare, w);
            }
            let c = counts();
            let t = Instant::now();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 760.0),
                )),
                time: Some(self.time),
                ..Default::default()
            };
            let away = host.away();
            let mut output = self.ctx.run_ui(input, |ui| {
                let WebHost { session, shell, .. } = host;
                if away {
                    shell.ui_away(ui);
                } else {
                    shell.ui(ui, session);
                }
            });
            output.textures_delta.clear();
            let ui = t.elapsed().as_secs_f64() * 1e3;
            let w = counts().since(c);
            self.note(if away { "ui away" } else { "ui" }.to_string(), ui, w);
            if self.verbose {
                println!(
                    "    frame {:>4}: pump {pump:7.1}  prepare {prepare:7.1} ({doing})  ui {ui:7.1}{}",
                    self.frames,
                    if away { " away" } else { "" }
                );
            }
            busy || host.shell.parameters_busy() || host.away()
        }

        fn run_until_idle(&mut self, host: &mut WebHost, label: &str) {
            self.stages.clear();
            self.order.clear();
            self.frames = 0;
            let start = Instant::now();
            let mut until_picture = None;
            let mut idle = 0;
            // Idle twice in a row: a frame may hand the next one work.
            while idle < 2 && self.frames < 20_000 {
                let more = self.frame(host);
                if until_picture.is_none() && !host.away() && !host.shell.parameters_busy() {
                    until_picture = Some(start.elapsed().as_secs_f64());
                }
                idle = if more { 0 } else { idle + 1 };
            }
            let total = start.elapsed().as_secs_f64();
            let quality = host.shell.scene_quality();
            let mut tolerance = |q| {
                host.session
                    .scene_with(q)
                    .ok()
                    .flatten()
                    .map_or(0.0, |s| s.tolerance)
            };
            let (preview, display) = (
                tolerance(craft3d_engine::SceneQuality::Preview),
                tolerance(quality),
            );
            println!(
                "  {label}: {total:.2} s in {} frames, new picture after {:.2} s (tolerance: preview {preview:e}, display {display:e})",
                self.frames,
                until_picture.unwrap_or(total)
            );
            for name in &self.order {
                if let Some(s) = self.stages.get(name) {
                    if s.ms < 1.0 && s.work.regenerations == 0 && s.work.meshes == 0 {
                        continue;
                    }
                    println!(
                        "    {name:<34} {:>4} frames {:>9.1} ms (longest {:>8.1})  {}",
                        s.frames,
                        s.ms,
                        s.longest,
                        s.work.text()
                    );
                }
            }
        }
    }

    pub fn main() -> Result<(), String> {
        let mut args = std::env::args().skip(1);
        let path = args
            .next()
            .ok_or("usage: param_timing JIG.3dcraftjig NAME=VALUE ...")?;
        let edits: Vec<(String, Value)> = args
            .filter_map(|a| {
                let (n, v) = a.split_once('=')?;
                let value = v
                    .parse::<f64>()
                    .map_or_else(|_| Value::from(v), Value::from);
                Some((n.to_string(), value))
            })
            .collect();
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        let name = std::path::Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("jig.3dcraftjig")
            .to_string();
        let mut host = WebHost::new(Shell::new());
        host.shell.set_parameters_single_thread(true);
        host.set_stepwise(true);
        host.session.set_body_budget(1 << 28);
        let mut frames = Frames {
            ctx: egui::Context::default(),
            time: 0.0,
            stages: BTreeMap::new(),
            order: Vec::new(),
            frames: 0,
            verbose: std::env::var_os("VERBOSE").is_some(),
        };
        println!("== {name} ({} threads)", craft3d_math::available_threads());
        let t = Instant::now();
        host.receive(&name, bytes, None)?;
        println!("  receive: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        frames.run_until_idle(&mut host, "open");
        for (param, value) in edits {
            let asked = host
                .shell
                .ui_param_queue(
                    &mut host.session,
                    Some(&json!({"name": param, "value": value})),
                )
                .map_err(|e| format!("{param}: {e}"))?;
            let _ = asked;
            frames.run_until_idle(&mut host, &format!("{param} = {value}"));
            if let Some(report) = &host.shell.last_command {
                let text: String = report.text.chars().take(100).collect();
                println!("    -> {} {}: {text}", report.label, report.ok);
            }
        }
        Ok(())
    }
}
