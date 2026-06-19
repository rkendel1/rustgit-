use rustgit_wasm_runtime::analyze_architecture_from_source;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(Path::new("src").join("lib.rs"))?;
    let snapshot = analyze_architecture_from_source(&source);

    println!("# Architecture Snapshot");
    println!("modules: {}", snapshot.modules.join(", "));
    println!("traits: {}", snapshot.traits.join(", "));
    println!("structs: {}", snapshot.structs.join(", "));
    println!("enums: {}", snapshot.enums.join(", "));
    println!("call_graph_edges:");
    for (from, to) in snapshot.call_graph.edges {
        println!("- {from} -> {to}");
    }

    Ok(())
}
