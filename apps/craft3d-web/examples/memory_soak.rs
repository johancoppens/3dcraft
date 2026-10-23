//! What the web build keeps as a Jig is used for a long time, measured
//! natively on the page's own host code (`WebHost`, the shell's sync that
//! builds the viewport's scene): every parameter set as a commit of its
//! own (the page commits once per drag), then the documents closed.
//! Prints the scene cache, the meshes of bodies and the process's
//! resident memory after each step, so growth shows (R1 B7).
//!
//! `cargo run -p craft3d-web --example memory_soak -- JIG [STEPS]`

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    use craft3d_ui_egui::Shell;
    use craft3d_web::host::WebHost;
    use serde_json::{Value, json};

    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: memory_soak JIG.3dcraftjig [STEPS]")?;
    let steps: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let name = std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("jig.3dcraftjig")
        .to_string();
    let mut host = WebHost::new(Shell::new());
    // The bodies a regeneration keeps, as on the page (`craft3d_regen`'s
    // wasm budget).
    host.session.set_body_budget(1 << 28);
    let rss = || -> f64 {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<f64>().ok())
            .map_or(0.0, |pages| pages * 4096.0 / (1 << 20) as f64)
    };
    let mb = |b: usize| b as f64 / (1 << 20) as f64;
    let report = |host: &mut WebHost, what: &str| {
        host.shell.sync(&mut host.session);
        println!(
            "{what:<28} scenes {:>8.1} MiB  meshes {:>8.1} MiB  undo {:>3}  rss {:>8.1} MiB",
            mb(host.session.scene_bytes()),
            mb(host.session.mesh_bytes()),
            host.session
                .execute("document.inspect", &json!({"measures": false}))
                .ok()
                .and_then(|v| v.pointer("/history/undo")?.as_array().map(Vec::len))
                .unwrap_or(0),
            rss()
        );
    };
    report(&mut host, "start");
    host.receive(&name, bytes.clone(), None)?;
    report(&mut host, "opened");
    let list = host
        .session
        .execute("param.list", &json!({}))
        .map_err(|e| e.to_string())?;
    // Every number parameter with a range, swept from its minimum to its
    // maximum and back, one commit per step, in turn.
    let sweeps: Vec<(String, f64, f64)> = list
        .get("parameters")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .filter(|p| {
                    matches!(
                        p.get("kind").and_then(Value::as_str),
                        Some("number" | "integer")
                    )
                })
                .filter_map(|p| {
                    Some((
                        p.get("name")?.as_str()?.to_string(),
                        p.pointer("/min/value")?.as_f64()?,
                        p.pointer("/max/value")?.as_f64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    if sweeps.is_empty() {
        return Err("the Jig has no number parameter with a range".to_string());
    }
    for step in 0..steps {
        let Some((name, lo, hi)) = sweeps.get(step % sweeps.len()) else {
            break;
        };
        let t = ((step / sweeps.len()) % 5) as f64 / 4.0;
        let value = lo + (hi - lo) * (0.25 + 0.5 * t);
        let value = if (value - value.round()).abs() < 1e-9 || value.abs() > 1.0 {
            value.round()
        } else {
            value
        };
        let mut values = serde_json::Map::new();
        values.insert(name.clone(), Value::from(value));
        let outcome = host
            .session
            .execute("param.set", &json!({"values": values, "clamp": true}));
        if let Err(e) = outcome {
            println!("step {step}: {name} = {value}: {}", e.message());
        }
        if step % 5 == 4 || step + 1 == steps {
            report(&mut host, &format!("after {} commits", step + 1));
        }
    }
    // Two more documents open beside it, then every document closed.
    host.receive("b.3dcraftjig", bytes.clone(), None)?;
    host.receive("c.3dcraftjig", bytes, None)?;
    report(&mut host, "three open");
    while let Some(status) = host.session.document_status() {
        host.session
            .execute("file.close", &json!({"discard": true}))
            .map_err(|e| format!("file.close {}: {e}", status.id))?;
    }
    report(&mut host, "all closed");
    Ok(())
}
