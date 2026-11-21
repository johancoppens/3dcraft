//! Temporary (investigation): prints the commands of two seeds, ids
//! filled in, group by group, with what each group came to.

use craft3d_fuzz::runner::Applied;
use craft3d_fuzz::steps::{map_strings, resolved_text};
use craft3d_fuzz::{Deep, Generator, Options, Runner, Setup};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn dump(seed: u64) {
    let setup = Setup {
        seed,
        scale: Generator::scale_of(seed),
    };
    let options = Options {
        deep: Deep::EveryGroup,
        identities: false,
        determinism: false,
        scratch: PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fuzz-dump"),
    };
    let mut generator = Generator::new(setup);
    let mut runner = Runner::new(setup, options).expect("a runner");
    println!(
        "SEED {seed} scale {} tolerance {}",
        setup.scale,
        setup.tolerance()
    );
    let mut all = Vec::new();
    for k in 0..Generator::length_of(seed) {
        let group = generator.group(&mut runner, k);
        let ids: BTreeMap<String, String> = runner
            .names()
            .iter()
            .map(|(id, name)| (name.clone(), id.clone()))
            .collect();
        let cmds: Vec<_> = group
            .commands
            .iter()
            .map(|c| {
                json!({"command": c.command,
                       "params": map_strings(&c.params, &|s| resolved_text(s, &ids), 0)})
            })
            .collect();
        let refused_before = runner.report.refused.len();
        let applied = runner.apply(&group);
        println!("GROUP {seed} {k} {} {applied:?}", group.step);
        println!("CMDS {seed} {k} {}", serde_json::Value::Array(cmds.clone()));
        if let Some(r) = runner.report.refused.get(refused_before) {
            println!("REFUSED {seed} {k} {}: {}", r.command, r.reason);
        }
        let invalid = runner
            .report
            .refused
            .get(refused_before)
            .is_some_and(|r| r.reason.contains("invalid body"));
        if applied == Applied::Accepted || invalid {
            all.extend(cmds);
        }
        if invalid || applied == Applied::Found {
            break;
        }
    }
    println!("SCRIPT {seed} {}", serde_json::Value::Array(all));
}

#[test]
fn dump_118385() {
    dump(118_385);
}

#[test]
fn dump_115098() {
    dump(115_098);
}
