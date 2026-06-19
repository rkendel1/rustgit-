use rustgit_wasm_runtime::extract_execution_flow_from_source;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(Path::new("src").join("lib.rs"))?;
    let flow = extract_execution_flow_from_source(&source);

    println!("# Execution Flow Graph");
    println!("entry_points:");
    for entry in flow.entry_points {
        println!("- {entry}");
    }
    println!("transitions:");
    for transition in flow.transitions {
        println!("- {transition}");
    }
    println!("runtime_calls:");
    for call in flow.runtime_calls {
        println!("- {call}");
    }

    Ok(())
}
