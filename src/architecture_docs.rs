use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArchitectureSnapshot {
    pub modules: Vec<String>,
    pub traits: Vec<String>,
    pub structs: Vec<String>,
    pub enums: Vec<String>,
    pub call_graph: CallGraph,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CallGraph {
    pub edges: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionFlowGraph {
    pub entry_points: Vec<String>,
    pub transitions: Vec<String>,
    pub runtime_calls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeneratedDocs {
    pub system_architecture: String,
    pub execution_flow: String,
    pub runtime_model: String,
}

pub fn analyze_architecture_from_source(source: &str) -> ArchitectureSnapshot {
    let mut modules = BTreeSet::new();
    let mut traits = BTreeSet::new();
    let mut structs = BTreeSet::new();
    let mut enums = BTreeSet::new();
    let mut edges = BTreeSet::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(name) = extract_name_after_keyword(trimmed, "mod ") {
            modules.insert(name);
        }
        if let Some(name) = extract_name_after_keyword(trimmed, "pub mod ") {
            modules.insert(name);
        }
        if let Some(name) = extract_name_after_keyword(trimmed, "trait ") {
            traits.insert(name);
        }
        if let Some(name) = extract_name_after_keyword(trimmed, "pub trait ") {
            traits.insert(name);
        }
        if let Some(name) = extract_name_after_keyword(trimmed, "struct ") {
            structs.insert(name);
        }
        if let Some(name) = extract_name_after_keyword(trimmed, "pub struct ") {
            structs.insert(name);
        }
        if let Some(name) = extract_name_after_keyword(trimmed, "enum ") {
            enums.insert(name);
        }
        if let Some(name) = extract_name_after_keyword(trimmed, "pub enum ") {
            enums.insert(name);
        }
    }

    if modules.is_empty() {
        modules.insert("crate::root".to_string());
    }

    add_call_edge_if_present(
        source,
        &mut edges,
        "WorkspaceManager::launch",
        "analyze_repository",
        "analyze_repository(&repository_root)",
    );
    add_call_edge_if_present(
        source,
        &mut edges,
        "analyze_repository",
        "BuildPlanner::build_graph",
        "BuildPlanner::build_graph(&analysis)",
    );
    add_call_edge_if_present(
        source,
        &mut edges,
        "WorkspaceManager::launch",
        "ExecutionEngine::start",
        "self.execution_engine.start(&mut ctx)",
    );
    add_call_edge_if_present(
        source,
        &mut edges,
        "ExecutionEngine::start",
        "ExecutionProvider::prepare",
        "provider.prepare(ctx)?",
    );
    add_call_edge_if_present(
        source,
        &mut edges,
        "ExecutionEngine::start",
        "ExecutionProvider::start",
        "provider.start(ctx)?",
    );
    add_call_edge_if_present(
        source,
        &mut edges,
        "ExecutionEngine::start",
        "ExecutionProvider::health",
        "provider.health(&handle)?",
    );
    add_call_edge_if_present(
        source,
        &mut edges,
        "ExecutionEngine::prime_artifacts",
        "ArtifactStore::exists",
        "self.artifact_store.exists(key)",
    );

    ArchitectureSnapshot {
        modules: modules.into_iter().collect(),
        traits: traits.into_iter().collect(),
        structs: structs.into_iter().collect(),
        enums: enums.into_iter().collect(),
        call_graph: CallGraph {
            edges: edges.into_iter().collect(),
        },
    }
}

pub fn extract_execution_flow_from_source(source: &str) -> ExecutionFlowGraph {
    let mut entry_points = BTreeSet::new();
    let mut transitions = BTreeSet::new();
    let mut runtime_calls = BTreeSet::new();

    if source.contains("impl WasmWorkspace for WorkspaceManager")
        && source.contains("fn launch(&self, repo_url: &str)")
    {
        entry_points.insert("WorkspaceManager::launch".to_string());
    }
    if source.contains("fn restart(&self, id: &str)") {
        entry_points.insert("WorkspaceManager::restart".to_string());
    }
    if source.contains("fn stop(&self, id: &str)") {
        entry_points.insert("WorkspaceManager::stop".to_string());
    }
    if source.contains("pub fn start(&self, ctx: &mut ExecutionContext)") {
        entry_points.insert("ExecutionEngine::start".to_string());
    }

    add_runtime_call_if_present(
        source,
        &mut runtime_calls,
        "RepositoryRegistry::compute_and_cache_profile",
        "RepositoryAnalysis is produced by repository analyzer",
    );
    add_runtime_call_if_present(
        source,
        &mut runtime_calls,
        "BuildPlanner::build_graph(&analysis)",
        "ExecutionGraph is generated via BuildPlanner",
    );
    add_runtime_call_if_present(
        source,
        &mut runtime_calls,
        "CacheKeyEngine::compute_node_key",
        "CacheKeyEngine computes node keys",
    );
    add_runtime_call_if_present(
        source,
        &mut runtime_calls,
        "self.artifact_store.exists(key)",
        "ArtifactStore is checked for existing outputs",
    );
    add_runtime_call_if_present(
        source,
        &mut runtime_calls,
        "provider.can_handle(ctx)",
        "ExecutionProvider is selected via can_handle()",
    );
    add_runtime_call_if_present(
        source,
        &mut runtime_calls,
        "provider.start(ctx)?",
        "Provider executes node",
    );
    add_runtime_call_if_present(
        source,
        &mut runtime_calls,
        "self.artifact_store.put(ExecutionArtifact",
        "Result is stored in ArtifactStore",
    );

    transitions.extend(extract_workspace_transitions(source));

    ExecutionFlowGraph {
        entry_points: entry_points.into_iter().collect(),
        transitions: transitions.into_iter().collect(),
        runtime_calls: runtime_calls.into_iter().collect(),
    }
}

pub fn generate_grounded_docs(
    snapshot: &ArchitectureSnapshot,
    flow: &ExecutionFlowGraph,
    source: &str,
) -> GeneratedDocs {
    let architecture = format!(
        "# System Architecture (Generated, Code-Grounded)\n\n\
## 1. Actual module structure\n\
- Crate layout: {}\n\
- Module declarations discovered directly from source declarations only.\n\n\
## 2. Real execution components\n\
- ExecutionEngine: {}\n\
- ExecutionProvider: {}\n\
- WorkspaceManager: {}\n\
- ExecutionGraph: {}\n\
- ArtifactStore: {}\n\
- RepositoryRegistry: {}\n\n\
## 3. Real data model inventory\n\
- Structs ({}): {}\n\
- Traits ({}): {}\n\
- Enums ({}): {}\n\n\
## 4. Call graph edges (code-reachable patterns)\n{}\n\n\
## 5. Runtime abstraction truth\n\
- WasmExecutionProvider: {}\n\
- NativeRuntimeEngine: {}\n\
- ExecutionDispatcher: {}\n\
\nAll statements above are derived from declarations or call patterns in `src/lib.rs` only.",
        snapshot.modules.join(", "),
        component_status(source, "struct ExecutionEngine", "impl ExecutionEngine"),
        component_status(
            source,
            "trait ExecutionProvider",
            "impl ExecutionProvider for"
        ),
        component_status(source, "struct WorkspaceManager", "impl WorkspaceManager"),
        component_status(source, "struct ExecutionGraph", "impl ExecutionGraph"),
        component_status(source, "struct ArtifactStore", "impl ArtifactStore"),
        component_status(
            source,
            "struct RepositoryRegistry",
            "impl RepositoryRegistry"
        ),
        snapshot.structs.len(),
        snapshot.structs.join(", "),
        snapshot.traits.len(),
        snapshot.traits.join(", "),
        snapshot.enums.len(),
        snapshot.enums.join(", "),
        markdown_list(
            &snapshot
                .call_graph
                .edges
                .iter()
                .map(|(from, to)| format!("{from} -> {to}"))
                .collect::<Vec<_>>(),
            "No call edges extracted from source patterns.",
        ),
        component_status(
            source,
            "struct WasmExecutionProvider",
            "impl ExecutionProvider for WasmExecutionProvider",
        ),
        component_status(
            source,
            "struct NativeRuntimeEngine",
            "impl NativeRuntimeEngine"
        ),
        component_status(
            source,
            "struct ExecutionDispatcher",
            "impl ExecutionDispatcher"
        ),
    );

    let execution_flow = format!(
        "# Execution Flow (Generated, Code-Grounded)\n\n\
## Entry points\n{}\n\n\
## Runtime behavior (derived from call paths)\n{}\n\n\
## Workspace state machine transitions (actual transitions only)\n{}\n\n\
If a transition or call is not listed above, it was not extracted from current code.",
        markdown_list(
            &flow.entry_points,
            "NOT PRESENT IN CODEBASE: no known runtime entry points extracted.",
        ),
        markdown_list(
            &flow.runtime_calls,
            "NOT PRESENT IN CODEBASE: no runtime call chain extracted.",
        ),
        markdown_list(
            &flow.transitions,
            "NOT PRESENT IN CODEBASE: no WorkspaceState transitions extracted.",
        ),
    );

    let runtime_model = format!(
        "# Runtime Model (Generated, Code-Grounded)\n\n\
## Cache system truth model\n\
- CacheKeyEngine: {}\n\
- ArtifactStore usage in execution path: {}\n\
- Fingerprint integration: {}\n\n\
## Distributed system truth\n\
- WorkerNode: {}\n\
- Scheduler logic: {}\n\
- Coordination/reassignment: {}\n\n\
## Truth labels\n\
- IMPLEMENTED means struct/trait and implementation block are both present.\n\
- PARTIAL means declarations exist but implementation evidence is incomplete.\n\
- NOT PRESENT IN CODEBASE means declaration evidence was not found.",
        component_status(source, "struct CacheKeyEngine", "impl CacheKeyEngine"),
        if source.contains("self.artifact_store.put(ExecutionArtifact")
            && source.contains("self.artifact_store.exists(key)")
        {
            "IMPLEMENTED"
        } else {
            "NOT PRESENT IN CODEBASE"
        },
        if source.contains("analysis.fingerprint") {
            "IMPLEMENTED"
        } else {
            "NOT PRESENT IN CODEBASE"
        },
        component_status(source, "struct WorkerNode", "fn worker_"),
        if source.contains("pub fn schedule(graph: ExecutionGraph, workers: Vec<WorkerNode>)")
            || source.contains("fn schedule_with_artifacts")
        {
            "IMPLEMENTED"
        } else {
            "NOT PRESENT IN CODEBASE"
        },
        if source.contains("reassign_stale_assignments") {
            "IMPLEMENTED"
        } else {
            "NOT PRESENT IN CODEBASE"
        },
    );

    GeneratedDocs {
        system_architecture: architecture,
        execution_flow,
        runtime_model,
    }
}

fn extract_name_after_keyword(line: &str, keyword: &str) -> Option<String> {
    if !line.starts_with(keyword) {
        return None;
    }
    let tail = line[keyword.len()..].trim();
    let mut name = String::new();
    for ch in tail.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
        } else {
            break;
        }
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn add_call_edge_if_present(
    source: &str,
    edges: &mut BTreeSet<(String, String)>,
    from: &str,
    to: &str,
    snippet: &str,
) {
    if source.contains(snippet) {
        edges.insert((from.to_string(), to.to_string()));
    }
}

fn add_runtime_call_if_present(
    source: &str,
    calls: &mut BTreeSet<String>,
    snippet: &str,
    statement: &str,
) {
    if source.contains(snippet) {
        calls.insert(statement.to_string());
    }
}

fn extract_workspace_transitions(source: &str) -> BTreeSet<String> {
    let mut transitions = BTreeSet::new();
    let Some(can_transition_block) = function_block(source, "fn can_transition(") else {
        return transitions;
    };
    let lines: Vec<&str> = can_transition_block.lines().collect();
    let mut line_index = 0usize;
    while line_index < lines.len() {
        let trimmed = lines[line_index].trim();
        if !trimmed.contains("WorkspaceState::") || !trimmed.contains("=>") {
            line_index += 1;
            continue;
        }

        let Some(from_start) = trimmed.find("WorkspaceState::") else {
            line_index += 1;
            continue;
        };
        let from_tail = &trimmed[from_start + "WorkspaceState::".len()..];
        let from = from_tail
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("")
            .to_string();
        if from.is_empty() {
            line_index += 1;
            continue;
        }

        let mut rhs_block = trimmed.split("=>").nth(1).unwrap_or_default().to_string();
        let mut next_line_index = line_index + 1;
        while next_line_index < lines.len() {
            let next = lines[next_line_index].trim();
            if next.starts_with("WorkspaceState::") && next.contains("=>") {
                break;
            }
            rhs_block.push(' ');
            rhs_block.push_str(next);
            next_line_index += 1;
        }

        for part in rhs_block.split("WorkspaceState::").skip(1) {
            let to = part
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("");
            if !to.is_empty() {
                transitions.insert(format!("{from} -> {to}"));
            }
        }

        line_index = next_line_index;
    }

    transitions
}

fn function_block<'a>(source: &'a str, signature: &str) -> Option<&'a str> {
    let start = source.find(signature)?;
    let from_sig = &source[start..];
    let open_offset = from_sig.find('{')?;
    let block_start = start + open_offset;
    let mut depth = 1usize;
    for (idx, ch) in source[block_start + 1..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&source[start..=block_start + 1 + idx]);
                }
            }
            _ => {}
        }
    }
    None
}

fn markdown_list(items: &[String], empty_message: &str) -> String {
    if items.is_empty() {
        return format!("- {empty_message}");
    }
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn component_status(
    source: &str,
    declaration_snippet: &str,
    implementation_snippet: &str,
) -> &'static str {
    let has_declaration = source.contains(declaration_snippet);
    let has_implementation = source.contains(implementation_snippet);
    match (has_declaration, has_implementation) {
        (true, true) => "IMPLEMENTED",
        (true, false) => "PARTIAL (stub or declaration only)",
        (false, _) => "NOT PRESENT IN CODEBASE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = include_str!("lib.rs");

    #[test]
    fn architecture_snapshot_tracks_real_components() {
        let snapshot = analyze_architecture_from_source(SOURCE);
        assert!(snapshot.structs.contains(&"ExecutionEngine".to_string()));
        assert!(snapshot.traits.contains(&"ExecutionProvider".to_string()));
        assert!(snapshot.enums.contains(&"WorkspaceState".to_string()));
        assert!(snapshot.call_graph.edges.contains(&(
            "WorkspaceManager::launch".to_string(),
            "ExecutionEngine::start".to_string()
        )));
    }

    #[test]
    fn execution_flow_extracts_runtime_calls_and_transitions() {
        let flow = extract_execution_flow_from_source(SOURCE);
        assert!(flow
            .runtime_calls
            .contains(&"ExecutionProvider is selected via can_handle()".to_string()));
        assert!(flow
            .transitions
            .contains(&"Created -> Materializing".to_string()));
    }

    #[test]
    fn generated_docs_mark_missing_components_as_not_present() {
        let snapshot = analyze_architecture_from_source(SOURCE);
        let flow = extract_execution_flow_from_source(SOURCE);
        let docs = generate_grounded_docs(&snapshot, &flow, SOURCE);
        assert!(docs
            .system_architecture
            .contains("ExecutionDispatcher: NOT PRESENT IN CODEBASE"));
        assert!(docs
            .execution_flow
            .contains("Workspace state machine transitions"));
        assert!(docs.runtime_model.contains("CacheKeyEngine"));
    }
}
