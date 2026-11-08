//! The third-party crates of the distributed graphs: no native code is built or shipped,
//! and every crate, build script and use of inline assembly is audited in
//! docs/dependencies.md (`einvoice_testkit::supply_chain`).

use einvoice_testkit::supply_chain::{self, SHIPPED};

#[test]
fn the_shipped_graphs_build_no_native_code_and_are_audited() {
    let (crates, problems) = supply_chain::check().unwrap();
    for c in &crates {
        let graphs: Vec<String> = c
            .graphs
            .iter()
            .map(|&i| {
                let g = SHIPPED[i];
                format!("{} {}", g.package, g.features.join(" "))
                    .trim_end()
                    .to_owned()
            })
            .collect();
        println!(
            "{} {}: build script {}, inline assembly {:?}, unsafe tokens {}, in {}",
            c.name,
            c.version,
            c.build_script.as_deref().unwrap_or("none"),
            c.inline_asm,
            c.unsafe_tokens,
            graphs.join(", ")
        );
    }
    assert_ne!(crates.len(), 0);
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}
