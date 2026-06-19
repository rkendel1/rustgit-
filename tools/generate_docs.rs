use rustgit_wasm_runtime::{
    analyze_architecture_from_source, extract_execution_flow_from_source, generate_grounded_docs,
};
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(Path::new("src").join("lib.rs"))?;
    let snapshot = analyze_architecture_from_source(&source);
    let flow = extract_execution_flow_from_source(&source);
    let docs = generate_grounded_docs(&snapshot, &flow, &source);

    fs::create_dir_all("docs")?;
    fs::write(
        Path::new("docs").join("system-architecture.generated.md"),
        docs.system_architecture,
    )?;
    fs::write(
        Path::new("docs").join("execution-flow.generated.md"),
        docs.execution_flow,
    )?;
    fs::write(
        Path::new("docs").join("runtime-model.generated.md"),
        docs.runtime_model,
    )?;

    println!("Generated docs/system-architecture.generated.md");
    println!("Generated docs/execution-flow.generated.md");
    println!("Generated docs/runtime-model.generated.md");
    Ok(())
}
