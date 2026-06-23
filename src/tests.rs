use super::*;
use wat::parse_str;

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rustgit_wasm_runtime-{}-{}",
        name,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

fn test_analysis(
    graph: ExecutionGraph,
    compatibility: WasmCompatibility,
    framework: Framework,
) -> RepositoryAnalysis {
    let fingerprint = RepositoryFingerprint {
        repo_id: "repo".to_string(),
        repo_url: "/tmp/repo".to_string(),
        repo_hash: "repo".to_string(),
        lockfile_hash: None,
        dependency_hash: None,
        language_signature: "Unknown".to_string(),
        framework_signature: Some(format!("{framework:?}")),
        ..RepositoryFingerprint::default()
    };
    let classification = RepositoryClassification {
        class: RepoClass::StaticSite,
        confidence: 1.0,
        primary_runtime: RuntimeType::Static,
        secondary_runtimes: vec![],
    };
    let execution_profile = ExecutionProfile {
        fingerprint: fingerprint.clone(),
        classification: classification.clone(),
        recommended_graph_strategy: GraphStrategy::Linear,
        runtime_affinity: RuntimeAffinity {
            preferred_provider: "WasmExecutionProvider".to_string(),
            fallback_providers: vec!["StaticRuntimeProvider".to_string()],
        },
        wasm_compatibility: compatibility,
    };
    let image_match = ExecutionMatchEngine::match_repository(&fingerprint);
    let analysis_seed = RepositoryAnalysis {
        root: PathBuf::from("/tmp/repo"),
        framework,
        language: Language::Unknown,
        execution_spec: None,
        dependency_files: vec![],
        topology: None,
        fingerprint: fingerprint.clone(),
        classification: classification.clone(),
        execution_profile: execution_profile.clone(),
        build_intelligence: BuildIntelligence {
            framework,
            package_manager: None,
            build_tooling: vec![],
            entrypoints: vec![],
            scripts: HashMap::new(),
        },
        execution_graph: graph.clone(),
        execution_image: image_match.image.clone(),
        image_match_confidence: image_match.confidence,
        runtime_spec: ExecutionRuntimeSpec {
            language: "unknown".to_string(),
            framework: "unknown".to_string(),
            package_manager: None,
            dependencies: vec![],
            filesystem: RuntimeFilesystemPlan {
                read_only_layers: vec![],
                dependency_cache_layer: "dependency-cache".to_string(),
                build_cache_layer: "build-cache".to_string(),
                execution_layer: "execution-layer".to_string(),
                temporary_layer: "temporary-layer".to_string(),
                copy_on_write: true,
            },
            network_policy: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
            memory_limit_mb: 0,
            cpu_limit_units: 0,
            cache_layers: vec![],
            environment: BTreeMap::new(),
            ports: vec![],
            services: vec![],
            build_steps: vec![],
            execution_steps: vec![],
            health_checks: vec![],
            recovery_steps: vec![],
            requires_wasm: false,
        },
        compiled_runtime: CompiledWasmExecutionEnvironment {
            environment_id: "runtime-uncompiled".to_string(),
            spec_fingerprint: "unknown".to_string(),
            warm_pool_key: "unknown".to_string(),
            deterministic: true,
            component_graph: vec![],
            wasi_component_graph: WasiComponentGraph::default(),
        },
    };
    let runtime_spec = ExecutionRuntimeSpecCompiler::compile(&analysis_seed);
    let compiled_runtime = WasmRuntimeCompiler::compile(&runtime_spec);
    RepositoryAnalysis {
        root: PathBuf::from("/tmp/repo"),
        framework,
        language: Language::Unknown,
        execution_spec: None,
        dependency_files: vec![],
        topology: None,
        fingerprint,
        classification,
        execution_profile,
        build_intelligence: BuildIntelligence {
            framework,
            package_manager: None,
            build_tooling: vec![],
            entrypoints: vec![],
            scripts: HashMap::new(),
        },
        execution_graph: graph,
        execution_image: image_match.image,
        image_match_confidence: image_match.confidence,
        runtime_spec,
        compiled_runtime,
    }
}

fn test_topology(topology_id: &str) -> ApplicationTopology {
    let service = ServiceDefinition {
        id: "web".to_string(),
        name: "web".to_string(),
        runtime: RuntimeType::Node,
        package_manager: Some("npm".to_string()),
        working_directory: ".".to_string(),
        start_command: "npm run dev".to_string(),
        ports: vec![3000],
        readiness_checks: vec![ReadinessCheck::Port(3000)],
    };
    ApplicationTopology {
        topology_id: topology_id.to_string(),
        services: vec![service.clone()],
        edges: vec![],
        global_network: infer_network_topology(&[service]),
        startup_strategy: StartupStrategy {
            stages: vec![vec!["web".to_string()]],
            enforce_dependencies: true,
        },
        health_policy: infer_health_policy(&[]),
        dependencies: vec![],
        startup_order: StartupOrder {
            stages: vec![vec!["web".to_string()]],
        },
    }
}

#[test]
fn detects_react_framework_from_package_json() {
    let repo = temp_dir("react-detect");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"react":"18.0.0"}}"#,
    )
    .expect("write package.json");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    assert_eq!(analysis.framework, Framework::React);
    assert_eq!(analysis.language, Language::JavaScript);
    assert!(analysis
        .execution_graph
        .nodes
        .iter()
        .any(|node| node.id == "install"));
    assert_eq!(
        analysis.execution_graph.primary_run_command().as_deref(),
        Some("npm run dev -- --host 0.0.0.0 --port {PORT}")
    );
    assert_eq!(
        analysis.build_intelligence.package_manager.as_deref(),
        Some("npm")
    );
}

#[test]
fn detects_nuxt_framework_from_package_json() {
    let repo = temp_dir("nuxt-detect");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"nuxt":"3.0.0"}}"#,
    )
    .expect("write package.json");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    assert_eq!(analysis.framework, Framework::Nuxt);
    assert_eq!(analysis.language, Language::JavaScript);
    assert_eq!(
        analysis.execution_graph.primary_run_command().as_deref(),
        Some("npm run dev -- --host 0.0.0.0 --port {PORT}")
    );
}

#[test]
fn detects_axum_framework_from_cargo_toml() {
    let repo = temp_dir("axum-detect");
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname='axum-app'\nversion='0.1.0'\n[dependencies]\naxum='0.7'\n",
    )
    .expect("write Cargo.toml");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    assert_eq!(analysis.framework, Framework::Axum);
    assert_eq!(analysis.language, Language::Rust);
    assert_eq!(
        analysis.build_intelligence.entrypoints,
        vec!["http://0.0.0.0:8080/"]
    );
    assert_eq!(
        analysis.execution_graph.primary_run_command().as_deref(),
        Some("cargo run")
    );
}

#[test]
fn detects_fastapi_framework_with_uv_package_manager() {
    let repo = temp_dir("fastapi-detect");
    fs::write(repo.join("requirements.txt"), "fastapi==0.115.0\n").expect("write requirements");
    fs::write(
        repo.join("app.py"),
        "from fastapi import FastAPI\napp = FastAPI()\n",
    )
    .expect("write app.py");
    fs::write(repo.join("uv.lock"), "version = 1").expect("write uv lock");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    assert_eq!(analysis.framework, Framework::FastApi);
    assert_eq!(analysis.language, Language::Python);
    assert_eq!(
        analysis.build_intelligence.package_manager.as_deref(),
        Some("uv")
    );
    assert_eq!(
        analysis.execution_graph.primary_run_command().as_deref(),
        Some("uvicorn app:app --host 0.0.0.0 --port {PORT}")
    );
}

#[test]
fn falls_back_to_node_strategy_when_repository_shape_is_unknown() {
    let repo = temp_dir("unknown-repo-fallback");
    fs::write(repo.join("README.md"), "# Hello World\n").expect("write readme");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    assert_eq!(analysis.framework, Framework::Node);
    assert_eq!(analysis.language, Language::JavaScript);
    assert_eq!(
        analysis.execution_graph.primary_run_command().as_deref(),
        Some("npm run dev -- --host 0.0.0.0 --port {PORT}")
    );
    assert!(analysis.topology.is_none());
}

#[test]
fn analyze_repository_detects_multi_service_topology_and_orders_startup() {
    let repo = temp_dir("multi-service-topology");
    fs::create_dir_all(repo.join("apps/web")).expect("create apps/web");
    fs::create_dir_all(repo.join("apps/api")).expect("create apps/api");
    fs::write(repo.join("pnpm-workspace.yaml"), "packages:\n  - apps/*\n")
        .expect("write workspace manifest");
    fs::write(
        repo.join("apps/web/package.json"),
        r#"{"dependencies":{"next":"14.2.0","react":"18.2.0"}}"#,
    )
    .expect("write web package");
    fs::write(
        repo.join("apps/api/package.json"),
        r#"{"dependencies":{"express":"4.0.0"}}"#,
    )
    .expect("write api package");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let topology = analysis.topology.expect("topology should exist");
    assert!(topology.topology_id.starts_with("mstr-"));
    assert_eq!(topology.services.len(), 2);
    assert_eq!(topology.edges, topology.dependencies);
    assert!(topology.dependencies.iter().any(
        |dependency| dependency.service_id == "apps-web" && dependency.depends_on == "apps-api"
    ));
    assert_eq!(
        topology.startup_order.stages,
        vec![vec!["apps-api".to_string()], vec!["apps-web".to_string()]]
    );
    let web = topology
        .services
        .iter()
        .find(|service| service.id == "apps-web")
        .expect("web service");
    assert!(web
        .readiness_checks
        .iter()
        .any(|check| check == &ReadinessCheck::Http("/".to_string())));
    assert_eq!(
        topology.startup_strategy.stages,
        topology.startup_order.stages
    );
    assert!(topology.startup_strategy.enforce_dependencies);
    assert!(topology.global_network.service_dns.contains_key("apps-web"));
    assert!(topology
        .health_policy
        .service_checks
        .contains_key("apps-web"));
    assert!(topology.health_policy.require_healthy_dependencies);
    assert_eq!(analysis.fingerprint.spec_version, "1.0");
    assert_eq!(analysis.fingerprint.services.len(), 2);
    assert!(analysis
        .fingerprint
        .dependency_graph
        .edges
        .iter()
        .any(|edge| edge.from == "apps-web" && edge.to == "apps-api"));
}

#[test]
fn multi_service_topology_upgrades_execution_graph() {
    let repo = temp_dir("multi-service-graph");
    fs::create_dir_all(repo.join("apps/web")).expect("create apps/web");
    fs::create_dir_all(repo.join("apps/api")).expect("create apps/api");
    fs::write(
        repo.join("apps/web/package.json"),
        r#"{"dependencies":{"react":"18.2.0"}}"#,
    )
    .expect("write web package");
    fs::write(repo.join("apps/api/requirements.txt"), "fastapi==0.115.0\n")
        .expect("write api requirements");
    fs::write(
        repo.join("apps/api/app.py"),
        "from fastapi import FastAPI\napp = FastAPI()\n",
    )
    .expect("write api app");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let graph = &analysis.execution_graph;
    assert!(graph.nodes.iter().any(|node| node.id == "shared-build"));
    assert!(graph.nodes.iter().any(|node| node.id == "apps-web-build"));
    assert!(graph.nodes.iter().any(|node| node.id == "apps-api-build"));
    assert!(graph.nodes.iter().any(|node| node.id == "apps-web-run"));
    assert!(graph.nodes.iter().any(|node| node.id == "apps-api-run"));
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.from == "shared-build" && edge.to == "apps-web-build"));
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.from == "shared-build" && edge.to == "apps-api-build"));
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.from == "apps-api-run" && edge.to == "apps-web-run"));
    let web_run = graph
        .nodes
        .iter()
        .find(|node| node.id == "apps-web-run")
        .expect("apps-web-run node");
    assert!(web_run
        .outputs
        .iter()
        .any(|output| output == "svc://apps-web.svc.local"));
}

#[test]
fn ddockit_execution_spec_is_used_as_primary_execution_contract() {
    let repo = temp_dir("ddockit-spec");
    fs::create_dir_all(repo.join(".ddockit")).expect("create .ddockit");
    fs::write(
        repo.join(".ddockit/ddockit.yaml"),
        r#"
version: 1
application:
  name: my-saas
services:
  frontend:
    runtime: node
    framework: nextjs
    install:
      - pnpm install
    build:
      - pnpm build
    run:
      - pnpm start
    port: 3000
    healthcheck:
      type: http
      path: /
  backend:
    runtime: python
    framework: fastapi
    install:
      - uv sync
    run:
      - uvicorn app:app --host 0.0.0.0 --port 8000
    port: 8000
    healthcheck:
      type: http
      path: /docs
dependencies:
  frontend:
    - backend
"#,
    )
    .expect("write ddockit.yaml");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let topology = analysis.topology.expect("topology should exist");
    assert!(analysis.execution_spec.is_some());
    assert_eq!(topology.services.len(), 2);
    assert!(topology.dependencies.iter().any(
        |dependency| dependency.service_id == "frontend" && dependency.depends_on == "backend"
    ));
    assert_eq!(
        topology.startup_order.stages,
        vec![vec!["backend".to_string()], vec!["frontend".to_string()]]
    );
    let frontend = topology
        .services
        .iter()
        .find(|service| service.id == "frontend")
        .expect("frontend");
    assert_eq!(frontend.start_command, "pnpm start");
    assert!(frontend
        .readiness_checks
        .contains(&ReadinessCheck::Http("/".to_string())));
    let graph = analysis.execution_graph;
    assert!(graph.nodes.iter().any(|node| {
        node.id == "frontend-run" && node.command.as_deref() == Some("pnpm start")
    }));
}

#[test]
fn analyze_repository_compiles_uwef_runtime_spec() {
    let repo = temp_dir("uwef-runtime-spec");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"next":"14.2.0"},"scripts":{"build":"next build","dev":"next dev"}}"#,
    )
    .expect("write package.json");
    fs::write(repo.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").expect("write pnpm lock");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    assert_eq!(analysis.runtime_spec.framework, "nextjs");
    assert_eq!(
        analysis.runtime_spec.package_manager.as_deref(),
        Some("pnpm")
    );
    assert!(analysis.runtime_spec.filesystem.copy_on_write);
    assert!(analysis
        .runtime_spec
        .cache_layers
        .contains(&"dependency-cache".to_string()));
    assert!(analysis
        .compiled_runtime
        .environment_id
        .starts_with("uwef-"));
    assert!(analysis
        .compiled_runtime
        .component_graph
        .contains(&"node-runtime".to_string()));
    assert!(analysis
        .compiled_runtime
        .wasi_component_graph
        .components
        .iter()
        .any(|component| component.module == "nodejs.wasm"));
    assert!(analysis
        .compiled_runtime
        .wasi_component_graph
        .capabilities
        .needs
        .contains("pnpm.package_manager"));
    assert!(WasiLinker::validate(
        &analysis.compiled_runtime.wasi_component_graph.capabilities,
        &analysis.compiled_runtime.wasi_component_graph
    ));
    assert!(!analysis
        .compiled_runtime
        .wasi_component_graph
        .links
        .is_empty());
}

#[test]
fn wasi_linker_resolves_imports_and_enforces_constraints() {
    let spec = ExecutionRuntimeSpec {
        language: "rust".to_string(),
        framework: "unknown".to_string(),
        package_manager: Some("cargo".to_string()),
        dependencies: vec!["Cargo.toml".to_string()],
        filesystem: RuntimeFilesystemPlan {
            read_only_layers: vec!["repository-snapshot".to_string()],
            dependency_cache_layer: "dependency-cache".to_string(),
            build_cache_layer: "build-cache".to_string(),
            execution_layer: "execution-layer".to_string(),
            temporary_layer: "temporary-layer".to_string(),
            copy_on_write: true,
        },
        network_policy: NetworkPolicy {
            allow_outbound: true,
            // Intentional duplicate to verify security-model deduplication.
            allowed_hosts: vec!["crates.io".to_string(), "crates.io".to_string()],
        },
        memory_limit_mb: 512,
        cpu_limit_units: 1_000,
        cache_layers: vec![],
        environment: BTreeMap::new(),
        ports: vec![],
        services: vec![],
        build_steps: vec!["cargo build".to_string()],
        execution_steps: vec!["cargo test".to_string()],
        health_checks: vec!["/health".to_string()],
        recovery_steps: vec!["retry-with-warm-pool".to_string()],
        requires_wasm: true,
    };

    let compiled = WasmRuntimeCompiler::compile(&spec);
    let mut graph = compiled.wasi_component_graph;
    graph
        .runtime_constraints
        .read_only_paths
        .push("/workspace".to_string());
    WasiLinker::enforce_security_model(&mut graph);
    assert!(graph
        .components
        .iter()
        .any(|component| component.id == "rust"));
    assert!(graph
        .components
        .iter()
        .any(|component| component.id == "cargo"));
    assert!(graph
        .runtime_constraints
        .network_allowlist
        .contains(&"crates.io".to_string()));
    assert_eq!(
        graph
            .runtime_constraints
            .network_allowlist
            .iter()
            .filter(|host| host.as_str() == "crates.io")
            .count(),
        1
    );
    assert_eq!(
        graph
            .runtime_constraints
            .read_only_paths
            .iter()
            .filter(|path| path.as_str() == "/workspace")
            .count(),
        1
    );
    assert!(WasiLinker::validate(&graph.capabilities, &graph));
    assert!(!graph.links.is_empty());
}

#[test]
fn interface_resolver_handles_version_compatible_interfaces() {
    let resolver = InterfaceResolver;
    let links = resolver.resolve(
        &[String::from("import:nextjs:filesystem.read@v2")],
        &[String::from("export:filesystem:filesystem.read@v1")],
    );

    assert_eq!(links.len(), 1);
    assert_eq!(links[0].from_component, "filesystem");
    assert_eq!(links[0].to_component, "nextjs");
    assert_eq!(links[0].capability, "filesystem.read@v2");
}

#[test]
fn wasi_component_loader_builds_and_caches_linked_graphs() {
    let mut loader = WasiComponentLoader::default();
    let mut capabilities = CapabilitySet::default();
    capabilities.insert("filesystem.read");
    let components = vec![
        WasiComponent {
            id: "filesystem".to_string(),
            module: "filesystem.wasm".to_string(),
            imports: vec![],
            exports: vec!["filesystem.read".to_string()],
            capabilities: vec!["filesystem.read".to_string()],
        },
        WasiComponent {
            id: "consumer".to_string(),
            module: "consumer.wasm".to_string(),
            imports: vec!["filesystem.read".to_string()],
            exports: vec![],
            capabilities: vec![],
        },
    ];
    let runtime_constraints = RuntimeConstraints {
        read_only_paths: vec!["/workspace".to_string()],
        network_allowlist: vec![],
        max_memory_mb: 64,
        max_cpu_units: 1_000,
        process_spawn_bounded: true,
    };

    let first = loader.load_graph(
        components.clone(),
        capabilities.clone(),
        runtime_constraints.clone(),
    );
    let second = loader.load_graph(components, capabilities, runtime_constraints);

    assert_eq!(first.links.len(), 1);
    assert_eq!(first.links[0].from_component, "filesystem");
    assert_eq!(first.links[0].to_component, "consumer");
    assert_eq!(
        first.execution_plan.startup_order,
        vec!["filesystem".to_string(), "consumer".to_string()]
    );
    assert_eq!(first, second);
    assert_eq!(loader.cache.entries.len(), 1);
}

#[test]
fn wasi_optimizer_prunes_dead_components_and_rebuilds_execution_plan() {
    let mut graph = WasiComponentGraph {
        components: vec![
            WasiComponent {
                id: "filesystem".to_string(),
                module: "filesystem.wasm".to_string(),
                imports: vec![],
                exports: vec!["filesystem.read".to_string()],
                capabilities: vec![
                    "filesystem.read".to_string(),
                    "filesystem.write".to_string(),
                ],
            },
            WasiComponent {
                id: "filesystem".to_string(),
                module: "filesystem-v2.wasm".to_string(),
                imports: vec![],
                exports: vec!["filesystem.read".to_string()],
                capabilities: vec!["filesystem.read".to_string()],
            },
            WasiComponent {
                id: "builder".to_string(),
                module: "builder.wasm".to_string(),
                imports: vec!["filesystem.read".to_string()],
                exports: vec![],
                capabilities: vec!["build.compile".to_string()],
            },
            WasiComponent {
                id: "terminal".to_string(),
                module: "terminal.wasm".to_string(),
                imports: vec![],
                exports: vec!["process.spawn".to_string()],
                capabilities: vec!["process.spawn".to_string()],
            },
        ],
        links: vec![WasiLink {
            from_component: "filesystem".to_string(),
            to_component: "builder".to_string(),
            capability: "filesystem.read".to_string(),
        }],
        capabilities: CapabilitySet {
            needs: BTreeSet::from(["filesystem.read".to_string(), "build.compile".to_string()]),
        },
        imports: vec!["import:builder:filesystem.read".to_string()],
        exports: vec![
            "export:filesystem:filesystem.read".to_string(),
            "export:terminal:process.spawn".to_string(),
        ],
        runtime_constraints: RuntimeConstraints::default(),
        execution_plan: ExecutionPlan::default(),
    };

    WasiLinker::optimize_graph(&mut graph);
    WasiLinker::enforce_security_model(&mut graph);

    assert_eq!(
        graph
            .components
            .iter()
            .map(|component| component.id.as_str())
            .collect::<Vec<_>>(),
        vec!["builder", "filesystem"]
    );
    assert!(graph
        .components
        .iter()
        .all(|component| component.id.as_str() != "terminal"));
    let filesystem_component = graph
        .components
        .iter()
        .find(|component| component.id == "filesystem")
        .expect("filesystem component should remain");
    assert_eq!(
        filesystem_component.capabilities,
        vec!["filesystem.read".to_string()]
    );
    assert_eq!(graph.links.len(), 1);
    assert!(graph
        .exports
        .iter()
        .all(|entry| entry.as_str() != "export:terminal:process.spawn"));
    assert_eq!(
        graph.execution_plan.startup_order,
        vec!["filesystem".to_string(), "builder".to_string()]
    );
    assert_eq!(
        graph.execution_plan.ordered_nodes,
        graph.execution_plan.startup_order
    );
    assert!(graph
        .runtime_constraints
        .read_only_paths
        .contains(&"/cache/dependency".to_string()));
    assert!(graph
        .runtime_constraints
        .read_only_paths
        .contains(&"/runtime/warm".to_string()));
}

#[test]
fn js_graph_contains_deterministic_dependencies() {
    let repo = temp_dir("js-graph");
    fs::write(
        repo.join("package.json"),
        r#"{"scripts":{"build":"vite build","dev":"vite"},"dependencies":{"vite":"5.0.0"}}"#,
    )
    .expect("write package.json");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let graph = &analysis.execution_graph;
    let ordered = graph.ordered_node_ids();
    assert_eq!(ordered.first().map(String::as_str), Some("install"));
    assert_eq!(ordered.get(1).map(String::as_str), Some("build"));
    assert!(ordered.contains(&"dev".to_string()));
    assert!(ordered.contains(&"test".to_string()));
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == "install")
            .and_then(|node| node.command.as_deref()),
        Some("npm install")
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == "build")
            .and_then(|node| node.command.as_deref()),
        Some("npm run build")
    );
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.from == "install" && edge.to == "build"));
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.from == "install" && edge.to == "test"));
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.from == "build" && edge.to == "dev"));
    assert!(!graph
        .edges
        .iter()
        .any(|edge| edge.from == "test" && edge.to == "dev"));
    assert_eq!(
        graph.primary_run_command().as_deref(),
        Some("npm run dev -- --host 0.0.0.0 --port {PORT}")
    );
    assert!(graph.nodes.iter().all(|node| node.cache_key.is_some()));
}

#[test]
fn static_web_graph_includes_wasm_compile_binding_step() {
    let repo = temp_dir("static-web-graph");
    fs::write(
        repo.join("index.html"),
        "<!doctype html><title>static</title>",
    )
    .expect("write index.html");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let graph = &analysis.execution_graph;
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.node_type == ExecutionNodeType::WasmCompile));
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.from == "wasm-compile" && edge.to == "serve"));
}

#[test]
fn js_graph_uses_detected_package_manager_commands() {
    let repo = temp_dir("js-pnpm-graph");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"vite":"5.0.0"}}"#,
    )
    .expect("write package.json");
    fs::write(repo.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'").expect("write pnpm lockfile");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let graph = &analysis.execution_graph;
    assert_eq!(
        analysis.build_intelligence.package_manager.as_deref(),
        Some("pnpm")
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == "install")
            .and_then(|node| node.command.as_deref()),
        Some("pnpm install --frozen-lockfile")
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == "build")
            .and_then(|node| node.command.as_deref()),
        Some("pnpm run build")
    );
}

#[test]
fn js_graph_uses_bun_commands_when_bun_lockfile_exists() {
    let repo = temp_dir("js-bun-graph");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"vite":"5.0.0"},"scripts":{"dev":"vite"}}"#,
    )
    .expect("write package.json");
    fs::write(repo.join("bun.lockb"), "bun-lock").expect("write bun lockfile");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let graph = &analysis.execution_graph;
    assert_eq!(
        analysis.build_intelligence.package_manager.as_deref(),
        Some("bun")
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == "install")
            .and_then(|node| node.command.as_deref()),
        Some("bun install --frozen-lockfile")
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == "dev")
            .and_then(|node| node.command.as_deref()),
        Some("bun run dev -- --host 0.0.0.0 --port {PORT}")
    );
}

#[test]
fn vite_framework_uses_vite_default_port() {
    let repo = temp_dir("vite-port");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"vite":"5.0.0"}}"#,
    )
    .expect("write package.json");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    assert_eq!(analysis.framework, Framework::Vite);
    assert_eq!(
        analysis.build_intelligence.entrypoints,
        vec!["http://0.0.0.0:5173/"]
    );
}

#[test]
fn lifecycle_transitions_start_stop_restart() {
    let runtime_root = temp_dir("runtime-root");
    let local_repo = temp_dir("local-repo");
    fs::write(
        local_repo.join("Cargo.toml"),
        "[package]\nname='demo'\nversion='0.1.0'\n",
    )
    .expect("write Cargo.toml");

    let manager = WorkspaceManager::new(&runtime_root);
    let workspace = manager
        .launch(local_repo.to_string_lossy().as_ref())
        .expect("launch workspace");
    assert_eq!(workspace.state, WorkspaceState::Running);

    manager.stop(&workspace.id).expect("stop workspace");
    manager.restart(&workspace.id).expect("restart workspace");

    let logs = manager.logs(&workspace.id).expect("workspace logs");
    assert!(logs.iter().any(|line| line.contains("workspace stopped")));
    assert!(logs.iter().any(|line| line.contains("workspace restarted")));
}

#[test]
fn execution_truth_tracks_exit_and_lifecycle() {
    let mut truth = ExecutionTruth::new("ws-test".to_string(), 3000, 12345);
    truth.update_from_event(ExecutionTruthEvent::ProcessAlive(true));
    truth.update_from_event(ExecutionTruthEvent::ObservedPort(Some(3000)));
    truth.update_from_event(ExecutionTruthEvent::HttpProbeOk(200));
    truth.update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Ready));
    assert_eq!(truth.readiness_state, ExecutionReadinessState::Ready);
    assert_eq!(truth.lifecycle_state, WorkspaceState::Ready);

    truth.update_from_event(ExecutionTruthEvent::ProcessExited(Some(137)));
    assert_eq!(truth.readiness_state, ExecutionReadinessState::Exited);
    assert_eq!(truth.lifecycle_state, WorkspaceState::Failed);
    assert_eq!(truth.exit_code, Some(137));
}

#[test]
fn execution_truth_preserves_stopped_lifecycle_after_process_end() {
    let mut truth = ExecutionTruth::new("ws-test".to_string(), 3000, 12345);
    truth.update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Stopping));
    truth.update_from_event(ExecutionTruthEvent::ProcessAlive(false));
    truth.update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Stopped));
    assert_eq!(truth.readiness_state, ExecutionReadinessState::Exited);
    assert_eq!(truth.lifecycle_state, WorkspaceState::Stopped);
    assert_eq!(truth.process_state, ProcessStatus::Stopped);
}

#[test]
fn launch_overrides_replace_command_version_placeholders() {
    let overrides = LaunchOverrides {
        branch: None,
        start_command: None,
        environment: BTreeMap::new(),
        versions: BTreeMap::from([(String::from("NODE_VERSION"), String::from("20"))]),
    };
    let rendered = WorkspaceManager::apply_command_overrides(
        "nvm use {NODE_VERSION} && npm run dev",
        &overrides,
    );
    assert_eq!(rendered, "nvm use 20 && npm run dev");
}

#[test]
fn reserve_prebound_port_keeps_port_unavailable_until_released() {
    let (port, listener) = WorkspaceManager::reserve_prebound_port_with_preferences(&[])
        .expect("prebound port should allocate");
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
    drop(listener);
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn reserve_prebound_port_with_preferences_uses_preferred_port_when_available() {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe port");
    let preferred_port = probe.local_addr().expect("probe addr").port();
    drop(probe);
    let (port, listener) =
        WorkspaceManager::reserve_prebound_port_with_preferences(&[preferred_port])
            .expect("preferred prebound port should allocate");
    assert_eq!(port, preferred_port);
    drop(listener);
}

#[test]
fn extract_workdir_and_command_parses_cd_prefix_with_chained_command() {
    let default_dir = Path::new("/workspace/repo");
    let (workdir, command) = WorkspaceManager::extract_workdir_and_command(
        "cd /workspace/repo/apps/server && npm run dev -- --host 0.0.0.0",
        default_dir,
    );
    assert_eq!(workdir, PathBuf::from("/workspace/repo/apps/server"));
    assert_eq!(command, "npm run dev -- --host 0.0.0.0");
}

#[test]
fn extract_workdir_and_command_resolves_relative_cd_path() {
    let default_dir = Path::new("/workspace/repo");
    let (workdir, command) =
        WorkspaceManager::extract_workdir_and_command("cd apps/server && npm ci", default_dir);
    assert_eq!(workdir, PathBuf::from("/workspace/repo/apps/server"));
    assert_eq!(command, "npm ci");
}

#[test]
fn extract_workdir_and_command_rejects_workdir_outside_default() {
    let default_dir = temp_dir("extract-workdir-default");
    let outside_dir = temp_dir("extract-workdir-outside");
    let command = format!("cd {} && npm run dev", outside_dir.display());
    let (workdir, extracted) =
        WorkspaceManager::extract_workdir_and_command(&command, default_dir.as_path());
    assert_eq!(workdir, default_dir);
    assert_eq!(extracted, "npm run dev");
}

#[cfg(unix)]
#[test]
fn run_command_with_timeout_kills_hung_process_and_fails_cleanly() {
    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    let result = run_command_with_timeout(&mut cmd, 1);
    assert!(matches!(result, Err(RuntimeError::CommandFailed(msg)) if msg.contains("timed out")));
}

#[test]
fn run_command_with_timeout_returns_output_for_fast_command() {
    let mut cmd = Command::new("echo");
    cmd.arg("hello");
    let output = run_command_with_timeout(&mut cmd, 5).expect("echo should succeed");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
}

#[test]
fn load_execution_manifest_start_command_reads_persisted_manifest() {
    let repo = temp_dir("execution-manifest-start-command");
    fs::write(
        repo.join(".execution.json"),
        r#"{"startCommand":"pnpm run dev -- --host 0.0.0.0 --port {PORT}"}"#,
    )
    .expect("write manifest");
    assert_eq!(
        load_execution_manifest_start_command(repo.as_path()),
        Some("pnpm run dev -- --host 0.0.0.0 --port {PORT}".to_string())
    );
}

#[test]
fn load_execution_manifest_start_command_prefers_runtime_manifest_v2() {
    let repo = temp_dir("runtime-manifest-start-command");
    fs::write(
            repo.join("runtime-manifest.json"),
            r#"{"schemaVersion":2,"runtime":{"startCommand":"pnpm dev","packageManager":"pnpm","installCommand":"pnpm install","nodeVersion":"22"},"project":{"framework":"nextjs","language":"typescript"},"network":{"preferredPorts":[3000],"healthCheck":"/"},"providers":{"compatible":["local"]},"confidence":{"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}}"#,
        )
        .expect("write runtime manifest");
    fs::write(
        repo.join(".execution.json"),
        r#"{"startCommand":"npm run dev"}"#,
    )
    .expect("write legacy manifest");
    assert_eq!(
        load_execution_manifest_start_command(repo.as_path()),
        Some("pnpm dev".to_string())
    );
}

#[test]
fn runtime_manifest_launch_config_exposes_install_ports_health_and_runtime_metadata() {
    let repo = temp_dir("runtime-manifest-launch-config");
    fs::write(
            repo.join("runtime-manifest.json"),
            r#"{"schemaVersion":2,"runtime":{"startCommand":"pnpm dev","packageManager":"pnpm","installCommand":"pnpm install","nodeVersion":"22"},"project":{"framework":"nextjs","language":"typescript"},"network":{"preferredPorts":[3000,3001],"healthCheck":"healthz"},"providers":{"compatible":["local"]},"confidence":{"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}}"#,
        )
        .expect("write runtime manifest");
    assert_eq!(
        load_execution_manifest_install_command(repo.as_path()),
        Some("pnpm install".to_string())
    );
    assert_eq!(
        load_execution_manifest_preferred_ports(repo.as_path()),
        vec![3000, 3001]
    );
    assert_eq!(
        load_execution_manifest_health_check(repo.as_path()),
        "/healthz".to_string()
    );
    assert_eq!(
        load_execution_manifest_node_version(repo.as_path()),
        Some("22".to_string())
    );
    assert_eq!(
        load_execution_manifest_package_manager(repo.as_path()),
        Some("pnpm".to_string())
    );
}

#[test]
fn runtime_repair_candidates_follow_manifest_command_runtime_order() {
    let input = RuntimeRepairInput {
        runtime_manifest: Some(json!({
            "runtime": {
                "startCommand": "npm run dev",
                "packageManager": "pnpm",
                "installCommand": "npm install",
                "nodeVersion": "20"
            },
            "network": {
                "healthCheck": "/ready"
            }
        })),
        execution_artifact: Some(json!({
            "metadata": {
                "launchCommand": "npm run dev"
            }
        })),
        launch_logs: vec!["proxy routing failure".to_string()],
        failure_message: "readiness probe timed out".to_string(),
    };
    let candidates = build_runtime_repair_candidates(
        &input,
        &LaunchOverrides::default(),
        "npm run dev",
        &RepositoryFingerprint::default(),
    );
    let ids = candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "ai-patched-manifest",
            "ai-patched-command",
            "alternative-detected-runtime"
        ]
    );
}

#[test]
fn begin_launch_persists_preheal_overrides_for_retries() {
    let runtime_root = temp_dir("runtime-root-overrides");
    let manager = WorkspaceManager::new(&runtime_root);
    let overrides = LaunchOverrides {
        branch: None,
        start_command: Some("npm run custom".to_string()),
        environment: BTreeMap::from([(String::from("PORT"), String::from("4321"))]),
        versions: BTreeMap::from([(String::from("NODE_VERSION"), String::from("20"))]),
    };
    let id = manager.begin_launch_with_overrides("https://github.com/example/repo.git", overrides);
    let workspaces = manager.workspaces.lock().expect("workspace lock");
    let record = workspaces.get(&id).expect("workspace record");
    assert_eq!(
        record.launch_overrides.start_command.as_deref(),
        Some("npm run custom")
    );
    assert_eq!(
        record
            .launch_overrides
            .environment
            .get("PORT")
            .map(String::as_str),
        Some("4321")
    );
    assert_eq!(
        record
            .launch_overrides
            .versions
            .get("NODE_VERSION")
            .map(String::as_str),
        Some("20")
    );
}

#[test]
fn complete_launch_writes_execution_artifact_for_successful_runs() {
    let runtime_root = temp_dir("runtime-root-execution-artifact-success");
    let local_repo = temp_dir("local-repo-execution-artifact-success");
    fs::write(local_repo.join("index.html"), "<h1>hello</h1>").expect("write index.html");

    let manager = WorkspaceManager::new(&runtime_root);
    let overrides = LaunchOverrides {
        branch: None,
        start_command: Some("python3 -m http.server {PORT}".to_string()),
        environment: BTreeMap::new(),
        versions: BTreeMap::new(),
    };
    let id = manager
        .begin_launch_with_overrides(local_repo.to_string_lossy().as_ref(), overrides.clone());
    manager.complete_launch_with_overrides(&id, local_repo.to_string_lossy().as_ref(), overrides);

    let workspace = manager.get_workspace(&id).expect("workspace");
    let artifact_path = workspace.root.join("execution-artifact.json");
    assert!(
        artifact_path.exists(),
        "execution artifact should be written"
    );
    let payload = fs::read_to_string(&artifact_path).expect("read execution artifact");
    let artifact: serde_json::Value =
        serde_json::from_str(&payload).expect("parse execution artifact");
    assert_eq!(artifact["schemaVersion"].as_u64(), Some(1));
    assert_eq!(artifact["executionId"].as_str(), Some(id.as_str()));
    assert_eq!(artifact["healthStatus"].as_str(), Some("healthy"));
    assert_eq!(
        artifact["metadata"]["launchCommand"].as_str(),
        Some("python3 -m http.server {PORT}")
    );
    let runtime_repair = artifact
        .get("metadata")
        .and_then(|metadata| metadata.get("runtimeRepair"))
        .expect("runtime repair telemetry");
    assert_eq!(
        runtime_repair
            .get("successfulPatch")
            .and_then(Value::as_str),
        Some("original-manifest")
    );
    assert_eq!(
        runtime_repair
            .get("finalRuntimeState")
            .and_then(Value::as_str),
        Some("ready")
    );
    assert!(artifact["startupTimeMs"].as_u64().unwrap_or_default() > 0);
    manager.stop(&id).expect("stop workspace");
}

#[test]
fn complete_launch_writes_execution_artifact_for_failed_runs() {
    let runtime_root = temp_dir("runtime-root-execution-artifact-failure");
    let local_repo = temp_dir("local-repo-execution-artifact-failure");
    fs::write(local_repo.join("index.html"), "<h1>hello</h1>").expect("write index.html");

    let manager = WorkspaceManager::new(&runtime_root);
    let overrides = LaunchOverrides {
        branch: None,
        start_command: Some("command-that-does-not-exist".to_string()),
        environment: BTreeMap::new(),
        versions: BTreeMap::new(),
    };
    let id = manager
        .begin_launch_with_overrides(local_repo.to_string_lossy().as_ref(), overrides.clone());
    manager.complete_launch_with_overrides(&id, local_repo.to_string_lossy().as_ref(), overrides);

    let workspace = manager.get_workspace(&id).expect("workspace");
    assert_eq!(workspace.state, WorkspaceState::Failed);
    let artifact_path = workspace.root.join("execution-artifact.json");
    assert!(
        artifact_path.exists(),
        "execution artifact should be written"
    );
    let payload = fs::read_to_string(&artifact_path).expect("read execution artifact");
    let artifact: serde_json::Value =
        serde_json::from_str(&payload).expect("parse execution artifact");
    assert_eq!(artifact["executionId"].as_str(), Some(id.as_str()));
    assert_eq!(artifact["healthStatus"].as_str(), Some("failed"));
    assert_eq!(artifact["exitCode"].as_i64(), Some(1));
    assert!(artifact["metadata"]["error"]
        .as_str()
        .unwrap_or_default()
        .contains("launch failed"));
    let runtime_repair = artifact
        .get("metadata")
        .and_then(|metadata| metadata.get("runtimeRepair"))
        .expect("runtime repair telemetry");
    assert_eq!(
        runtime_repair
            .get("finalRuntimeState")
            .and_then(Value::as_str),
        Some("failed")
    );
}

#[test]
fn stop_requires_running_or_paused_state() {
    let runtime_root = temp_dir("runtime-root-stop-guard");
    let local_repo = temp_dir("local-repo-stop-guard");
    fs::write(
        local_repo.join("Cargo.toml"),
        "[package]\nname='demo'\nversion='0.1.0'\n",
    )
    .expect("write Cargo.toml");

    let manager = WorkspaceManager::new(&runtime_root);
    let workspace = manager
        .launch(local_repo.to_string_lossy().as_ref())
        .expect("launch workspace");

    manager.stop(&workspace.id).expect("first stop succeeds");
    let err = manager
        .stop(&workspace.id)
        .expect_err("second stop must fail");
    assert!(matches!(
        err,
        RuntimeError::InvalidTransition {
            from: WorkspaceState::Stopped,
            to: WorkspaceState::Stopping,
        },
    ));
}

#[test]
fn state_machine_allows_and_rejects_expected_transitions() {
    assert!(can_transition(
        WorkspaceState::Created,
        WorkspaceState::Materializing
    ));
    assert!(can_transition(
        WorkspaceState::Materializing,
        WorkspaceState::Analyzing
    ));
    assert!(can_transition(
        WorkspaceState::Analyzing,
        WorkspaceState::Planning
    ));
    assert!(can_transition(
        WorkspaceState::Planning,
        WorkspaceState::Starting
    ));
    assert!(can_transition(
        WorkspaceState::Starting,
        WorkspaceState::Running
    ));
    assert!(can_transition(
        WorkspaceState::Paused,
        WorkspaceState::Running
    ));
    assert!(can_transition(
        WorkspaceState::Failed,
        WorkspaceState::Starting
    ));
    assert!(can_transition(
        WorkspaceState::Stopped,
        WorkspaceState::Destroyed
    ));

    assert!(!can_transition(
        WorkspaceState::Created,
        WorkspaceState::Running
    ));
    assert!(!can_transition(
        WorkspaceState::Running,
        WorkspaceState::Created
    ));
    assert!(!can_transition(
        WorkspaceState::Stopped,
        WorkspaceState::Stopping
    ));
    assert!(!can_transition(
        WorkspaceState::Destroyed,
        WorkspaceState::Created
    ));
}

#[test]
fn virtual_filesystem_snapshot_and_restore() {
    let root = temp_dir("vfs");
    let fs = VirtualFileSystem::new(root.clone());
    fs.write("src/main.rs", b"fn main() {}")
        .expect("write source file");

    let snapshot = fs.snapshot().expect("snapshot");
    fs.write("src/main.rs", b"fn main(){println!(\"changed\");}")
        .expect("mutate file");

    fs.restore(&snapshot).expect("restore snapshot");
    let bytes = fs.read("src/main.rs").expect("read restored file");
    assert_eq!(bytes, b"fn main() {}");
}

#[test]
fn cache_key_engine_changes_with_command() {
    let mut graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("cargo build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["Cargo.toml".to_string()],
            outputs: vec!["target".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    };
    let first = graph.compute_cache_keys();
    graph.nodes[0].command = Some("cargo build --release".to_string());
    let second = graph.compute_cache_keys();

    assert_ne!(first.get("build"), second.get("build"));
}

#[test]
fn cache_key_engine_changes_with_repository_fingerprint() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("cargo build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["Cargo.toml".to_string()],
            outputs: vec!["target".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    };
    let first = graph.compute_cache_keys_with_fingerprint(Some(&RepositoryFingerprint {
        repo_id: "repo-a".to_string(),
        repo_url: "repo-a".to_string(),
        repo_hash: "repo-a".to_string(),
        lockfile_hash: None,
        dependency_hash: None,
        language_signature: "Rust".to_string(),
        framework_signature: Some("Rust".to_string()),
        ..RepositoryFingerprint::default()
    }));
    let second = graph.compute_cache_keys_with_fingerprint(Some(&RepositoryFingerprint {
        repo_id: "repo-b".to_string(),
        repo_url: "repo-b".to_string(),
        repo_hash: "repo-b".to_string(),
        lockfile_hash: None,
        dependency_hash: None,
        language_signature: "Rust".to_string(),
        framework_signature: Some("Rust".to_string()),
        ..RepositoryFingerprint::default()
    }));

    assert_ne!(first.get("build"), second.get("build"));
}

#[test]
fn cache_key_engine_changes_with_identity_partition() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("cargo build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["Cargo.toml".to_string()],
            outputs: vec!["target".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    };
    let key_for_user = CacheKeyEngine::compute_node_key_for_identity(
        &graph.nodes[0],
        &graph,
        None,
        Some("user-1"),
    );
    let key_for_anon = CacheKeyEngine::compute_node_key_for_identity(
        &graph.nodes[0],
        &graph,
        None,
        Some("anon-1"),
    );
    assert_ne!(key_for_user, key_for_anon);
}

#[test]
fn repository_registry_classifies_and_tracks_repo_delta() {
    let repo = temp_dir("registry-monorepo");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"next":"14.2.0","react":"18.2.0"}}"#,
    )
    .expect("write package manifest");
    fs::write(repo.join("pnpm-workspace.yaml"), "packages:\n  - apps/*\n")
        .expect("write workspace manifest");
    fs::create_dir_all(repo.join("apps/web")).expect("create apps dir");
    fs::write(repo.join("apps/web/package.json"), r#"{"name":"web"}"#)
        .expect("write nested package manifest");

    let profile = RepositoryRegistry::get_or_compute(repo.to_string_lossy().as_ref());
    assert_eq!(profile.classification.class, RepoClass::Monorepo);
    assert_eq!(
        profile.recommended_graph_strategy,
        GraphStrategy::MonorepoSegmented
    );

    fs::write(
        repo.join("apps/web/package.json"),
        r#"{"name":"web","private":true}"#,
    )
    .expect("modify nested package manifest");
    let _ = RepositoryRegistry::get_or_compute(repo.to_string_lossy().as_ref());
    let delta = RepositoryRegistry::latest_delta(repo.to_string_lossy().as_ref())
        .expect("delta should be available");
    assert!(delta
        .modified_files
        .iter()
        .any(|path| path == "apps/web/package.json"));
}

#[test]
fn analyze_repository_emits_execution_profile() {
    let repo = temp_dir("analysis-profile");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"react":"18.0.0"}}"#,
    )
    .expect("write package.json");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    assert!(!analysis.fingerprint.repo_hash.is_empty());
    assert_eq!(analysis.classification.class, RepoClass::NodeApp);
    assert_eq!(
        analysis
            .execution_profile
            .runtime_affinity
            .preferred_provider,
        "NodeRuntimeProvider"
    );
    assert_eq!(
        analysis.execution_profile.wasm_compatibility,
        WasmCompatibility::Partial
    );
    assert!(!analysis.fingerprint.repo_id.is_empty());
    assert_eq!(analysis.fingerprint.spec_version, "1.0");
    assert!(analysis.fingerprint.runtime_signals.node_detected);
    assert!(analysis
        .fingerprint
        .entrypoints
        .iter()
        .any(|entry| entry.path == "package.json"));
    assert!(!analysis.fingerprint.build_signals.has_lockfile);
}

#[test]
fn artifact_store_round_trips_execution_artifact() {
    let root = temp_dir("artifact-store");
    let store = ArtifactStore::new(root.clone());
    let artifact = ExecutionArtifact {
        key: "cache-key".to_string(),
        node_id: "build".to_string(),
        artifact_type: ArtifactType::BuildOutput,
        path: root.join("build-output").to_string_lossy().to_string(),
        created_at: 42,
    };

    store.put(artifact.clone());

    assert!(store.exists("cache-key"));
    assert_eq!(store.get("cache-key"), Some(artifact));
}

#[test]
fn artifact_store_registers_wasm_artifact_and_binding() {
    let root = temp_dir("artifact-store-wasm");
    let store = ArtifactStore::new(root);

    store.register_wasm_artifact(WasmArtifact {
        node_id: "serve".to_string(),
        module_path: "/repo/pkg/app_bg.wasm".to_string(),
        hash: "abc123".to_string(),
    });
    store.register_wasm_artifact_binding(WasmArtifactBinding {
        node_id: "serve".to_string(),
        artifact_key: "cache-key".to_string(),
        build_fingerprint: "repo-fp".to_string(),
        source_files_hash: "src-fp".to_string(),
    });

    assert_eq!(
        store.get_wasm_artifact("serve"),
        Some(WasmArtifact {
            node_id: "serve".to_string(),
            module_path: "/repo/pkg/app_bg.wasm".to_string(),
            hash: "abc123".to_string(),
        })
    );
    assert_eq!(
        store.get_wasm_artifact_binding("serve"),
        Some(WasmArtifactBinding {
            node_id: "serve".to_string(),
            artifact_key: "cache-key".to_string(),
            build_fingerprint: "repo-fp".to_string(),
            source_files_hash: "src-fp".to_string(),
        })
    );
}

#[test]
fn distributed_scheduler_respects_dependency_capability_and_label_constraints() {
    let graph = ExecutionGraph {
        nodes: vec![
            ExecutionNode {
                id: "build".to_string(),
                node_type: ExecutionNodeType::Build,
                command: Some("cargo build".to_string()),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["Cargo.toml".to_string()],
                outputs: vec!["target".to_string()],
                cache_key: None,
                runtime: None,
                cache_binding: None,
            },
            ExecutionNode {
                id: "wasm-test".to_string(),
                node_type: ExecutionNodeType::Test,
                command: Some("wasm-test-runner".to_string()),
                execution_mode: ExecutionMode::Wasm,
                inputs: vec!["target".to_string()],
                outputs: vec!["report".to_string()],
                cache_key: None,
                runtime: None,
                cache_binding: None,
            },
        ],
        edges: vec![ExecutionEdge {
            from: "build".to_string(),
            to: "wasm-test".to_string(),
        }],
    };

    let workers = vec![
        WorkerNode {
            id: "native-a".to_string(),
            capabilities: WorkerCapabilities {
                wasm: false,
                native: true,
                cpu_cores: 8,
                memory_mb: 8192,
                labels: vec!["high-cpu".to_string()],
            },
            status: WorkerStatus::Ready,
        },
        WorkerNode {
            id: "wasm-b".to_string(),
            capabilities: WorkerCapabilities {
                wasm: true,
                native: false,
                cpu_cores: 4,
                memory_mb: 4096,
                labels: vec!["wasm".to_string()],
            },
            status: WorkerStatus::Ready,
        },
    ];

    let artifact_store = DistributedArtifactStore::default();
    artifact_store.store(ExecutionArtifact {
        key: "build-out".to_string(),
        node_id: "build".to_string(),
        artifact_type: ArtifactType::BuildOutput,
        path: "/tmp/build".to_string(),
        created_at: 1,
    });

    let mut config = DistributedExecutionConfig::default();
    config
        .required_worker_labels
        .insert("wasm-test".to_string(), vec!["wasm".to_string()]);
    config
        .required_artifacts
        .insert("wasm-test".to_string(), vec!["build-out".to_string()]);
    config.lease_ttl_secs = 30;

    let plan =
        DistributedScheduler.schedule_with_context(graph, workers, &artifact_store, &config, 100);

    assert_eq!(plan.ordered_nodes, vec!["build", "wasm-test"]);
    assert!(plan.unscheduled_nodes.is_empty());
    assert_eq!(plan.assignments.len(), 2);
    assert_eq!(plan.assignments[0].node_id, "build");
    assert_eq!(plan.assignments[0].worker_id, "native-a");
    assert_eq!(plan.assignments[1].node_id, "wasm-test");
    assert_eq!(plan.assignments[1].worker_id, "wasm-b");
    assert_eq!(
        plan.leases.get("wasm-test").map(|lease| lease.expires_at),
        Some(130)
    );
}

#[test]
fn distributed_scheduler_blocks_nodes_when_required_artifacts_are_missing() {
    let graph = ExecutionGraph {
        nodes: vec![
            ExecutionNode {
                id: "build".to_string(),
                node_type: ExecutionNodeType::Build,
                command: Some("cargo build".to_string()),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["Cargo.toml".to_string()],
                outputs: vec!["target".to_string()],
                cache_key: None,
                runtime: None,
                cache_binding: None,
            },
            ExecutionNode {
                id: "test".to_string(),
                node_type: ExecutionNodeType::Test,
                command: Some("cargo test".to_string()),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["target".to_string()],
                outputs: vec!["report".to_string()],
                cache_key: None,
                runtime: None,
                cache_binding: None,
            },
        ],
        edges: vec![ExecutionEdge {
            from: "build".to_string(),
            to: "test".to_string(),
        }],
    };
    let workers = vec![WorkerNode {
        id: "native-a".to_string(),
        capabilities: WorkerCapabilities {
            wasm: false,
            native: true,
            cpu_cores: 8,
            memory_mb: 8192,
            labels: vec![],
        },
        status: WorkerStatus::Ready,
    }];
    let artifact_store = DistributedArtifactStore::default();
    let mut config = DistributedExecutionConfig::default();
    config
        .required_artifacts
        .insert("test".to_string(), vec!["missing-build-out".to_string()]);

    let plan =
        DistributedScheduler.schedule_with_context(graph, workers, &artifact_store, &config, 10);

    assert_eq!(plan.assignments.len(), 1);
    assert_eq!(plan.assignments[0].node_id, "build");
    assert_eq!(plan.unscheduled_nodes, vec!["test"]);
}

#[test]
fn distributed_execution_scheduler_prioritizes_interactive_jobs_and_logs_decisions() {
    let mut scheduler = DistributedExecutionScheduler::default();
    scheduler.register_runtime_node(RuntimeNode {
        node_id: "dea-east".to_string(),
        runtime_type: RuntimeNodeType::Dea,
        capacity_cpu: 8,
        capacity_memory: 16_384,
        current_load: 0,
        health_status: RuntimeNodeHealth::Healthy,
        region: "us-east".to_string(),
        cost_per_second: 0.01,
        latency_ms: 20,
        max_concurrent_executions: 4,
        active_jobs: vec![],
        last_heartbeat: 100,
        success_rate: 0.99,
        warm_pool_ready: true,
    });
    scheduler.register_runtime_node(RuntimeNode {
        node_id: "cloud-west".to_string(),
        runtime_type: RuntimeNodeType::Cloud,
        capacity_cpu: 16,
        capacity_memory: 32_768,
        current_load: 0,
        health_status: RuntimeNodeHealth::Healthy,
        region: "us-west".to_string(),
        cost_per_second: 0.03,
        latency_ms: 40,
        max_concurrent_executions: 8,
        active_jobs: vec![],
        last_heartbeat: 100,
        success_rate: 0.95,
        warm_pool_ready: false,
    });

    scheduler.enqueue(QueuedExecution {
        execution_id: "exec-batch".to_string(),
        org_id: "org-1".to_string(),
        priority: ExecutionPriority::Batch,
        status: ExecutionQueueStatus::Queued,
        submitted_at: 10,
        preferred_region: Some("us-east".to_string()),
    });
    scheduler.enqueue(QueuedExecution {
        execution_id: "exec-interactive".to_string(),
        org_id: "org-1".to_string(),
        priority: ExecutionPriority::Interactive,
        status: ExecutionQueueStatus::Queued,
        submitted_at: 11,
        preferred_region: Some("us-east".to_string()),
    });

    let decision = scheduler
        .schedule_next(20)
        .expect("interactive execution should be scheduled");
    assert_eq!(decision.execution_id, "exec-interactive");
    assert_eq!(decision.selected_node.as_deref(), Some("dea-east"));
    assert_eq!(scheduler.queue_length(), 1);
    assert_eq!(scheduler.scheduler_events.len(), 1);
    assert!(scheduler
        .scheduler_events
        .iter()
        .any(|event| event.reason.contains("routing policy score")));
}

#[test]
fn distributed_execution_scheduler_applies_backpressure_to_batch_jobs() {
    let mut scheduler = DistributedExecutionScheduler {
        backpressure_threshold: 0,
        ..DistributedExecutionScheduler::default()
    };
    scheduler.register_runtime_node(RuntimeNode {
        node_id: "dea-east".to_string(),
        runtime_type: RuntimeNodeType::Dea,
        capacity_cpu: 8,
        capacity_memory: 16_384,
        current_load: 0,
        health_status: RuntimeNodeHealth::Healthy,
        region: "us-east".to_string(),
        cost_per_second: 0.01,
        latency_ms: 20,
        max_concurrent_executions: 1,
        active_jobs: vec![],
        last_heartbeat: 100,
        success_rate: 0.99,
        warm_pool_ready: false,
    });
    scheduler.enqueue(QueuedExecution {
        execution_id: "exec-batch".to_string(),
        org_id: "org-1".to_string(),
        priority: ExecutionPriority::Batch,
        status: ExecutionQueueStatus::Queued,
        submitted_at: 5,
        preferred_region: None,
    });

    let decision = scheduler
        .schedule_next(10)
        .expect("scheduler should emit backpressure decision");
    assert_eq!(decision.execution_id, "exec-batch");
    assert!(decision.selected_node.is_none());
    assert!(decision.reason.contains("backpressure"));
    assert_eq!(scheduler.queue_length(), 1);
    assert_eq!(
        scheduler
            .queue
            .executions
            .front()
            .map(|execution| execution.status),
        Some(ExecutionQueueStatus::Blocked)
    );
}

#[test]
fn distributed_execution_scheduler_requeues_jobs_after_heartbeat_timeout() {
    let mut scheduler = DistributedExecutionScheduler::default();
    scheduler.register_runtime_node(RuntimeNode {
        node_id: "dea-a".to_string(),
        runtime_type: RuntimeNodeType::Dea,
        capacity_cpu: 8,
        capacity_memory: 16_384,
        current_load: 0,
        health_status: RuntimeNodeHealth::Healthy,
        region: "us-east".to_string(),
        cost_per_second: 0.01,
        latency_ms: 15,
        max_concurrent_executions: 1,
        active_jobs: vec![],
        last_heartbeat: 100,
        success_rate: 0.99,
        warm_pool_ready: true,
    });
    scheduler.register_runtime_node(RuntimeNode {
        node_id: "dea-b".to_string(),
        runtime_type: RuntimeNodeType::Dea,
        capacity_cpu: 8,
        capacity_memory: 16_384,
        current_load: 0,
        health_status: RuntimeNodeHealth::Healthy,
        region: "us-east".to_string(),
        cost_per_second: 0.02,
        latency_ms: 20,
        max_concurrent_executions: 1,
        active_jobs: vec![],
        last_heartbeat: 115,
        success_rate: 0.98,
        warm_pool_ready: false,
    });
    scheduler.enqueue(QueuedExecution {
        execution_id: "exec-1".to_string(),
        org_id: "org-1".to_string(),
        priority: ExecutionPriority::Interactive,
        status: ExecutionQueueStatus::Queued,
        submitted_at: 101,
        preferred_region: Some("us-east".to_string()),
    });

    let first = scheduler
        .schedule_next(105)
        .expect("first assignment should succeed");
    assert_eq!(first.selected_node.as_deref(), Some("dea-a"));
    assert_eq!(scheduler.queue_length(), 0);

    let recovered = scheduler.recover_failed_executions(120, 10);
    assert_eq!(recovered, vec!["exec-1".to_string()]);
    assert_eq!(scheduler.queue_length(), 1);
    assert_eq!(
        scheduler
            .registry
            .nodes
            .get("dea-a")
            .map(|node| node.health_status),
        Some(RuntimeNodeHealth::Unhealthy)
    );

    let second = scheduler
        .schedule_next(121)
        .expect("execution should be reassigned to healthy worker");
    assert_eq!(second.execution_id, "exec-1");
    assert_eq!(second.selected_node.as_deref(), Some("dea-b"));
}

#[test]
fn distributed_execution_scheduler_emits_runtime_scale_signal_only_when_saturated() {
    let mut scheduler = DistributedExecutionScheduler::default();
    scheduler.enqueue(QueuedExecution {
        execution_id: "exec-scale".to_string(),
        org_id: "org-1".to_string(),
        priority: ExecutionPriority::Batch,
        status: ExecutionQueueStatus::Queued,
        submitted_at: 1,
        preferred_region: None,
    });

    assert!(!scheduler.should_scale_runtime(RuntimeNodeType::Cloud));

    scheduler.register_runtime_node(RuntimeNode {
        node_id: "cloud-a".to_string(),
        runtime_type: RuntimeNodeType::Cloud,
        capacity_cpu: 8,
        capacity_memory: 16_384,
        current_load: 1,
        health_status: RuntimeNodeHealth::Healthy,
        region: "us-east".to_string(),
        cost_per_second: 0.05,
        latency_ms: 30,
        max_concurrent_executions: 1,
        active_jobs: vec!["active-1".to_string()],
        last_heartbeat: 1,
        success_rate: 0.95,
        warm_pool_ready: false,
    });
    assert!(scheduler.should_scale_runtime(RuntimeNodeType::Cloud));

    scheduler.register_runtime_node(RuntimeNode {
        node_id: "cloud-b".to_string(),
        runtime_type: RuntimeNodeType::Cloud,
        capacity_cpu: 8,
        capacity_memory: 16_384,
        current_load: 0,
        health_status: RuntimeNodeHealth::Healthy,
        region: "us-west".to_string(),
        cost_per_second: 0.05,
        latency_ms: 35,
        max_concurrent_executions: 2,
        active_jobs: vec![],
        last_heartbeat: 1,
        success_rate: 0.95,
        warm_pool_ready: false,
    });
    assert!(!scheduler.should_scale_runtime(RuntimeNodeType::Cloud));
}

#[test]
fn worker_registry_tracks_heartbeats_and_detects_stale_workers() {
    let worker = WorkerNode {
        id: "worker-a".to_string(),
        capabilities: WorkerCapabilities {
            wasm: true,
            native: true,
            cpu_cores: 4,
            memory_mb: 4096,
            labels: vec![],
        },
        status: WorkerStatus::Ready,
    };
    let mut registry = WorkerRegistry::from_workers(vec![worker], 5, 100);

    assert!(registry.detect_failed_workers(104).is_empty());
    assert_eq!(registry.detect_failed_workers(106), vec!["worker-a"]);
    assert_eq!(
        registry
            .workers
            .get("worker-a")
            .map(|worker| worker.status.clone()),
        Some(WorkerStatus::Offline)
    );

    assert!(registry.record_heartbeat("worker-a", 107));
    assert_eq!(
        registry
            .workers
            .get("worker-a")
            .map(|worker| worker.status.clone()),
        Some(WorkerStatus::Ready)
    );
}

#[test]
fn execution_plan_reassigns_expired_leases_to_active_workers() {
    let mut plan = ExecutionPlan {
        assignments: vec![NodeAssignment {
            node_id: "node-a".to_string(),
            worker_id: "worker-a".to_string(),
            sequence: 0,
        }],
        leases: HashMap::from([(
            "node-a".to_string(),
            NodeLease {
                node_id: "node-a".to_string(),
                worker_id: "worker-a".to_string(),
                expires_at: 10,
            },
        )]),
        worker_queues: HashMap::from([
            (
                "worker-a".to_string(),
                WorkerQueue {
                    queued_nodes: vec!["node-a".to_string()],
                },
            ),
            ("worker-b".to_string(), WorkerQueue::default()),
        ]),
        ..ExecutionPlan::default()
    };

    let workers = vec![
        WorkerNode {
            id: "worker-a".to_string(),
            capabilities: WorkerCapabilities {
                wasm: true,
                native: true,
                cpu_cores: 4,
                memory_mb: 4096,
                labels: vec![],
            },
            status: WorkerStatus::Ready,
        },
        WorkerNode {
            id: "worker-b".to_string(),
            capabilities: WorkerCapabilities {
                wasm: true,
                native: true,
                cpu_cores: 4,
                memory_mb: 4096,
                labels: vec![],
            },
            status: WorkerStatus::Ready,
        },
    ];

    let reassigned = plan.reassign_stale_assignments(&workers, 30, 10);
    assert_eq!(reassigned, vec!["node-a"]);
    assert_eq!(
        plan.leases
            .get("node-a")
            .map(|lease| lease.worker_id.as_str()),
        Some("worker-b")
    );
    assert_eq!(plan.assignments[0].worker_id, "worker-b");
    assert!(plan
        .worker_queues
        .get("worker-a")
        .is_some_and(|queue| queue.queued_nodes.is_empty()));
    assert_eq!(
        plan.worker_queues
            .get("worker-b")
            .map(|queue| queue.queued_nodes.clone()),
        Some(vec!["node-a".to_string()])
    );

    assert!(plan.reassign_stale_assignments(&workers, 30, 39).is_empty());
}

#[test]
fn execution_coordinator_reassigns_work_when_worker_goes_offline() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "wasm-build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("wasm-pack build".to_string()),
            execution_mode: ExecutionMode::Wasm,
            inputs: vec!["src".to_string()],
            outputs: vec!["pkg".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    };
    let workers = vec![
        WorkerNode {
            id: "worker-a".to_string(),
            capabilities: WorkerCapabilities {
                wasm: true,
                native: false,
                cpu_cores: 2,
                memory_mb: 2048,
                labels: vec!["wasm".to_string()],
            },
            status: WorkerStatus::Ready,
        },
        WorkerNode {
            id: "worker-b".to_string(),
            capabilities: WorkerCapabilities {
                wasm: true,
                native: false,
                cpu_cores: 2,
                memory_mb: 2048,
                labels: vec!["wasm".to_string()],
            },
            status: WorkerStatus::Ready,
        },
    ];
    let mut coordinator = ExecutionCoordinator::new(workers, DistributedArtifactStore::default());
    let config = DistributedExecutionConfig::default();

    let initial = coordinator.plan(graph.clone(), &config, 50);
    assert_eq!(initial.assignments[0].worker_id, "worker-a");

    let recovered = coordinator.recover_failed_worker(graph, "worker-a", &config, 55);
    assert_eq!(recovered.assignments[0].worker_id, "worker-b");
}

#[test]
fn execution_coordinator_detects_worker_failure_and_reassigns_current_lease() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "wasm-build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("wasm-pack build".to_string()),
            execution_mode: ExecutionMode::Wasm,
            inputs: vec!["src".to_string()],
            outputs: vec!["pkg".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    };
    let workers = vec![
        WorkerNode {
            id: "worker-a".to_string(),
            capabilities: WorkerCapabilities {
                wasm: true,
                native: false,
                cpu_cores: 2,
                memory_mb: 2048,
                labels: vec!["wasm".to_string()],
            },
            status: WorkerStatus::Ready,
        },
        WorkerNode {
            id: "worker-b".to_string(),
            capabilities: WorkerCapabilities {
                wasm: true,
                native: false,
                cpu_cores: 2,
                memory_mb: 2048,
                labels: vec!["wasm".to_string()],
            },
            status: WorkerStatus::Ready,
        },
    ];
    let mut coordinator = ExecutionCoordinator::new(workers, DistributedArtifactStore::default());
    coordinator.worker_registry.heartbeat_timeout_secs = 5;
    assert!(coordinator.heartbeat("worker-a", 100));
    assert!(coordinator.heartbeat("worker-b", 100));

    let mut config = DistributedExecutionConfig::default();
    config.lease_ttl_secs = 5;
    let mut plan = coordinator.plan(graph, &config, 100);

    assert!(coordinator.heartbeat("worker-b", 104));
    assert!(coordinator.detect_failed_workers(105).is_empty());
    assert_eq!(coordinator.detect_failed_workers(106), vec!["worker-a"]);

    let reassigned = plan.reassign_failed_worker(
        "worker-a",
        &coordinator.worker_registry.snapshot_workers(),
        config.lease_ttl_secs,
        106,
    );
    assert_eq!(reassigned, vec!["wasm-build"]);
    assert_eq!(
        plan.leases
            .get("wasm-build")
            .map(|lease| lease.worker_id.as_str()),
        Some("worker-b")
    );
}

#[test]
fn static_site_routes_to_wasm_target() {
    let profile = ExecutionProfile {
        fingerprint: RepositoryFingerprint {
            repo_id: "repo".to_string(),
            repo_url: "repo".to_string(),
            repo_hash: "repo".to_string(),
            lockfile_hash: None,
            dependency_hash: None,
            language_signature: "Unknown".to_string(),
            framework_signature: Some("StaticWeb".to_string()),
            ..RepositoryFingerprint::default()
        },
        classification: RepositoryClassification {
            class: RepoClass::StaticSite,
            confidence: 0.9,
            primary_runtime: RuntimeType::Static,
            secondary_runtimes: vec![],
        },
        recommended_graph_strategy: GraphStrategy::Linear,
        runtime_affinity: RuntimeAffinity {
            preferred_provider: "WasmExecutionProvider".to_string(),
            fallback_providers: vec!["StaticRuntimeProvider".to_string()],
        },
        wasm_compatibility: WasmCompatibility::Full,
    };
    let node = ExecutionNode {
        id: "serve".to_string(),
        node_type: ExecutionNodeType::StaticServe,
        command: Some("serve .".to_string()),
        execution_mode: ExecutionMode::Wasm,
        inputs: vec![],
        outputs: vec![],
        cache_key: None,
        runtime: None,
        cache_binding: None,
    };

    match ExecutionRouter::route(&node, &profile) {
        ExecutionTarget::Wasm(spec) => {
            assert!(spec.enabled);
            assert!(spec.wasi);
        }
        other => panic!("expected wasm routing target, got {other:?}"),
    }
}

#[test]
fn execution_router_selects_preferred_runtime_provider() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "serve".to_string(),
            node_type: ExecutionNodeType::StaticServe,
            command: Some("serve .".to_string()),
            execution_mode: ExecutionMode::Wasm,
            inputs: vec![],
            outputs: vec![],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let analysis = test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb);
    let ctx = ExecutionContext {
        workspace_id: "ws-router-preferred".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec: analysis.runtime_spec.clone(),
        compiled_runtime: analysis.compiled_runtime.clone(),
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };
    let router = ExecutionRouter::new(vec![
        Box::new(WasmExecutionProvider),
        Box::new(NodeRuntimeProvider),
        Box::new(RustRuntimeProvider),
        Box::new(StaticRuntimeProvider),
    ]);

    let selection = router.select(&ctx).expect("select preferred provider");
    assert_eq!(selection.provider_id, "WasmExecutionProvider");
    assert_eq!(selection.runtime, RuntimeType::Wasm);
    assert_eq!(selection.selected_tier, ExecutionTier::LocalMachine);
    assert_eq!(
        selection.trace_uri,
        "ddockit://workspace/ws-router-preferred/trace"
    );
    assert_eq!(
        selection.trace_url,
        "https://trythissoftware.com/e/ws-router-preferred"
    );
}

#[test]
fn execution_engine_uses_router_fallback_provider() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("cargo build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["Cargo.toml".to_string()],
            outputs: vec!["target".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let mut analysis = test_analysis(graph.clone(), WasmCompatibility::Partial, Framework::Rust);
    analysis.execution_profile.runtime_affinity = RuntimeAffinity {
        preferred_provider: "NodeRuntimeProvider".to_string(),
        fallback_providers: vec!["RustRuntimeProvider".to_string()],
    };

    let runtime_spec = analysis.runtime_spec.clone();
    let compiled_runtime = analysis.compiled_runtime.clone();
    let mut ctx = ExecutionContext {
        workspace_id: "ws-router-fallback".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec,
        compiled_runtime,
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };

    let engine = ExecutionEngine::new(
        vec![
            Box::new(WasmExecutionProvider),
            Box::new(DockerExecutionProvider),
            Box::new(NodeRuntimeProvider),
            Box::new(RustRuntimeProvider),
            Box::new(StaticRuntimeProvider),
        ],
        ArtifactStore::new(temp_dir("router-engine-artifacts")),
    );

    let handle = engine.start(&mut ctx).expect("engine should use fallback");
    assert!(handle.pid_hint.starts_with("rust:"));
    assert_eq!(
        handle.trace_uri.as_deref(),
        Some("ddockit://workspace/ws-router-fallback/trace")
    );
    assert_eq!(
        handle.trace_url.as_deref(),
        Some("https://trythissoftware.com/e/ws-router-fallback")
    );
}

#[test]
fn runtime_trace_provider_inference_maps_known_pid_prefixes() {
    assert_eq!(
        infer_provider_from_pid_hint("wasm:serve:cache"),
        "WasmExecutionProvider"
    );
    assert_eq!(
        infer_provider_from_pid_hint("dea:agent:workspace"),
        "LocalAgentProvider"
    );
    assert_eq!(
        infer_provider_from_pid_hint("node:cache"),
        "NodeRuntimeProvider"
    );
    assert_eq!(
        infer_provider_from_pid_hint("go:cache"),
        "GoExecutionProvider"
    );
    assert_eq!(
        infer_provider_from_pid_hint("python:cache"),
        "PythonExecutionProvider"
    );
    assert_eq!(
        infer_provider_from_pid_hint("java:cache"),
        "JavaExecutionProvider"
    );
    assert_eq!(
        infer_provider_from_pid_hint("docker:cache"),
        "DockerExecutionProvider"
    );
    assert_eq!(
        infer_provider_from_pid_hint("rust:cache"),
        "RustRuntimeProvider"
    );
    assert_eq!(
        infer_provider_from_pid_hint("static:cache"),
        "StaticRuntimeProvider"
    );
}

#[test]
fn execution_router_escalates_through_tiers_and_records_trace() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("cargo build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["Cargo.toml".to_string()],
            outputs: vec!["target".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let mut analysis = test_analysis(graph.clone(), WasmCompatibility::Partial, Framework::Rust);
    analysis.execution_profile.runtime_affinity = RuntimeAffinity {
        preferred_provider: "NodeRuntimeProvider".to_string(),
        fallback_providers: vec!["RustRuntimeProvider".to_string()],
    };
    let runtime_spec = analysis.runtime_spec.clone();
    let compiled_runtime = analysis.compiled_runtime.clone();
    let ctx = ExecutionContext {
        workspace_id: "ws-escalation-trace".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec,
        compiled_runtime,
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };
    let router = ExecutionRouter::new(vec![
        Box::new(WasmExecutionProvider),
        Box::new(DockerExecutionProvider),
        Box::new(NodeRuntimeProvider),
        Box::new(RustRuntimeProvider),
        Box::new(StaticRuntimeProvider),
    ]);

    let selection = router.select(&ctx).expect("select escalated provider");
    assert_eq!(selection.provider_id, "RustRuntimeProvider");
    assert_eq!(selection.selected_tier, ExecutionTier::ExternalProvider);
    assert!(selection
        .escalation_trace
        .iter()
        .any(|step| step.tier == ExecutionTier::LocalMachine && step.provider_id.is_none()));
    assert!(selection.escalation_trace.iter().any(|step| {
        step.tier == ExecutionTier::ExternalProvider
            && step.provider_id.as_deref() == Some("RustRuntimeProvider")
            && step.result == "selected"
    }));
}

#[test]
fn runtime_escalation_chain_is_shared_between_execution_and_workspace_routers() {
    assert_eq!(
        ExecutionRouter::tier_order(),
        ExecutionTier::ESCALATION_CHAIN
    );
    assert_eq!(
        RUNTIME_FAILOVER_PRIORITY,
        [
            WorkspaceRuntimeType::Dea,
            WorkspaceRuntimeType::Docker,
            WorkspaceRuntimeType::External,
            WorkspaceRuntimeType::Cloud
        ]
    );
    assert_eq!(
        RUNTIME_FAILOVER_TIERS,
        [
            ExecutionTier::LocalMachine,
            ExecutionTier::LocalDocker,
            ExecutionTier::ExternalProvider,
            ExecutionTier::CloudPartner
        ]
    );
}

#[test]
fn execution_router_selects_docker_provider_for_docker_manifest_command() {
    let repo = temp_dir("router-docker-provider");
    fs::write(
        repo.join(".execution.json"),
        r#"{"startCommand":"docker compose up"}"#,
    )
    .expect("write execution manifest");

    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "run".to_string(),
            node_type: ExecutionNodeType::DevServer,
            command: Some("docker compose up".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec![],
            outputs: vec![],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();

    let mut analysis = test_analysis(
        graph.clone(),
        WasmCompatibility::NotSupported,
        Framework::Node,
    );
    analysis.execution_profile.runtime_affinity = RuntimeAffinity {
        preferred_provider: "DockerExecutionProvider".to_string(),
        fallback_providers: vec!["NodeRuntimeProvider".to_string()],
    };
    let runtime_spec = analysis.runtime_spec.clone();
    let compiled_runtime = analysis.compiled_runtime.clone();
    let ctx = ExecutionContext {
        workspace_id: "ws-docker-manifest-routing".to_string(),
        repo_path: repo.to_string_lossy().to_string(),
        runtime_spec,
        compiled_runtime,
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };

    let router = ExecutionRouter::new(vec![
        Box::new(DockerExecutionProvider),
        Box::new(NodeRuntimeProvider),
        Box::new(RustRuntimeProvider),
    ]);
    let selection = router.select(&ctx).expect("select docker provider");
    assert_eq!(selection.provider_id, "DockerExecutionProvider");
    assert_eq!(selection.selected_tier, ExecutionTier::LocalDocker);
}

#[test]
fn docker_provider_rejects_bare_docker_command() {
    let err = DockerExecutionProvider::ensure_docker_ready("docker")
        .expect_err("bare docker command should be rejected");
    assert!(matches!(
        err,
        RuntimeError::CommandFailed(message)
            if message.contains("missing a subcommand")
    ));
}

#[test]
fn docker_provider_detects_compose_command_prefixes() {
    assert!(DockerExecutionProvider::is_compose_command(
        "docker compose up"
    ));
    assert!(DockerExecutionProvider::is_compose_command(
        "docker-compose up --build"
    ));
    assert!(!DockerExecutionProvider::is_compose_command(
        "docker run hello-world"
    ));
}

#[test]
fn execution_router_uses_generated_runtime_spec_for_provider_matching() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("cargo build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["Cargo.toml".to_string()],
            outputs: vec!["target".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let mut analysis = test_analysis(
        graph.clone(),
        WasmCompatibility::Partial,
        Framework::Unknown,
    );
    analysis.runtime_spec.language = "rust".to_string();
    analysis.runtime_spec.framework = "unknown".to_string();
    analysis.execution_profile.runtime_affinity = RuntimeAffinity {
        preferred_provider: "RustRuntimeProvider".to_string(),
        fallback_providers: vec!["NodeRuntimeProvider".to_string()],
    };
    let runtime_spec = analysis.runtime_spec.clone();
    let compiled_runtime = analysis.compiled_runtime.clone();
    let ctx = ExecutionContext {
        workspace_id: "ws-runtime-spec-routing".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec,
        compiled_runtime,
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };
    let router = ExecutionRouter::new(vec![
        Box::new(NodeRuntimeProvider),
        Box::new(RustRuntimeProvider),
    ]);
    let selection = router.select(&ctx).expect("select provider");
    assert_eq!(selection.provider_id, "RustRuntimeProvider");
}

#[test]
fn execution_router_selects_go_provider_for_go_runtime() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("go build ./...".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["go.mod".to_string()],
            outputs: vec!["bin/app".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let mut analysis = test_analysis(graph.clone(), WasmCompatibility::NotSupported, Framework::Go);
    analysis.runtime_spec.language = "go".to_string();
    analysis.runtime_spec.framework = "go".to_string();
    analysis.runtime_spec.package_manager = Some("go".to_string());
    analysis.execution_profile.runtime_affinity = RuntimeAffinity {
        preferred_provider: "GoExecutionProvider".to_string(),
        fallback_providers: vec!["RustRuntimeProvider".to_string()],
    };
    let runtime_spec = analysis.runtime_spec.clone();
    let compiled_runtime = analysis.compiled_runtime.clone();
    let ctx = ExecutionContext {
        workspace_id: "ws-go-provider".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec,
        compiled_runtime,
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };
    let router = ExecutionRouter::new(vec![
        Box::new(NodeRuntimeProvider),
        Box::new(GoExecutionProvider),
        Box::new(RustRuntimeProvider),
    ]);
    let selection = router.select(&ctx).expect("select provider");
    assert_eq!(selection.provider_id, "GoExecutionProvider");
}

#[test]
fn execution_router_selects_python_provider_for_uv_package_manager() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "run".to_string(),
            node_type: ExecutionNodeType::DevServer,
            command: Some("uv run python main.py".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["pyproject.toml".to_string()],
            outputs: vec![],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let mut analysis = test_analysis(
        graph.clone(),
        WasmCompatibility::NotSupported,
        Framework::Unknown,
    );
    analysis.runtime_spec.language = "python".to_string();
    analysis.runtime_spec.framework = "unknown".to_string();
    analysis.runtime_spec.package_manager = Some("uv".to_string());
    analysis.execution_profile.runtime_affinity = RuntimeAffinity {
        preferred_provider: "PythonExecutionProvider".to_string(),
        fallback_providers: vec!["NodeRuntimeProvider".to_string()],
    };
    let runtime_spec = analysis.runtime_spec.clone();
    let compiled_runtime = analysis.compiled_runtime.clone();
    let ctx = ExecutionContext {
        workspace_id: "ws-python-uv-provider".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec,
        compiled_runtime,
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };
    let router = ExecutionRouter::new(vec![
        Box::new(NodeRuntimeProvider),
        Box::new(PythonExecutionProvider),
        Box::new(RustRuntimeProvider),
    ]);
    let selection = router.select(&ctx).expect("select provider");
    assert_eq!(selection.provider_id, "PythonExecutionProvider");
}

#[test]
fn execution_router_selects_java_provider_for_java_runtime() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "run".to_string(),
            node_type: ExecutionNodeType::DevServer,
            command: Some("mvn spring-boot:run".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["pom.xml".to_string()],
            outputs: vec![],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let mut analysis = test_analysis(
        graph.clone(),
        WasmCompatibility::NotSupported,
        Framework::Unknown,
    );
    analysis.runtime_spec.language = "java".to_string();
    analysis.runtime_spec.framework = "java".to_string();
    analysis.runtime_spec.package_manager = Some("maven".to_string());
    analysis.execution_profile.runtime_affinity = RuntimeAffinity {
        preferred_provider: "JavaExecutionProvider".to_string(),
        fallback_providers: vec!["NodeRuntimeProvider".to_string()],
    };
    let runtime_spec = analysis.runtime_spec.clone();
    let compiled_runtime = analysis.compiled_runtime.clone();
    let ctx = ExecutionContext {
        workspace_id: "ws-java-provider".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec,
        compiled_runtime,
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };
    let router = ExecutionRouter::new(vec![
        Box::new(NodeRuntimeProvider),
        Box::new(JavaExecutionProvider),
        Box::new(RustRuntimeProvider),
    ]);
    let selection = router.select(&ctx).expect("select provider");
    assert_eq!(selection.provider_id, "JavaExecutionProvider");
}

#[test]
fn wasm_runtime_engine_executes_module_and_reports_exports() {
    let wasm_bytes = parse_str(
        r#"
            (module
                (func (export "run"))
            )
            "#,
    )
    .expect("compile wat");
    let spec = WasmRuntimeSpec {
        enabled: true,
        wasi: true,
        memory_limit_mb: 64,
        cpu_limit_units: 2_000,
        allowed_syscalls: vec!["fd_read".to_string(), "fd_write".to_string()],
    };
    let runtime = WasmRuntimeEngine::new().expect("create wasm runtime engine");
    let result = runtime
        .execute_module(&wasm_bytes, &spec, &WasiContext::default())
        .expect("execute wasm module");

    assert!(result.exported_functions.contains(&"run".to_string()));
}

#[test]
fn wasm_runtime_engine_enforces_memory_limit() {
    let wasm_bytes = parse_str(
        r#"
            (module
                (memory (export "memory") 32)
            )
            "#,
    )
    .expect("compile wat");
    let spec = WasmRuntimeSpec {
        enabled: true,
        wasi: true,
        memory_limit_mb: 1,
        cpu_limit_units: 1_000,
        allowed_syscalls: vec![],
    };
    let runtime = WasmRuntimeEngine::new().expect("create wasm runtime engine");
    let err = runtime
        .execute_module(&wasm_bytes, &spec, &WasiContext::default())
        .expect_err("memory limit should be enforced");

    assert!(matches!(err, RuntimeError::WasmRuntime(message) if message.contains("exceeds limit")));
}

#[test]
fn wasi_kernel_executes_module_within_capabilities() {
    let module_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");
    let mut capabilities = CapabilitySet::default();
    capabilities.insert("filesystem.read");
    let component_graph = WasiComponentGraph {
        components: vec![WasiComponent {
            id: "filesystem".to_string(),
            module: "filesystem.wasm".to_string(),
            imports: vec![],
            exports: vec!["filesystem.read".to_string()],
            capabilities: vec!["filesystem.read".to_string()],
        }],
        links: vec![],
        capabilities,
        imports: vec![],
        exports: vec![],
        runtime_constraints: RuntimeConstraints {
            read_only_paths: vec!["/workspace".to_string()],
            network_allowlist: vec![],
            max_memory_mb: 64,
            max_cpu_units: 2_000,
            process_spawn_bounded: true,
        },
        execution_plan: ExecutionPlan::default(),
    };
    let request = WasiKernelExecutionRequest {
        component_graph,
        runtime_spec: WasmRuntimeSpec {
            enabled: true,
            wasi: true,
            memory_limit_mb: 64,
            cpu_limit_units: 2_000,
            allowed_syscalls: vec!["fd_read".to_string()],
        },
        capabilities: WasiKernelCapability {
            filesystem: WasiKernelFilesystemCapability {
                read: vec!["/workspace".to_string()],
                write: vec![],
            },
            network: WasiKernelNetworkCapability::default(),
            process: WasiKernelProcessCapability::default(),
            env: WasiKernelEnvCapability {
                variables: vec!["CI".to_string()],
            },
            runtime: WasiKernelRuntimeCapability {
                memory_limit_mb: 64,
                cpu_limit: 2_000.0,
            },
        },
        filesystem_snapshot: WorkspaceSnapshot::default(),
        environment: BTreeMap::from([(String::from("CI"), String::from("true"))]),
        module_bytes,
    };

    let mut kernel = WasiKernel::new().expect("create kernel");
    let response = kernel.execute(&request).expect("execute through kernel");

    assert_eq!(response.result, "ok");
    assert!(response.logs.contains(&"run".to_string()));
    assert!(response.trace_id.starts_with("trace-"));
    assert!(response.metrics.contains_key("execution_ms"));
    assert!(response.metrics.contains_key("exported_function_count"));
    assert!(response.execution_graph_diff.is_empty());
}

#[test]
fn wasi_kernel_rejects_network_capability_violations() {
    let module_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");
    let mut capabilities = CapabilitySet::default();
    capabilities.insert("network.http");
    let component_graph = WasiComponentGraph {
        components: vec![WasiComponent {
            id: "network".to_string(),
            module: "network.wasm".to_string(),
            imports: vec![],
            exports: vec!["network.http".to_string()],
            capabilities: vec!["network.http".to_string()],
        }],
        links: vec![],
        capabilities,
        imports: vec![],
        exports: vec![],
        runtime_constraints: RuntimeConstraints {
            read_only_paths: vec!["/workspace".to_string()],
            network_allowlist: vec!["crates.io".to_string()],
            max_memory_mb: 64,
            max_cpu_units: 2_000,
            process_spawn_bounded: true,
        },
        execution_plan: ExecutionPlan::default(),
    };
    let request = WasiKernelExecutionRequest {
        component_graph,
        runtime_spec: WasmRuntimeSpec {
            enabled: true,
            wasi: true,
            memory_limit_mb: 64,
            cpu_limit_units: 2_000,
            allowed_syscalls: vec![],
        },
        capabilities: WasiKernelCapability {
            filesystem: WasiKernelFilesystemCapability::default(),
            network: WasiKernelNetworkCapability::default(),
            process: WasiKernelProcessCapability::default(),
            env: WasiKernelEnvCapability::default(),
            runtime: WasiKernelRuntimeCapability {
                memory_limit_mb: 64,
                cpu_limit: 2_000.0,
            },
        },
        filesystem_snapshot: WorkspaceSnapshot::default(),
        environment: BTreeMap::new(),
        module_bytes,
    };

    let mut kernel = WasiKernel::new().expect("create kernel");
    let err = kernel
        .execute(&request)
        .expect_err("network capability validation should fail");
    assert!(matches!(err, RuntimeError::WasmRuntime(message) if message.contains("allowlist")));
}

#[test]
fn wasi_kernel_reports_execution_graph_diff_when_links_are_missing() {
    let module_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");
    let component_graph = WasiComponentGraph {
        components: vec![
            WasiComponent {
                id: "producer".to_string(),
                module: "producer.wasm".to_string(),
                imports: vec![],
                exports: vec!["filesystem.read".to_string()],
                capabilities: vec!["filesystem.read".to_string()],
            },
            WasiComponent {
                id: "consumer".to_string(),
                module: "consumer.wasm".to_string(),
                imports: vec!["filesystem.read".to_string()],
                exports: vec![],
                capabilities: vec![],
            },
        ],
        links: vec![],
        capabilities: CapabilitySet::default(),
        imports: vec!["import:consumer:filesystem.read".to_string()],
        exports: vec!["export:producer:filesystem.read".to_string()],
        runtime_constraints: RuntimeConstraints {
            read_only_paths: vec!["/workspace".to_string()],
            network_allowlist: vec![],
            max_memory_mb: 64,
            max_cpu_units: 2_000,
            process_spawn_bounded: true,
        },
        execution_plan: ExecutionPlan::default(),
    };
    let request = WasiKernelExecutionRequest {
        component_graph,
        runtime_spec: WasmRuntimeSpec {
            enabled: true,
            wasi: true,
            memory_limit_mb: 64,
            cpu_limit_units: 2_000,
            allowed_syscalls: vec![],
        },
        capabilities: WasiKernelCapability {
            filesystem: WasiKernelFilesystemCapability {
                read: vec!["/workspace".to_string()],
                write: vec![],
            },
            network: WasiKernelNetworkCapability::default(),
            process: WasiKernelProcessCapability::default(),
            env: WasiKernelEnvCapability::default(),
            runtime: WasiKernelRuntimeCapability {
                memory_limit_mb: 64,
                cpu_limit: 2_000.0,
            },
        },
        filesystem_snapshot: WorkspaceSnapshot::default(),
        environment: BTreeMap::new(),
        module_bytes,
    };

    let mut kernel = WasiKernel::new().expect("create kernel");
    let response = kernel.execute(&request).expect("execute through kernel");
    assert_eq!(
        response.execution_graph_diff,
        vec!["producer->consumer:filesystem.read".to_string()]
    );
}

#[test]
fn hybrid_execution_bridge_dispatches_wasm_nodes() {
    let node = ExecutionNode {
        id: "serve".to_string(),
        node_type: ExecutionNodeType::StaticServe,
        command: Some("serve".to_string()),
        execution_mode: ExecutionMode::Wasm,
        inputs: vec![],
        outputs: vec![],
        cache_key: None,
        runtime: None,
        cache_binding: None,
    };
    let graph = ExecutionGraph {
        nodes: vec![node.clone()],
        edges: vec![],
    }
    .with_cache_keys();
    let analysis = test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb);
    let ctx = ExecutionContext {
        workspace_id: "ws-1".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec: analysis.runtime_spec.clone(),
        compiled_runtime: analysis.compiled_runtime.clone(),
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };
    let bridge = HybridExecutionBridge::new().expect("create hybrid bridge");
    let wasm_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");

    let handle = bridge
        .dispatch(&node, &ctx, &WasiContext::default(), &wasm_bytes)
        .expect("dispatch wasm node");
    assert!(handle.pid_hint.starts_with("wasm:"));
}

#[test]
fn wasm_execution_provider_uses_compiled_wasm_artifact() {
    let repo_root = temp_dir("wasm-provider-artifact");
    let pkg = repo_root.join("pkg");
    fs::create_dir_all(&pkg).expect("create wasm output dir");
    let wasm_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");
    fs::write(pkg.join("app_bg.wasm"), wasm_bytes).expect("write wasm artifact");

    let node = ExecutionNode {
        id: "serve".to_string(),
        node_type: ExecutionNodeType::StaticServe,
        command: Some("serve".to_string()),
        execution_mode: ExecutionMode::Wasm,
        inputs: vec![],
        outputs: vec!["pkg".to_string()],
        cache_key: None,
        runtime: None,
        cache_binding: None,
    };
    let graph = ExecutionGraph {
        nodes: vec![node],
        edges: vec![],
    }
    .with_cache_keys();
    let analysis = test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb);
    let ctx = ExecutionContext {
        workspace_id: "ws-1".to_string(),
        repo_path: repo_root.to_string_lossy().to_string(),
        runtime_spec: analysis.runtime_spec.clone(),
        compiled_runtime: analysis.compiled_runtime.clone(),
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };

    let provider = WasmExecutionProvider;
    let handle = provider.start(&ctx).expect("start wasm provider");
    assert!(handle.pid_hint.starts_with("wasm:"));
}

#[test]
fn wasm_execution_provider_handles_wasm_compile_then_serve_graph() {
    let repo_root = temp_dir("wasm-provider-compile-then-serve");
    let pkg = repo_root.join("pkg");
    fs::create_dir_all(&pkg).expect("create wasm output dir");
    let wasm_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");
    fs::write(pkg.join("app_bg.wasm"), wasm_bytes).expect("write wasm artifact");

    let graph = ExecutionGraph {
        nodes: vec![
            ExecutionNode {
                id: "wasm-compile".to_string(),
                node_type: ExecutionNodeType::WasmCompile,
                command: Some("wasm-pack build --target web".to_string()),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["src".to_string()],
                outputs: vec!["pkg/app_bg.wasm".to_string()],
                cache_key: None,
                runtime: None,
                cache_binding: None,
            },
            ExecutionNode {
                id: "serve".to_string(),
                node_type: ExecutionNodeType::StaticServe,
                command: Some("serve .".to_string()),
                execution_mode: ExecutionMode::Wasm,
                inputs: vec!["pkg/app_bg.wasm".to_string()],
                outputs: vec!["http://0.0.0.0:4173/".to_string()],
                cache_key: None,
                runtime: None,
                cache_binding: None,
            },
        ],
        edges: vec![ExecutionEdge {
            from: "wasm-compile".to_string(),
            to: "serve".to_string(),
        }],
    }
    .with_cache_keys();
    let analysis = test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb);
    let ctx = ExecutionContext {
        workspace_id: "ws-1".to_string(),
        repo_path: repo_root.to_string_lossy().to_string(),
        runtime_spec: analysis.runtime_spec.clone(),
        compiled_runtime: analysis.compiled_runtime.clone(),
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };

    let provider = WasmExecutionProvider;
    assert!(provider.can_handle(&ctx));
    let handle = provider
        .start(&ctx)
        .expect("start mixed wasm compile/serve provider");
    assert!(handle.pid_hint.starts_with("wasm:serve:"));
}

#[test]
fn wasm_execution_provider_requires_compiled_wasm_artifact() {
    let repo_root = temp_dir("wasm-provider-missing-artifact");
    let node = ExecutionNode {
        id: "serve".to_string(),
        node_type: ExecutionNodeType::StaticServe,
        command: Some("serve".to_string()),
        execution_mode: ExecutionMode::Wasm,
        inputs: vec![],
        outputs: vec!["pkg".to_string()],
        cache_key: None,
        runtime: None,
        cache_binding: None,
    };
    let graph = ExecutionGraph {
        nodes: vec![node],
        edges: vec![],
    }
    .with_cache_keys();
    let analysis = test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb);
    let ctx = ExecutionContext {
        workspace_id: "ws-1".to_string(),
        repo_path: repo_root.to_string_lossy().to_string(),
        runtime_spec: analysis.runtime_spec.clone(),
        compiled_runtime: analysis.compiled_runtime.clone(),
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };

    let provider = WasmExecutionProvider;
    let err = provider
        .start(&ctx)
        .expect_err("expected missing artifact to fail");
    assert!(
        matches!(err, RuntimeError::WasmRuntime(message) if message.contains("no compiled wasm artifact found"))
    );
}

#[test]
fn cache_key_engine_changes_with_execution_mode() {
    let mut graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("cargo build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["Cargo.toml".to_string()],
            outputs: vec!["target".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    };

    let first = graph.compute_cache_keys();
    graph.nodes[0].execution_mode = ExecutionMode::Wasm;
    let second = graph.compute_cache_keys();
    assert_ne!(first.get("build"), second.get("build"));
}

#[test]
fn workspace_session_tracks_graph_events_and_controls() {
    let mut session =
        WorkspaceSession::new("session-1", "repo-1", "graph-1", "http://coordinator:8080");
    assert_eq!(session.sync_state, WorkspaceSessionSyncState::Connecting);

    session.record_graph_event(GraphEvent {
        node_id: "build".to_string(),
        event_type: GraphEventType::NodeStarted,
        timestamp: 10,
    });
    assert_eq!(session.stream_for_node("build").len(), 1);

    session.apply_control(ExecutionControl::Pause);
    assert_eq!(session.sync_state, WorkspaceSessionSyncState::Paused);

    session.apply_control(ExecutionControl::RetryNode {
        node_id: "build".to_string(),
    });
    assert_eq!(session.sync_state, WorkspaceSessionSyncState::Live);
    assert_eq!(
        session
            .stream_for_node("build")
            .last()
            .map(|event| event.event_type),
        Some(GraphEventType::NodeQueued)
    );
}

#[test]
fn workspace_session_event_buffers_are_bounded() {
    let mut session =
        WorkspaceSession::new("session-2", "repo-2", "graph-2", "http://coordinator:8080");
    for index in 0..=SESSION_GRAPH_EVENT_BUFFER_LIMIT {
        session.record_graph_event(GraphEvent {
            node_id: format!("node-{index}"),
            event_type: GraphEventType::NodeStarted,
            timestamp: index as u64,
        });
    }
    for index in 0..=SESSION_WORKER_EVENT_BUFFER_LIMIT {
        session.record_worker_event(WorkerEvent {
            worker_id: "worker-1".to_string(),
            message: format!("event-{index}"),
            timestamp: index as u64,
        });
    }

    assert_eq!(session.graph_events.len(), SESSION_GRAPH_EVENT_BUFFER_LIMIT);
    assert_eq!(
        session.worker_events.len(),
        SESSION_WORKER_EVENT_BUFFER_LIMIT
    );
    assert_eq!(
        session
            .graph_events
            .front()
            .map(|event| event.node_id.as_str()),
        Some("node-1")
    );
    assert_eq!(
        session
            .worker_events
            .front()
            .map(|event| event.message.as_str()),
        Some("event-1")
    );
}

#[test]
fn execution_graph_view_applies_live_state_events() {
    let mut graph_view = ExecutionGraphView {
        nodes: vec![UIExecutionNode {
            id: "test".to_string(),
        }],
        ..ExecutionGraphView::default()
    };
    let event = GraphEvent {
        node_id: "test".to_string(),
        event_type: GraphEventType::NodeCompleted,
        timestamp: 25,
    };

    graph_view.apply_graph_event(&event);

    assert_eq!(
        graph_view.live_states.get("test"),
        Some(&GraphEventType::NodeCompleted)
    );
}

#[test]
fn browser_ide_syncs_files_and_streams_logs() {
    let mut ide = BrowserIDE::default();
    ide.sync_file_content("src/lib.rs", "pub fn sample() {}");
    assert_eq!(ide.file_tree.files, vec!["src/lib.rs".to_string()]);
    assert_eq!(ide.editor.active_path.as_deref(), Some("src/lib.rs"));
    assert!(!ide.editor.dirty);

    ide.append_log(WorkerEvent {
        worker_id: "worker-1".to_string(),
        message: "node build started".to_string(),
        timestamp: 100,
    });
    assert_eq!(ide.log_stream.entries.len(), 1);
}

#[test]
fn control_plane_workspace_state_transitions_cover_runtime_lifecycle() {
    assert!(can_transition(
        WorkspaceState::Pending,
        WorkspaceState::Provisioning
    ));
    assert!(can_transition(
        WorkspaceState::Provisioning,
        WorkspaceState::Starting
    ));
    assert!(can_transition(
        WorkspaceState::Running,
        WorkspaceState::Degraded
    ));
    assert!(can_transition(
        WorkspaceState::Degraded,
        WorkspaceState::Restarting
    ));
    assert!(can_transition(
        WorkspaceState::Running,
        WorkspaceState::Migrating
    ));
    assert!(!can_transition(
        WorkspaceState::Pending,
        WorkspaceState::Running
    ));
}

#[test]
fn ucpe_ti_rest_api_exposes_unified_control_plane_routes() {
    let spec = RestApiSpec::default();
    assert!(spec.routes.contains(&"POST /execute"));
    assert!(spec.routes.contains(&"GET /state/{execution_id}"));
    assert!(spec.routes.contains(&"POST /migrate/{execution_id}"));
    assert!(spec.routes.contains(&"GET /agents"));
    assert!(spec.routes.contains(&"GET /topology/{id}"));
}

#[test]
fn rest_api_spec_includes_execution_api_layer_routes() {
    let spec = RestApiSpec::default();
    assert!(spec.routes.contains(&"POST /auth/login"));
    assert!(spec.routes.contains(&"POST /auth/logout"));
    assert!(spec.routes.contains(&"GET /auth/me"));
    assert!(spec.routes.contains(&"GET /auth/github/callback"));
    assert!(spec.routes.contains(&"GET /auth/google/callback"));
    assert!(spec.routes.contains(&"POST /orgs"));
    assert!(spec.routes.contains(&"GET /orgs/{org_id}"));
    assert!(spec.routes.contains(&"POST /orgs/{org_id}/members"));
    assert!(spec.routes.contains(&"POST /workspaces"));
    assert!(spec.routes.contains(&"GET /workspaces?org_id={org_id}"));
    assert!(spec.routes.contains(&"GET /workspaces/{id}"));
    assert!(spec.routes.contains(&"POST /workspaces/{id}/bind"));
    assert!(spec.routes.contains(&"POST /workspaces/{id}/migrate"));
    assert!(spec.routes.contains(&"DELETE /workspaces/{id}"));
    assert!(spec.routes.contains(&"GET /executions?org_id={org_id}"));
    assert!(spec.routes.contains(&"POST /api/v1/repositories/analyze"));
    assert!(spec.routes.contains(&"POST /api/v1/repositories/publish"));
    assert!(spec.routes.contains(&"POST /api/v1/execution/plan"));
    assert!(spec.routes.contains(&"POST /api/v1/executions"));
    assert!(spec.routes.contains(&"POST /api/v1/executions/{id}/claim"));
    assert!(spec.routes.contains(&"GET /api/v1/executions/{id}"));
    assert!(spec.routes.contains(&"GET /api/v1/executions/{id}/logs"));
    assert!(spec
        .routes
        .contains(&"POST /api/v1/executions/{id}/restart"));
    assert!(spec.routes.contains(&"POST /api/v1/executions/{id}/stop"));
    assert!(spec
        .routes
        .contains(&"POST /api/v1/executions/{id}/migrate"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/history"));
    assert!(spec.routes.contains(&"GET /executions/{id}/history"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/healing"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/last-good"));
    assert!(spec
        .routes
        .contains(&"GET /api/repositories/{id}/intelligence"));
    assert!(spec.routes.contains(&"POST /api/repositories/{id}/ask"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/twin"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/behavior"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/architecture"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/timeline"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/predictions"));
    assert!(spec
        .routes
        .contains(&"GET /repositories/{id}/recommendations"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/blast-radius"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/dna"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/risk"));
    assert!(spec.routes.contains(&"GET /repositories/{id}/memory"));
    assert!(spec.routes.contains(&"POST /repositories/{id}/simulate"));
    assert!(spec.routes.contains(&"POST /repositories/{id}/infer"));
    assert!(spec.routes.contains(&"POST /repositories/{id}/compare"));
    assert!(spec.routes.contains(&"POST /repositories/{id}/predict"));
    assert!(spec.routes.contains(&"POST /repositories/{id}/explain"));
    assert!(spec.routes.contains(&"GET /intelligence/{execution}"));
    assert!(spec.routes.contains(&"GET /intelligence/similar"));
    assert!(spec.routes.contains(&"GET /intelligence/patterns"));
    assert!(spec.routes.contains(&"GET /intelligence/repairs"));
    assert!(spec.routes.contains(&"GET /intelligence/context"));
    assert!(spec.routes.contains(&"POST /intelligence/retrieve"));
    assert!(spec.routes.contains(&"POST /intelligence/learn"));
    assert!(spec.routes.contains(&"POST /intelligence/optimize"));
    assert!(spec.routes.contains(&"GET /billing/usage?org_id={org_id}"));
    assert!(spec.routes.contains(&"GET /billing/summary"));
    assert!(spec.routes.contains(&"POST /billing/invoice"));
    assert!(spec.routes.contains(&"GET /api/v1/dual-surface/contract"));
    assert!(spec
        .routes
        .contains(&"GET /api/v1/surfaces/extension/actions"));
    assert!(spec
        .routes
        .contains(&"GET /api/v1/surfaces/portal/navigation"));
    assert!(spec.routes.contains(&"GET /api/v1/surfaces/extension/ui"));
    assert!(spec.routes.contains(&"GET /api/v1/surfaces/portal/ui"));
    assert!(spec.routes.contains(&"POST /api/badges/generate"));
    assert!(spec.routes.contains(&"POST /api/badge/generate"));
    assert!(spec.routes.contains(&"GET /badge/{owner}/{repo}.svg"));
    assert!(spec
        .routes
        .contains(&"GET /badge/healed/{owner}/{repo}.svg"));
    assert!(spec.routes.contains(&"GET /seed/{owner}/{repo}"));
}

#[test]
fn ucpe_ti_scheduler_follows_policy_tiers() {
    let scheduler = ucpe_ti::ExecutionScheduler;
    let policy = ucpe_ti::PolicyEngine::default();

    let trusted_cached = scheduler.schedule(
        &ucpe_ti::SchedulingContext {
            authenticated_identity: true,
            trusted_repo: true,
            cached_runtime: true,
            cold_start_required: false,
            resource_heavy: false,
        },
        &policy,
    );
    assert_eq!(trusted_cached, ExecutionTier::LocalMachine);

    let cold_start = scheduler.schedule(
        &ucpe_ti::SchedulingContext {
            authenticated_identity: true,
            trusted_repo: false,
            cached_runtime: false,
            cold_start_required: true,
            resource_heavy: false,
        },
        &policy,
    );
    assert_eq!(cold_start, ExecutionTier::LocalDocker);

    let heavy = scheduler.schedule(
        &ucpe_ti::SchedulingContext {
            authenticated_identity: true,
            trusted_repo: true,
            cached_runtime: true,
            cold_start_required: false,
            resource_heavy: true,
        },
        &policy,
    );
    assert_eq!(heavy, ExecutionTier::DDockitCloud);

    let anonymous = scheduler.schedule(
        &ucpe_ti::SchedulingContext {
            authenticated_identity: false,
            trusted_repo: true,
            cached_runtime: true,
            cold_start_required: false,
            resource_heavy: false,
        },
        &policy,
    );
    assert_eq!(anonymous, ExecutionTier::ExternalProvider);
}

#[test]
fn mesh_scheduler_places_wasi_components_across_node_types() {
    let graph = WasiComponentGraph {
        components: vec![
            WasiComponent {
                id: "build".to_string(),
                capabilities: vec!["build.compute".to_string(), "filesystem".to_string()],
                ..WasiComponent::default()
            },
            WasiComponent {
                id: "serve".to_string(),
                capabilities: vec!["http.serve".to_string(), "latency.sensitive".to_string()],
                ..WasiComponent::default()
            },
        ],
        ..WasiComponentGraph::default()
    };
    let nodes = vec![
        MeshNode {
            id: "local-1".to_string(),
            node_type: MeshNodeType::Local,
            trust_level: MeshNodeTrustLevel::FullAccess,
            capabilities: WorkerCapabilities {
                wasm: true,
                native: true,
                cpu_cores: 2,
                memory_mb: 2048,
                labels: vec!["filesystem".to_string()],
            },
            status: WorkerStatus::Ready,
        },
        MeshNode {
            id: "cloud-1".to_string(),
            node_type: MeshNodeType::Cloud,
            trust_level: MeshNodeTrustLevel::Sandboxed,
            capabilities: WorkerCapabilities {
                wasm: true,
                native: true,
                cpu_cores: 8,
                memory_mb: 8192,
                labels: vec!["filesystem".to_string(), "network".to_string()],
            },
            status: WorkerStatus::Ready,
        },
        MeshNode {
            id: "edge-1".to_string(),
            node_type: MeshNodeType::Edge,
            trust_level: MeshNodeTrustLevel::RestrictedIo,
            capabilities: WorkerCapabilities {
                wasm: true,
                native: false,
                cpu_cores: 4,
                memory_mb: 2048,
                labels: vec!["network".to_string()],
            },
            status: WorkerStatus::Ready,
        },
    ];

    let planned = MeshScheduler.plan(&graph, &nodes);
    let build = planned
        .placements
        .iter()
        .find(|placement| placement.component_id == "build")
        .expect("build placement");
    let serve = planned
        .placements
        .iter()
        .find(|placement| placement.component_id == "serve")
        .expect("serve placement");

    assert_eq!(build.preferred_node_type, MeshNodeType::Cloud);
    assert_eq!(serve.preferred_node_type, MeshNodeType::Edge);
    assert_eq!(
        build.fallback_nodes.first().map(String::as_str),
        Some("cloud-1")
    );
    assert_eq!(
        serve.fallback_nodes.first().map(String::as_str),
        Some("edge-1")
    );
    assert!(planned
        .partitions
        .iter()
        .any(|partition| partition.node_id == "cloud-1"));
    assert!(planned
        .partitions
        .iter()
        .any(|partition| partition.node_id == "edge-1"));
}

#[test]
fn mesh_router_supports_migration_and_replication() {
    let router = MeshExecutionRouter;
    let mut placements = vec![ComponentPlacement {
        component_id: "install".to_string(),
        preferred_node_type: MeshNodeType::Cloud,
        constraints: ComponentPlacementConstraints {
            cpu: 2,
            memory_mb: 1024,
            network: true,
            filesystem: true,
            latency_sensitive: false,
        },
        affinity_rules: vec![],
        fallback_nodes: vec![
            "cloud-1".to_string(),
            "edge-1".to_string(),
            "local-1".to_string(),
        ],
    }];

    assert!(router.migrate("install", &mut placements, "edge-1"));
    assert_eq!(
        placements[0].fallback_nodes.first().map(String::as_str),
        Some("edge-1")
    );
    assert_eq!(
        router.replicate("install", &placements, 2),
        vec!["edge-1".to_string(), "cloud-1".to_string()]
    );
}

#[test]
fn execution_mesh_heals_failed_components_with_fallback_node() {
    let mut mesh = ExecutionMesh::default();
    mesh.nodes = vec![
        MeshNode {
            id: "cloud-1".to_string(),
            node_type: MeshNodeType::Cloud,
            trust_level: MeshNodeTrustLevel::Sandboxed,
            capabilities: WorkerCapabilities {
                wasm: true,
                native: true,
                cpu_cores: 4,
                memory_mb: 4096,
                labels: vec!["filesystem".to_string(), "network".to_string()],
            },
            status: WorkerStatus::Ready,
        },
        MeshNode {
            id: "edge-1".to_string(),
            node_type: MeshNodeType::Edge,
            trust_level: MeshNodeTrustLevel::RestrictedIo,
            capabilities: WorkerCapabilities {
                wasm: true,
                native: true,
                cpu_cores: 2,
                memory_mb: 2048,
                labels: vec!["network".to_string()],
            },
            status: WorkerStatus::Ready,
        },
    ];

    let mut placements = vec![ComponentPlacement {
        component_id: "serve".to_string(),
        preferred_node_type: MeshNodeType::Edge,
        constraints: ComponentPlacementConstraints {
            cpu: 1,
            memory_mb: 256,
            network: true,
            filesystem: false,
            latency_sensitive: true,
        },
        affinity_rules: vec![],
        fallback_nodes: vec!["edge-1".to_string(), "cloud-1".to_string()],
    }];

    let moved_to = mesh.heal_component(
        &mut placements,
        "serve",
        "edge-1",
        MeshFailureClass::NodeUnavailable,
        42,
    );
    assert_eq!(moved_to.as_deref(), Some("cloud-1"));
    assert_eq!(
        placements[0].fallback_nodes.first().map(String::as_str),
        Some("cloud-1")
    );
    assert_eq!(mesh.failure_detector.events.len(), 1);
    assert_eq!(mesh.failure_detector.events[0].component_id, "serve");
}

#[test]
fn ucpe_ti_migration_engine_moves_execution_to_new_agent() {
    let fingerprint = RepositoryFingerprint::default();
    let image_spec = ExecutionImageCompiler::compile(&fingerprint).image_spec;
    let mut execution = ucpe_ti::ExecutionState {
        urfs: fingerprint,
        topology: test_topology("migration-topology"),
        execution_image: image_spec,
        selected_agent: AgentIdentity {
            agent_id: "agent-1".to_string(),
            device_fingerprint: "device-1".to_string(),
            public_key: "pk-1".to_string(),
            trusted: true,
        },
        runtime_status: ucpe_ti::RuntimeState::Running,
    };

    ucpe_ti::ExecutionMigrationEngine::migrate(
        &mut execution,
        AgentIdentity {
            agent_id: "agent-2".to_string(),
            device_fingerprint: "device-2".to_string(),
            public_key: "pk-2".to_string(),
            trusted: true,
        },
    );

    assert_eq!(execution.selected_agent.agent_id, "agent-2");
    assert_eq!(execution.runtime_status, ucpe_ti::RuntimeState::Migrated);
}

#[test]
fn ucpe_ti_control_plane_applies_events_to_single_state_of_truth() {
    let router = ExecutionRouter::new(vec![Box::new(StaticRuntimeProvider)]);
    let mut control_plane = ucpe_ti::ControlPlane::new(router);
    let fingerprint = RepositoryFingerprint {
        repo_id: "repo-ucpe".to_string(),
        repo_url: "repo-ucpe".to_string(),
        repo_hash: "repo-ucpe".to_string(),
        lockfile_hash: Some("lock".to_string()),
        dependency_hash: Some("deps".to_string()),
        language_signature: "javascript".to_string(),
        framework_signature: Some("nextjs".to_string()),
        ..RepositoryFingerprint::default()
    };
    let image_spec = ExecutionImageCompiler::compile(&fingerprint).image_spec;
    let topology = test_topology("topology-ucpe");
    let agent = AgentIdentity {
        agent_id: "agent-ucpe".to_string(),
        device_fingerprint: "device-ucpe".to_string(),
        public_key: "pk-ucpe".to_string(),
        trusted: true,
    };

    control_plane.apply_event(ucpe_ti::ControlPlaneEvent::RepositoryAnalyzed {
        repo_id: fingerprint.repo_id.clone(),
        fingerprint: fingerprint.clone(),
    });
    control_plane.apply_event(ucpe_ti::ControlPlaneEvent::ImageCompiled {
        repo_id: fingerprint.repo_id.clone(),
        spec: image_spec.clone(),
    });
    control_plane.apply_event(ucpe_ti::ControlPlaneEvent::TopologyBuilt {
        topology_id: topology.topology_id.clone(),
        topology: topology.clone(),
    });
    control_plane.apply_event(ucpe_ti::ControlPlaneEvent::AgentRegistered {
        agent_id: agent.agent_id.clone(),
        status: ucpe_ti::AgentStatusSnapshot {
            status: AgentStatus::Idle,
            load: 0,
            trust_level: 100,
            latency_ms: 10,
        },
    });
    control_plane.apply_event(ucpe_ti::ControlPlaneEvent::ExecutionStarted {
        execution_id: "exec-ucpe".to_string(),
        state: ucpe_ti::ExecutionState {
            urfs: fingerprint.clone(),
            topology: topology.clone(),
            execution_image: image_spec.clone(),
            selected_agent: agent.clone(),
            runtime_status: ucpe_ti::RuntimeState::Running,
        },
    });
    control_plane.apply_event(ucpe_ti::ControlPlaneEvent::ExecutionMigrated {
        execution_id: "exec-ucpe".to_string(),
        next_agent: AgentIdentity {
            agent_id: "agent-ucpe-2".to_string(),
            device_fingerprint: "device-ucpe-2".to_string(),
            public_key: "pk-ucpe-2".to_string(),
            trusted: true,
        },
    });

    assert!(control_plane
        .state
        .urfs_fingerprints
        .contains_key("repo-ucpe"));
    assert!(control_plane
        .state
        .execution_image_specs
        .contains_key("repo-ucpe"));
    assert!(control_plane
        .state
        .topology_graphs
        .contains_key("topology-ucpe"));
    assert!(control_plane.state.agent_states.contains_key("agent-ucpe"));
    assert_eq!(
        control_plane
            .state
            .executions
            .get("exec-ucpe")
            .map(|state| state.runtime_status),
        Some(ucpe_ti::RuntimeState::Migrated)
    );

    control_plane.apply_event(ucpe_ti::ControlPlaneEvent::ExecutionFailed {
        execution_id: "exec-ucpe".to_string(),
    });
    assert_eq!(
        control_plane
            .state
            .executions
            .get("exec-ucpe")
            .map(|state| state.runtime_status),
        Some(ucpe_ti::RuntimeState::Failed)
    );
    assert_eq!(
        control_plane
            .registry
            .executions
            .get("exec-ucpe")
            .map(|state| state.runtime_status),
        Some(ucpe_ti::RuntimeState::Failed)
    );
}

#[test]
fn workspace_router_resolves_stable_url_and_proxy_target() {
    let mut registry = WorkspaceRegistry::default();
    registry.upsert(WorkspaceRecord {
        workspace_id: "a1b2".to_string(),
        repository_id: "repo-1".to_string(),
        org_id: "org-1".to_string(),
        created_by: "user-1".to_string(),
        visibility: WorkspaceVisibility::Private,
        execution_id: "exec-1".to_string(),
        assigned_worker: Some("worker-3".to_string()),
        assigned_runtime: RuntimeType::Node,
        assigned_url: stable_workspace_url("a1b2", true),
        state: WorkspaceState::Running,
        created_at: 1,
        updated_at: 1,
        quota: WorkspaceQuota {
            max_cpu: 1000,
            max_memory: 2048,
            max_runtime_hours: 4,
        },
    });
    let mut proxy = WorkspaceProxy::default();
    proxy.bind("a1b2", "worker-3", "http://worker-3:3012");
    let router = WorkspaceRouter::default();

    let route = router
        .route_request(&registry, &proxy, "workspace-a1b2.trythissoftware.com")
        .expect("route for stable host should resolve");
    assert_eq!(route.worker_id, "worker-3");
    assert_eq!(route.target, "http://worker-3:3012");
    assert_eq!(
        registry
            .get("a1b2")
            .map(|record| record.assigned_url.clone()),
        Some(WorkspaceUrl(
            "workspace-a1b2.trythissoftware.com".to_string()
        ))
    );
}

#[test]
fn workspace_router_migrates_runtime_without_url_change() {
    let mut router = WorkspaceRouter::default();
    let workspace = router.create_workspace("repo-1", "aaaaaaa", "org-1", "user-1", 10);
    assert!(router.bind_runtime(
        &workspace.workspace_id,
        WorkspaceRuntimeBinding {
            runtime_type: WorkspaceRuntimeType::Dea,
            runtime_instance_id: "dea-worker-1".to_string(),
            endpoint: "http://dea-worker-1:3012".to_string(),
            lease_expires_at: 20,
            runtime_heartbeat: 10,
            last_request_time: 10,
            execution_health: true,
        },
        10
    ));
    let stable_url = router
        .registry
        .get(&workspace.workspace_id)
        .expect("workspace must exist")
        .assigned_url
        .clone();

    assert!(router.migrate_runtime(
        &workspace.workspace_id,
        WorkspaceRuntimeBinding {
            runtime_type: WorkspaceRuntimeType::Cloud,
            runtime_instance_id: "cloud-worker-9".to_string(),
            endpoint: "https://cloud-worker-9.trythissoftware.com".to_string(),
            lease_expires_at: 50,
            runtime_heartbeat: 21,
            last_request_time: 21,
            execution_health: true,
        },
        21
    ));
    let route = router
        .route_workspace_request(
            &format!("workspace-{}.trythissoftware.com", workspace.workspace_id),
            22,
        )
        .expect("workspace route should resolve after migration");
    assert_eq!(route.target, "https://cloud-worker-9.trythissoftware.com");
    assert_eq!(
        router
            .registry
            .get(&workspace.workspace_id)
            .expect("workspace must exist")
            .assigned_url,
        stable_url
    );
    assert!(router
        .events
        .iter()
        .any(|event| event.event_type == "runtime_migrated"));
    assert_eq!(
        router.select_failover_runtime(&[
            WorkspaceRuntimeType::Cloud,
            WorkspaceRuntimeType::External
        ]),
        Some(WorkspaceRuntimeType::External)
    );
}

#[test]
fn worker_heartbeat_and_lease_expiry_drive_failure_detection() {
    let worker = WorkerNode {
        id: "worker-a".to_string(),
        capabilities: WorkerCapabilities {
            wasm: true,
            native: true,
            cpu_cores: 8,
            memory_mb: 8192,
            labels: vec![],
        },
        status: WorkerStatus::Ready,
    };
    let mut workers = WorkerRegistry::from_workers(vec![worker], 10, 100);
    assert!(workers.record_worker_heartbeat(WorkerHeartbeat {
        worker_id: "worker-a".to_string(),
        cpu: 40,
        memory: 2048,
        running_workspaces: 3,
        health: true,
        timestamp: 105,
    }));
    let mut leases = ExecutionLeaseRegistry::default();
    leases.assign("a1b2", "worker-a", 105, 10);

    assert_eq!(workers.detect_failed_workers(116), vec!["worker-a"]);
    assert_eq!(leases.expire_for_worker("worker-a", 116), vec!["a1b2"]);
}

#[test]
fn distributed_execution_agent_registers_and_reports_heartbeat() {
    let mut agent = DistributedExecutionAgent::new(AgentIdentity {
        agent_id: "agent-1".to_string(),
        device_fingerprint: "device-123".to_string(),
        public_key: "pk-agent-1".to_string(),
        trusted: true,
    });
    let worker = agent.register(AgentCapabilities {
        cpu: 8,
        memory: "32GB".to_string(),
        runtimes: vec!["node".to_string(), "python".to_string(), "rust".to_string()],
        supports_wasm: true,
    });

    assert_eq!(worker.id, "agent-1");
    assert_eq!(worker.status, WorkerStatus::Ready);
    assert_eq!(worker.capabilities.cpu_cores, 8);
    assert_eq!(worker.capabilities.memory_mb, 32 * 1024);
    assert!(worker.capabilities.wasm);
    assert_eq!(agent.status, AgentStatus::Idle);

    let heartbeat = agent.heartbeat(12.5, 46.0);
    assert_eq!(heartbeat.agent_id, "agent-1");
    assert_eq!(heartbeat.active_executions, 0);
    assert_eq!(heartbeat.status, AgentStatus::Idle);
    assert_eq!(
        agent.stable_workspace_url("workspace-abc"),
        "https://workspace-abc.trythissoftware.com"
    );
}

#[test]
fn distributed_execution_agent_requires_signed_execution_graphs() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("npm run build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["package.json".to_string()],
            outputs: vec!["dist".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let mut agent = DistributedExecutionAgent::new(AgentIdentity {
        agent_id: "agent-2".to_string(),
        device_fingerprint: "device-abc".to_string(),
        public_key: "pk-agent-2".to_string(),
        trusted: true,
    });
    agent.register(AgentCapabilities {
        cpu: 4,
        memory: "8GB".to_string(),
        runtimes: vec!["node".to_string()],
        supports_wasm: false,
    });

    let signed = SignedExecutionGraph {
        graph: graph.clone(),
        signature: agent.sign_graph(&graph),
    };
    assert!(agent.assign_execution(&signed).is_ok());
    assert_eq!(agent.status, AgentStatus::Running);
    assert_eq!(agent.active_executions, 1);

    let invalid = SignedExecutionGraph {
        graph,
        signature: "bad-signature".to_string(),
    };
    let err = agent
        .assign_execution(&invalid)
        .expect_err("unsigned graph should fail verification");
    assert!(matches!(
        err,
        RuntimeError::CommandFailed(message)
            if message.contains("rejected unsigned execution graph")
    ));
}

#[test]
fn local_agent_provider_participates_in_escalation_and_fails_over() {
    let graph = ExecutionGraph {
        nodes: vec![ExecutionNode {
            id: "build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some("cargo build".to_string()),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["Cargo.toml".to_string()],
            outputs: vec!["target".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        }],
        edges: vec![],
    }
    .with_cache_keys();
    let mut analysis = test_analysis(graph.clone(), WasmCompatibility::Partial, Framework::Rust);
    analysis.execution_profile.runtime_affinity = RuntimeAffinity {
        preferred_provider: "LocalAgentProvider".to_string(),
        fallback_providers: vec!["RustRuntimeProvider".to_string()],
    };
    let runtime_spec = analysis.runtime_spec.clone();
    let compiled_runtime = analysis.compiled_runtime.clone();
    let ctx = ExecutionContext {
        workspace_id: "ws-local-agent-failover".to_string(),
        repo_path: "/tmp/repo".to_string(),
        runtime_spec,
        compiled_runtime,
        analysis,
        execution_graph: graph,
        wasm_sandbox: None,
        resources: ResourceQuotas {
            max_memory_mb: 512,
            max_cpu_millis: 1000,
        },
        network: NetworkPolicy {
            allow_outbound: false,
            allowed_hosts: vec![],
        },
    };
    let mut unavailable_agent = DistributedExecutionAgent::new(AgentIdentity {
        agent_id: "agent-offline".to_string(),
        device_fingerprint: "device-offline".to_string(),
        public_key: "pk-offline".to_string(),
        trusted: false,
    });
    unavailable_agent.register(AgentCapabilities {
        cpu: 4,
        memory: "8GB".to_string(),
        runtimes: vec!["rust".to_string()],
        supports_wasm: false,
    });
    let router = ExecutionRouter::new(vec![
        Box::new(LocalAgentProvider::new(unavailable_agent)),
        Box::new(RustRuntimeProvider),
        Box::new(StaticRuntimeProvider),
    ]);

    let selection = router.select(&ctx).expect("failover should select rust");
    assert_eq!(selection.provider_id, "RustRuntimeProvider");
    assert_eq!(selection.selected_tier, ExecutionTier::ExternalProvider);
    assert!(selection.escalation_trace.iter().any(|step| {
        step.tier == ExecutionTier::LocalMachine
            && step.provider_id.is_none()
            && step.result == "no available provider"
    }));
}

#[test]
fn recovery_manager_migrates_workspace_without_changing_url() {
    let mut registry = WorkspaceRegistry::default();
    let url = stable_workspace_url("z9y8", true);
    registry.upsert(WorkspaceRecord {
        workspace_id: "z9y8".to_string(),
        repository_id: "repo-2".to_string(),
        org_id: "org-2".to_string(),
        created_by: "user-2".to_string(),
        visibility: WorkspaceVisibility::Private,
        execution_id: "exec-2".to_string(),
        assigned_worker: Some("worker-a".to_string()),
        assigned_runtime: RuntimeType::Rust,
        assigned_url: url.clone(),
        state: WorkspaceState::Running,
        created_at: 1,
        updated_at: 1,
        quota: WorkspaceQuota::default(),
    });

    let mut leases = ExecutionLeaseRegistry::default();
    leases.assign("z9y8", "worker-a", 1, 5);
    let recovery = WorkspaceRecoveryManager;
    assert!(recovery.migrate(&mut registry, &mut leases, "z9y8", "worker-b", 10, 10));

    let record = registry.get("z9y8").expect("workspace should exist");
    assert_eq!(record.assigned_worker.as_deref(), Some("worker-b"));
    assert_eq!(record.assigned_url, url);
    assert_eq!(record.state, WorkspaceState::Running);
}

#[test]
fn execution_gateway_routes_by_canonical_execution_url() {
    let mut gateway = ExecutionGateway::default();
    gateway.bind_execution(ExecutionIdentity {
        execution_id: "abc123".to_string(),
        workspace_id: "ws-a".to_string(),
        repository_id: "repo-a".to_string(),
        current_url: "http://worker-a:3000".to_string(),
        canonical_url: ExecutionIdentity::canonical_url_for("abc123"),
        current_tier: ExecutionTier::LocalMachine,
        state: ExecutionState::Running,
    });

    let route = gateway
        .route_request("https://trythissoftware.com/e/abc123", None)
        .expect("canonical route should resolve");
    assert_eq!(route.execution_id, "abc123");
    assert_eq!(route.runtime_url, "http://worker-a:3000");
    assert_eq!(route.canonical_url, "https://trythissoftware.com/e/abc123");
    assert_eq!(route.tier, ExecutionTier::LocalMachine);
}

#[test]
fn execution_rebinding_updates_endpoint_without_changing_canonical_url() {
    let mut resolver = ExecutionUrlResolver::default();
    resolver.upsert(ExecutionIdentity {
        execution_id: "exec-42".to_string(),
        workspace_id: "ws-42".to_string(),
        repository_id: "repo-42".to_string(),
        current_url: "http://local:3000".to_string(),
        canonical_url: ExecutionIdentity::canonical_url_for("exec-42"),
        current_tier: ExecutionTier::LocalMachine,
        state: ExecutionState::Running,
    });
    let mut trace = ExecutionTrace {
        execution_id: "exec-42".to_string(),
        events: vec![],
    };

    let rebound = ExecutionRebindingEngine.rebind(
        &mut resolver,
        &mut trace,
        "exec-42",
        ExecutionTier::DDockitCloud,
        "https://cloud.trythissoftware.com/runtime/42",
    );
    assert!(rebound);

    let identity = resolver
        .get("exec-42")
        .expect("identity should remain bound");
    assert_eq!(
        identity.canonical_url,
        "https://trythissoftware.com/e/exec-42"
    );
    assert_eq!(
        identity.current_url,
        "https://cloud.trythissoftware.com/runtime/42"
    );
    assert_eq!(identity.current_tier, ExecutionTier::DDockitCloud);
    assert!(trace.events.contains(&TraceEvent::ExecutionMigrated {
        from: ExecutionTier::LocalMachine,
        to: ExecutionTier::DDockitCloud
    }));
    assert!(trace.events.contains(&TraceEvent::UrlRebound {
        new_endpoint: "https://cloud.trythissoftware.com/runtime/42".to_string()
    }));
}

#[test]
fn execution_gateway_enforces_session_affinity() {
    let mut gateway = ExecutionGateway::default();
    gateway.bind_execution(ExecutionIdentity {
        execution_id: "exec-affinity".to_string(),
        workspace_id: "ws-affinity".to_string(),
        repository_id: "repo-affinity".to_string(),
        current_url: "http://worker-affinity:3010".to_string(),
        canonical_url: ExecutionIdentity::canonical_url_for("exec-affinity"),
        current_tier: ExecutionTier::ExternalProvider,
        state: ExecutionState::Running,
    });
    gateway.bind_session_affinity(SessionAffinity {
        execution_id: "exec-affinity".to_string(),
        session_id: "session-7".to_string(),
        preferred_provider: "RustRuntimeProvider".to_string(),
    });

    let route = gateway
        .route_request(
            "https://trythissoftware.com/e/exec-affinity",
            Some("session-7"),
        )
        .expect("session-bound canonical route should resolve");
    assert_eq!(route.runtime_url, "http://worker-affinity:3010");
    assert_eq!(
        route.preferred_provider.as_deref(),
        Some("RustRuntimeProvider")
    );
    assert!(gateway
        .route_request(
            "https://trythissoftware.com/e/other-exec",
            Some("session-7")
        )
        .is_none());
}

#[test]
fn capacity_scheduler_prefers_highest_score_under_limit_and_metrics_render() {
    let scheduler = CapacityScheduler {
        max_workspaces_per_worker: 100,
    };
    let selected = scheduler
        .select_worker(&[
            WorkerCapacitySnapshot {
                worker_id: "worker-a".to_string(),
                cpu_available: 800,
                memory_available: 4096,
                workspace_capacity: 95,
            },
            WorkerCapacitySnapshot {
                worker_id: "worker-b".to_string(),
                cpu_available: 700,
                memory_available: 8192,
                workspace_capacity: 80,
            },
            WorkerCapacitySnapshot {
                worker_id: "worker-c".to_string(),
                cpu_available: 900,
                memory_available: 8192,
                workspace_capacity: 101,
            },
        ])
        .expect("one worker should be schedulable");
    assert_eq!(selected, "worker-b");

    let metrics = WorkspaceMetrics {
        active_workspaces: 100,
        failed_workspaces: 2,
        workspace_restarts: 7,
        migration_count: 3,
        router_latency: 1.5,
        worker_utilization: 0.72,
        warm_pool_hits: 44,
        cold_start_fallbacks: 2,
        image_match_confidence: 96.5,
        cache_hit_ratio: 0.91,
        execution_start_latency: 3.4,
        commit_execution_success_rate: 0.85,
        fallback_depth_distribution: 1.2,
        last_known_good_distance: 2.0,
        commit_cache_hit_rate: 0.7,
    };
    let (path, body) = metrics_endpoint(&metrics);
    assert_eq!(path, "/metrics");
    assert!(body.contains("active_workspaces 100"));
    assert!(body.contains("worker_utilization 0.72"));
    assert!(body.contains("warm_pool_hits 44"));
    assert!(body.contains("cache_hit_ratio 0.91"));
}

#[test]
fn execution_api_layer_endpoints_emit_expected_payloads() {
    let repo = temp_dir("execution-api");
    fs::create_dir_all(repo.join(".ddockit")).expect("create .ddockit");
    fs::write(
        repo.join(".ddockit/ddockit.yaml"),
        r#"
version: 1
services:
  backend:
    runtime: rust
    run:
      - cargo run
    port: 8080
"#,
    )
    .expect("write ddockit spec");
    let analysis = analyze_repository(&repo).expect("analyze repo");

    let (analyze_path, analyze_body) = repositories_analyze_endpoint(
        &RepositoryAnalyzeRequest {
            repo_url: "https://github.com/example/app".to_string(),
        },
        &analysis,
    );
    assert_eq!(analyze_path, "/api/v1/repositories/analyze");
    assert!(analyze_body.contains("\"fingerprint_id\""));
    assert!(analyze_body.contains("\"services\":[\"backend\"]"));
    assert!(analyze_body.contains("\"preflight\""));
    assert!(analyze_body.contains("\"environment_confidence\""));

    let (plan_path, plan_body) = execution_plan_endpoint(&analysis);
    assert_eq!(plan_path, "/api/v1/execution/plan");
    assert!(plan_body.contains("\"execution_plan_id\""));
    assert!(plan_body.contains("\"startup_order\":[\"backend\"]"));
    assert!(plan_body.contains("\"preflight\""));

    let (start_path, start_body) = executions_start_endpoint(&ExecutionStartRequest {
        org_id: Some("org-1".to_string()),
        user_id: Some("user-1".to_string()),
        anon_user_id: None,
        anon_session_id: None,
        device_fingerprint: None,
        repo_url: "https://github.com/example/app".to_string(),
        branch: Some("main".to_string()),
        commit: None,
    });
    assert_eq!(start_path, "/api/v1/executions");
    assert!(start_body.contains("\"org_id\":\"org-1\""));
    assert!(start_body.contains("\"user_id\":\"user-1\""));
    assert!(start_body.contains("\"identity_type\":\"authenticated\""));
    assert!(start_body.contains("\"status\":\"starting\""));
    assert!(start_body.contains("\"workspace_url\":\"https://workspace-"));

    let (workspace_create_path, workspace_create_body) =
        workspace_create_endpoint(&WorkspaceCreateRequest {
            repository_id: "repo-1".to_string(),
            commit_hash: "aaaaaaa".to_string(),
            org_id: "org-1".to_string(),
            created_by: "user-1".to_string(),
            visibility: WorkspaceVisibility::Private,
        });
    assert_eq!(workspace_create_path, "/workspaces");
    assert!(workspace_create_body.contains("\"org_id\":\"org-1\""));
    assert!(workspace_create_body.contains("\"workspace_url\":\"workspace-"));

    let mut workspace_router = WorkspaceRouter::default();
    let workspace = workspace_router.create_workspace("repo-1", "aaaaaaa", "org-1", "user-1", 1);
    let (workspace_resolve_path, workspace_resolve_body) =
        workspace_resolve_endpoint(&workspace.workspace_id, &workspace_router);
    assert_eq!(
        workspace_resolve_path,
        format!("/workspaces/{}", workspace.workspace_id)
    );
    assert!(workspace_resolve_body.contains("\"workspace_id\""));
    assert!(workspace_resolve_body.contains("\"org_id\":\"org-1\""));

    let (workspace_bind_path, workspace_bind_body) = workspace_bind_endpoint(
        &workspace.workspace_id,
        &WorkspaceRuntimeRequest {
            runtime_type: "DEA".to_string(),
            runtime_instance_id: "dea-1".to_string(),
            endpoint: "http://dea-1:3012".to_string(),
            lease_expires_at: 10,
        },
    );
    assert_eq!(
        workspace_bind_path,
        format!("/workspaces/{}/bind", workspace.workspace_id)
    );
    assert!(workspace_bind_body.contains("\"runtime_type\":\"DEA\""));

    let (workspace_migrate_path, workspace_migrate_body) = workspace_migrate_endpoint(
        &workspace.workspace_id,
        &WorkspaceRuntimeRequest {
            runtime_type: "CLOUD".to_string(),
            runtime_instance_id: "cloud-1".to_string(),
            endpoint: "https://cloud-1.trythissoftware.com".to_string(),
            lease_expires_at: 20,
        },
    );
    assert_eq!(
        workspace_migrate_path,
        format!("/workspaces/{}/migrate", workspace.workspace_id)
    );
    assert!(workspace_migrate_body.contains("\"runtime_type\":\"CLOUD\""));

    let (status_path, status_body) = execution_status_endpoint("exec-1");
    assert_eq!(status_path, "/api/v1/executions/exec-1");
    assert!(status_body.contains("\"health\":\"healthy\""));

    let (logs_path, logs_body) = execution_logs_endpoint("exec-1", &["line1".to_string()]);
    assert_eq!(logs_path, "/api/v1/executions/exec-1/logs");
    assert!(logs_body.contains("\"logs\":[\"line1\"]"));

    let (restart_path, restart_body) = execution_restart_endpoint("exec-1");
    assert_eq!(restart_path, "/api/v1/executions/exec-1/restart");
    assert!(restart_body.contains("\"status\":\"restarting\""));

    let (stop_path, stop_body) = execution_stop_endpoint("exec-1");
    assert_eq!(stop_path, "/api/v1/executions/exec-1/stop");
    assert!(stop_body.contains("\"status\":\"stopped\""));

    let (migrate_path, migrate_body) = execution_migrate_endpoint(
        "exec-1",
        &ExecutionMigrateRequest {
            target: "cloud".to_string(),
        },
    );
    assert_eq!(migrate_path, "/api/v1/executions/exec-1/migrate");
    assert!(migrate_body.contains("\"target\":\"cloud\""));

    let (claim_path, claim_body) = execution_claim_endpoint(
        "exec-1",
        &ExecutionClaimRequest {
            anon_user_id: "anon-1".to_string(),
            user_id: "user-1".to_string(),
            org_id: Some("org-1".to_string()),
        },
    );
    assert_eq!(claim_path, "/api/v1/executions/exec-1/claim");
    assert!(claim_body.contains("\"status\":\"claimed\""));

    let (workspace_list_path, workspace_list_body) =
        workspaces_list_endpoint("org-1", &workspace_router);
    assert_eq!(workspace_list_path, "/workspaces?org_id=org-1");
    assert!(workspace_list_body.contains("\"workspace_id\""));

    let (workspace_delete_path, workspace_delete_body) =
        workspace_delete_endpoint(&workspace.workspace_id, "org-1");
    assert_eq!(
        workspace_delete_path,
        format!("/workspaces/{}", workspace.workspace_id)
    );
    assert!(workspace_delete_body.contains("\"status\":\"deleted\""));
}

#[test]
fn preflight_intelligence_synthesizes_environment_and_failure_predictions() {
    let repo = temp_dir("preflight-intelligence");
    fs::write(
            repo.join("package.json"),
            r#"{"dependencies":{"next":"14.2.0","prisma":"5.0.0","openai":"4.0.0"},"scripts":{"dev":"next dev","build":"next build"}}"#,
        )
        .expect("write package.json");
    fs::write(repo.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").expect("write pnpm lock");
    fs::write(
        repo.join(".env.example"),
        "NODE_ENV=development\nPORT=3000\nHOST=0.0.0.0\n",
    )
    .expect("write env example");
    fs::create_dir_all(repo.join("prisma")).expect("create prisma");
    fs::write(
        repo.join("prisma/schema.prisma"),
        "datasource db { provider = \"sqlite\" }\n",
    )
    .expect("write prisma schema");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let (_, analyze_body) = repositories_analyze_endpoint(
        &RepositoryAnalyzeRequest {
            repo_url: "https://github.com/example/preflight".to_string(),
        },
        &analysis,
    );
    let payload: Value = serde_json::from_str(&analyze_body).expect("parse analyze payload");
    let preflight = payload
        .get("preflight")
        .and_then(Value::as_object)
        .expect("preflight payload");

    assert_eq!(
        preflight
            .get("pipeline")
            .and_then(Value::as_array)
            .expect("pipeline")
            .len(),
        16
    );
    assert!(preflight
        .get("pipeline")
        .and_then(Value::as_array)
        .expect("pipeline")
        .iter()
        .any(|entry| entry.as_str() == Some("execution-preparation")));
    assert!(preflight
        .get("pipeline")
        .and_then(Value::as_array)
        .expect("pipeline")
        .iter()
        .any(|entry| entry.as_str() == Some("execution-specification")));
    assert!(preflight
        .get("execution_specification")
        .and_then(Value::as_object)
        .is_some());
    assert!(preflight
        .get("portable_execution_toml")
        .and_then(Value::as_str)
        .is_some_and(|toml| toml.contains("[runtime]")));
    assert!(preflight
        .get("execution_lock")
        .and_then(Value::as_str)
        .is_some_and(|lock| lock.contains("runtime_hash = \"sha256:")));
    assert!(preflight
        .get("runtime_graph_json")
        .and_then(Value::as_object)
        .is_some());
    assert!(preflight
        .get("capabilities_toml")
        .and_then(Value::as_str)
        .is_some_and(|caps| caps.contains("[capabilities]")));
    assert!(preflight
        .get("environment_schema_json")
        .and_then(Value::as_object)
        .is_some());
    assert!(preflight
        .get("provenance_json")
        .and_then(Value::as_object)
        .is_some());
    assert!(preflight
        .get("healing_patch")
        .and_then(Value::as_str)
        .is_some());
    assert!(preflight
        .get("execution_fingerprint")
        .and_then(Value::as_str)
        .is_some_and(|fingerprint| fingerprint.starts_with("sha256:")));
    assert!(preflight
        .get("environment_graph")
        .and_then(Value::as_array)
        .expect("environment graph")
        .iter()
        .any(|entry| {
            entry.get("name").and_then(Value::as_str) == Some("DATABASE_URL")
                && entry.get("value_source").and_then(Value::as_str) == Some("synthesized")
        }));
    assert!(preflight
        .get("simulation")
        .and_then(Value::as_object)
        .and_then(|simulation| simulation.get("expected_failures"))
        .and_then(Value::as_array)
        .expect("expected failures")
        .iter()
        .any(|entry| {
            entry.get("failure").and_then(Value::as_str) == Some("Missing DATABASE_URL")
        }));
    assert!(preflight
        .get("environment_confidence")
        .and_then(Value::as_object)
        .and_then(|confidence| confidence.get("expected_success"))
        .and_then(Value::as_u64)
        .is_some());
}

#[test]
fn preflight_intelligence_prefers_discovered_execution_specification_before_deriving() {
    let repo = temp_dir("preflight-intelligence-discovery");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"next":"14.2.0"},"scripts":{"dev":"next dev","build":"next build"}}"#,
    )
    .expect("write package.json");
    fs::write(
        repo.join("execution.toml"),
        r#"version = "1"
[runtime]
language = "typescript"
[dependencies]
all = ["next"]
[environment]
NODE_ENV = "development"
[capabilities]
all = ["network"]
"#,
    )
    .expect("write execution.toml");
    fs::write(
        repo.join("execution.json"),
        r#"{"version":"1","runtime":{"language":"javascript"}}"#,
    )
    .expect("write execution.json");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let (_, analyze_body) = repositories_analyze_endpoint(
        &RepositoryAnalyzeRequest {
            repo_url: "https://github.com/example/preflight-discovery".to_string(),
        },
        &analysis,
    );
    let payload: Value = serde_json::from_str(&analyze_body).expect("parse analyze payload");
    let preflight = payload
        .get("preflight")
        .and_then(Value::as_object)
        .expect("preflight payload");

    let discovery = preflight
        .get("execution_specification_discovery")
        .and_then(Value::as_object)
        .expect("discovery payload");
    assert_eq!(
        discovery.get("path").and_then(Value::as_str),
        Some("execution.toml")
    );
    assert_eq!(
        discovery.get("decision").and_then(Value::as_str),
        Some("validate")
    );
    assert_eq!(
        discovery
            .get("used_fallback_derivation")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert!(preflight
        .get("portable_execution_toml")
        .and_then(Value::as_str)
        .is_some_and(|toml| toml.contains("[runtime]")));
    assert!(preflight
        .get("execution_lock")
        .and_then(Value::as_str)
        .is_some_and(|lock| lock.contains("execution_fingerprint = \"sha256:")));
    assert!(preflight
        .get("execution_fingerprint")
        .and_then(Value::as_str)
        .is_some_and(|fingerprint| fingerprint.starts_with("sha256:")));
}

#[test]
fn ddockit_publish_endpoint_emits_runtime_locked_artifacts() {
    let repo = temp_dir("ddockit-publish-artifacts");
    fs::write(
        repo.join("package.json"),
        r#"{"dependencies":{"next":"14.2.0"},"scripts":{"build":"next build"}}"#,
    )
    .expect("write package.json");
    fs::write(repo.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").expect("write pnpm lock");

    let analysis = analyze_repository(&repo).expect("analyze repo");
    let (path, body) = ddockit_publish_endpoint(&analysis);
    assert_eq!(path, "/api/v1/repositories/publish");
    assert!(body.contains("\"status\":\"repository_ready\""));
    assert!(body.contains("\"execution.lock\""));
    assert!(body.contains("\"runtime.graph.json\""));
    assert!(body.contains("\"provenance.json\""));
    assert!(body.contains("\"execution_fingerprint\":\"sha256:"));
    assert!(body.contains("\"open_pull_request\""));
}

#[test]
fn badge_seed_endpoints_emit_runtime_state_and_seed_pipeline_payloads() {
    let (badge_path, badge_body) = badge_svg_endpoint(
        "octocat",
        "hello-world",
        &BadgeExecutionSnapshot {
            health_score: 98.5,
            execution_readiness: 0.92,
            last_run_status: "success".to_string(),
            has_execution_history: true,
            healed_artifact_available: false,
        },
    );
    assert_eq!(badge_path, "/badge/octocat/hello-world.svg");
    assert!(badge_body.contains("<svg"));
    assert!(badge_body.contains("🟢 Production Ready"));
    assert!(badge_body.contains("octocat/hello-world"));

    let (healed_path, healed_body) = healed_badge_variant_endpoint("octocat", "hello-world");
    assert_eq!(healed_path, "/badge/healed/octocat/hello-world.svg");
    assert!(healed_body.contains("🔵 Healed"));

    let (seed_path, seed_body) = badge_seed_launch_endpoint("octocat", "hello-world", None);
    assert_eq!(seed_path, "/seed/octocat/hello-world");
    assert!(seed_body.contains("\"entrypoint\":\"readme_badge\""));
    assert!(seed_body.contains("\"analyze_endpoint\":\"/api/v1/repositories/analyze\""));
    assert!(seed_body.contains("\"ownership_transfer\""));
    assert!(seed_body.contains("\"workspace_url\":\"https://workspace-"));
    let seed_payload: Value = serde_json::from_str(&seed_body).expect("seed payload json");
    let execution_start_endpoint = seed_payload
        .get("pipeline")
        .and_then(Value::as_object)
        .and_then(|pipeline| pipeline.get("execution_start_endpoint"))
        .and_then(Value::as_str)
        .expect("seed pipeline should include execution start endpoint");
    assert_eq!(execution_start_endpoint, "/api/v1/executions");
}

#[test]
fn badge_generator_endpoint_emits_embed_snippets_and_variants() {
    let request = BadgeGenerateRequest {
        repo_url: "https://github.com/vercel/next.js".to_string(),
        branch: Some("canary".to_string()),
        mode: Some("wasm".to_string()),
        visibility: Some("private".to_string()),
    };
    let (path, body) = badge_generate_endpoint(&request);
    assert_eq!(path, "/api/badges/generate");

    let payload: Value = serde_json::from_str(&body).expect("badge payload json");
    assert_eq!(
        payload
            .get("repo")
            .and_then(Value::as_object)
            .and_then(|repo| repo.get("owner"))
            .and_then(Value::as_str),
        Some("vercel")
    );
    assert_eq!(
        payload
            .get("repo")
            .and_then(Value::as_object)
            .and_then(|repo| repo.get("name"))
            .and_then(Value::as_str),
        Some("next.js")
    );
    assert_eq!(
        payload
            .get("repo")
            .and_then(Value::as_object)
            .and_then(|repo| repo.get("branch"))
            .and_then(Value::as_str),
        Some("canary")
    );
    assert_eq!(
        payload
            .get("config")
            .and_then(Value::as_object)
            .and_then(|config| config.get("mode"))
            .and_then(Value::as_str),
        Some("wasm")
    );
    assert_eq!(
        payload
            .get("config")
            .and_then(Value::as_object)
            .and_then(|config| config.get("visibility"))
            .and_then(Value::as_str),
        Some("private")
    );
    assert_eq!(
        payload.get("badge_url").and_then(Value::as_str),
        Some("https://api.trythissoftware.com/badge/vercel/next.js.svg")
    );
    assert_eq!(
        payload.get("seed_url").and_then(Value::as_str),
        Some("https://trythissoftware.com/seed/vercel/next.js")
    );
    assert_eq!(
            payload
                .get("embed_snippets")
                .and_then(Value::as_object)
                .and_then(|snippets| snippets.get("markdown"))
                .and_then(Value::as_str),
            Some(
                "[<img src=\"https://api.trythissoftware.com/badge/vercel/next.js.svg\" alt=\"vercel/next.js execution status badge\">](https://trythissoftware.com/seed/vercel/next.js)"
            )
        );
    assert_eq!(
        payload.get("auto_update_notice").and_then(Value::as_str),
        Some("This badge updates automatically based on repository execution health.")
    );
}

#[test]
fn badge_generator_endpoint_rejects_non_github_urls() {
    let request = BadgeGenerateRequest {
        repo_url: "https://example.com/not-github/repo".to_string(),
        branch: None,
        mode: None,
        visibility: None,
    };
    let (path, body) = badge_generate_endpoint(&request);
    assert_eq!(path, "/api/badges/generate");
    assert!(body.contains("\"error\":\"invalid_github_repo_url\""));
}

#[test]
fn auth_and_org_endpoints_emit_org_scoped_identity_payloads() {
    let user = UserIdentity {
        user_id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        name: "User One".to_string(),
        auth_provider: AuthProvider::Github,
        created_at: 1,
    };
    let (login_path, login_body) = auth_login_endpoint(&AuthLoginRequest {
        user: user.clone(),
        org_id: "org-1".to_string(),
        role: MembershipRole::Admin,
    });
    assert_eq!(login_path, "/auth/login");
    assert!(login_body.contains("\"org_id\":\"org-1\""));
    assert!(login_body.contains("workspace_create"));

    let claims = AuthClaims {
        user_id: "user-1".to_string(),
        org_id: "org-1".to_string(),
        role: MembershipRole::Admin,
        permissions: RbacPolicyEngine::role_permissions(MembershipRole::Admin),
    };
    let context = RbacPolicyEngine::authorize(
        &claims,
        "org-1",
        &[Permission::WorkspaceDelete, Permission::OrgAdmin],
    )
    .expect("admin should be authorized");
    assert!(RbacPolicyEngine::authorize(&claims, "org-2", &[Permission::OrgAdmin]).is_none());

    let (me_path, me_body) = auth_me_endpoint(&context);
    assert_eq!(me_path, "/auth/me");
    assert!(me_body.contains("\"role\":\"admin\""));

    let (logout_path, logout_body) = auth_logout_endpoint(&context);
    assert_eq!(logout_path, "/auth/logout");
    assert!(logout_body.contains("\"status\":\"logged_out\""));

    let (org_create_path, org_create_body) = org_create_endpoint(&OrganizationCreateRequest {
        name: "Org One".to_string(),
        slug: "org-one".to_string(),
        plan: OrganizationPlan::Pro,
        created_by: user.user_id.clone(),
    });
    assert_eq!(org_create_path, "/orgs");
    assert!(org_create_body.contains("\"plan\":\"pro\""));

    let org = OrganizationIdentity {
        org_id: "org-1".to_string(),
        name: "Org One".to_string(),
        slug: "org-one".to_string(),
        plan: OrganizationPlan::Pro,
        created_at: 1,
    };
    let (org_get_path, org_get_body) = org_get_endpoint(&org);
    assert_eq!(org_get_path, "/orgs/org-1");
    assert!(org_get_body.contains("\"org_id\":\"org-1\""));

    let (member_path, member_body) =
        org_add_member_endpoint(&OrganizationMembershipCreateRequest {
            org_id: "org-1".to_string(),
            user_id: "user-2".to_string(),
            role: MembershipRole::Developer,
        });
    assert_eq!(member_path, "/orgs/org-1/members");
    assert!(member_body.contains("\"role\":\"developer\""));
}

#[test]
fn oauth_callback_endpoints_emit_token_exchange_identity_org_and_redirect_payloads() {
    let (github_path, github_body) = github_oauth_callback_endpoint(&GithubOAuthCallbackRequest {
        code: "github-code".to_string(),
        state: Some("state-1".to_string()),
        extension_id: None,
        github_id: 123_456,
        github_login: "octocat".to_string(),
        github_email: Some("octocat@github.com".to_string()),
        existing_user_id: None,
        existing_org_id: None,
        role: MembershipRole::Admin,
    });
    assert_eq!(github_path, "/auth/github/callback");
    assert!(github_body.contains("\"url\":\"https://github.com/login/oauth/access_token\""));
    assert!(github_body.contains("\"url\":\"https://api.github.com/user\""));
    assert!(github_body.contains("\"name\":\"octocat-org\""));
    assert!(github_body.contains("\"provider\":\"github\""));
    assert!(github_body.contains("https://trythissoftware.com/auth/success?token="));

    let (google_path, google_body) = google_oauth_callback_endpoint(&GoogleOAuthCallbackRequest {
        code: "google-code".to_string(),
        state: Some("state-2".to_string()),
        extension_id: Some("abcdefghijklmnop".to_string()),
        google_sub: "google-sub-1".to_string(),
        google_email: "person@example.com".to_string(),
        google_name: "Person One".to_string(),
        existing_user_id: Some("user-existing".to_string()),
        existing_org_id: Some("org-existing".to_string()),
        role: MembershipRole::Developer,
    });
    assert_eq!(google_path, "/auth/google/callback");
    assert!(google_body.contains("\"url\":\"https://oauth2.googleapis.com/token\""));
    assert!(google_body.contains("\"url\":\"https://openidconnect.googleapis.com/v1/userinfo\""));
    assert!(google_body.contains("\"status\":\"existing\""));
    assert!(google_body.contains("\"provider\":\"google\""));
    assert!(google_body.contains("chrome-extension://abcdefghijklmnop/auth/success?token="));
    assert!(google_body.contains("\"org_id\":\"org-existing\""));
}

#[test]
fn executions_list_endpoint_filters_by_org() {
    let executions = vec![
        EidbExecutionRecord {
            execution_id: "exec-1".to_string(),
            org_id: Some("org-1".to_string()),
            user_id: Some("user-1".to_string()),
            anon_user_id: None,
            workspace_id: "ws-1".to_string(),
            repository_id: "repo".to_string(),
            commit_hash: "aaaaaaa".to_string(),
            started_at: 1,
            completed_at: None,
            status: "running".to_string(),
            execution_tier: "DEA".to_string(),
        },
        EidbExecutionRecord {
            execution_id: "exec-2".to_string(),
            org_id: Some("org-2".to_string()),
            user_id: Some("user-2".to_string()),
            anon_user_id: None,
            workspace_id: "ws-2".to_string(),
            repository_id: "repo".to_string(),
            commit_hash: "bbbbbbb".to_string(),
            started_at: 1,
            completed_at: None,
            status: "running".to_string(),
            execution_tier: "CLOUD".to_string(),
        },
    ];

    let (path, body) = executions_list_endpoint("org-1", &executions);
    assert_eq!(path, "/executions?org_id=org-1");
    assert!(body.contains("\"execution_id\":\"exec-1\""));
    assert!(!body.contains("\"execution_id\":\"exec-2\""));
}

#[test]
fn identity_merge_engine_claims_anonymous_execution_history() {
    let mut database = ExecutionIntelligenceDatabase::default();
    database.record_execution(EidbExecutionRecord {
        execution_id: "exec-anon-1".to_string(),
        org_id: None,
        user_id: None,
        anon_user_id: Some("anon-user-1".to_string()),
        workspace_id: "ws-anon-1".to_string(),
        repository_id: "repo".to_string(),
        commit_hash: "aaaaaaa".to_string(),
        started_at: 1,
        completed_at: None,
        status: "running".to_string(),
        execution_tier: "DEA".to_string(),
    });

    let engine = IdentityMergeEngine;
    let merged =
        engine.claim_anonymous_executions(&mut database, "anon-user-1", "user-1", Some("org-1"));
    assert_eq!(merged, 1);
    assert_eq!(database.executions[0].user_id.as_deref(), Some("user-1"));
    assert_eq!(database.executions[0].org_id.as_deref(), Some("org-1"));
    assert_eq!(
        database.executions[0].anon_user_id.as_deref(),
        Some("anon-user-1")
    );
}

#[test]
fn dual_surface_contract_uses_single_execution_api_and_control_plane() {
    let (path, body) = dual_surface_experience_contract_endpoint();
    assert_eq!(path, "/api/v1/dual-surface/contract");
    assert!(body.contains("\"github_overlay_extension\""));
    assert!(body.contains("\"portal\""));
    assert!(body.contains("\"execution_api\":\"/api/v1/executions\""));
    assert!(body.contains("\"control_plane\":\"unified\""));
    assert!(body.contains("\"same_execution_ids\""));
    assert!(body.contains("\"same_urls\""));
    assert!(body.contains("\"same_state\""));
    assert!(body.contains("\"ui_endpoint\":\"/api/v1/surfaces/extension/ui\""));
    assert!(body.contains("\"ui_endpoint\":\"/api/v1/surfaces/portal/ui\""));
}

#[test]
fn overlay_repository_detection_extracts_owner_repo_and_branch() {
    let context = detect_overlay_repository_context("https://github.com/org/repo")
        .expect("github URL should parse");
    assert_eq!(context.owner, "org");
    assert_eq!(context.repo, "repo");
    assert_eq!(context.branch, "main");

    let branch_context =
        detect_overlay_repository_context("https://github.com/org/repo/tree/release")
            .expect("github URL with branch should parse");
    assert_eq!(branch_context.branch, "release");

    let nested_branch_context =
        detect_overlay_repository_context("https://github.com/org/repo/tree/feature/ui")
            .expect("github URL with nested branch should parse");
    assert_eq!(nested_branch_context.branch, "feature/ui");

    assert!(detect_overlay_repository_context("https://github.com/org/repo/tree").is_none());
}

#[test]
fn extension_and_portal_execution_starts_share_ids_and_urls() {
    let request = ExecutionStartRequest {
        org_id: Some("org-1".to_string()),
        user_id: Some("user-1".to_string()),
        anon_user_id: None,
        anon_session_id: None,
        device_fingerprint: None,
        repo_url: "https://github.com/example/app".to_string(),
        branch: Some("main".to_string()),
        commit: None,
    };
    let (extension_path, extension_body) =
        surface_execution_start_endpoint(ProductSurface::GitHubOverlayExtension, &request);
    let (portal_path, portal_body) =
        surface_execution_start_endpoint(ProductSurface::Portal, &request);

    assert_eq!(extension_path, "/api/v1/executions");
    assert_eq!(portal_path, "/api/v1/executions");

    let extension_payload: serde_json::Value =
        serde_json::from_str(&extension_body).expect("extension payload json");
    let portal_payload: serde_json::Value =
        serde_json::from_str(&portal_body).expect("portal payload json");

    assert_eq!(
        extension_payload
            .get("execution_id")
            .and_then(serde_json::Value::as_str),
        portal_payload
            .get("execution_id")
            .and_then(serde_json::Value::as_str)
    );
    assert_eq!(
        extension_payload
            .get("workspace_url")
            .and_then(serde_json::Value::as_str),
        portal_payload
            .get("workspace_url")
            .and_then(serde_json::Value::as_str)
    );
}

#[test]
fn executions_start_endpoint_supports_anonymous_identity() {
    let (path, body) = executions_start_endpoint(&ExecutionStartRequest {
        org_id: None,
        user_id: None,
        anon_user_id: Some("anon-1".to_string()),
        anon_session_id: Some("session-1".to_string()),
        device_fingerprint: Some("browser-chrome:extension-a".to_string()),
        repo_url: "https://github.com/example/app".to_string(),
        branch: Some("main".to_string()),
        commit: None,
    });

    assert_eq!(path, "/api/v1/executions");
    assert!(body.contains("\"anon_user_id\":\"anon-1\""));
    assert!(body.contains("\"identity_type\":\"anonymous\""));
    assert!(body.contains("\"claim_workspace_prompt\":true"));
}

#[test]
fn dual_surface_endpoints_expose_extension_actions_and_portal_navigation() {
    let (extension_path, extension_body) = extension_overlay_actions_endpoint();
    assert_eq!(extension_path, "/api/v1/surfaces/extension/actions");
    assert!(extension_body.contains("\"run\""));
    assert!(extension_body.contains("\"instant_run\""));
    assert!(extension_body.contains("\"ask_repository\""));
    assert!(extension_body.contains("\"run_entrypoint\":\"/api/v1/executions\""));
    assert!(extension_body.contains("\"ui_endpoint\":\"/api/v1/surfaces/extension/ui\""));

    let (portal_path, portal_body) = portal_navigation_endpoint();
    assert_eq!(portal_path, "/api/v1/surfaces/portal/navigation");
    assert!(portal_body.contains("\"dashboard\""));
    assert!(portal_body.contains("\"organization\""));
    assert!(portal_body.contains("\"members\""));
    assert!(portal_body.contains("\"workspaces\""));
    assert!(portal_body.contains("\"billing\""));
    assert!(portal_body.contains("\"org_switcher\""));
    assert!(portal_body.contains("\"workspace_path\":\"/api/v1/executions/{id}\""));
    assert!(portal_body.contains("\"ui_endpoint\":\"/api/v1/surfaces/portal/ui\""));
    assert!(portal_body.contains("\"publish_api\":\"/api/v1/repositories/publish\""));
}

#[test]
fn dual_surface_ui_endpoints_expose_actual_surface_layouts() {
    let (extension_ui_path, extension_ui_body) = extension_overlay_ui_endpoint();
    assert_eq!(extension_ui_path, "/api/v1/surfaces/extension/ui");
    assert!(extension_ui_body.contains("\"view\":\"overlay_panel\""));
    assert!(extension_ui_body.contains("\"shell\":\"github_overlay_shell\""));
    assert!(extension_ui_body.contains("\"quick_actions\""));
    assert!(extension_ui_body.contains("\"latest_execution\""));
    assert!(extension_ui_body.contains("\"component_registry\""));
    assert!(extension_ui_body.contains("\"rendered\""));
    assert!(extension_ui_body.contains("\"screenshot\""));
    assert!(extension_ui_body.contains("\"shape\":\"orb\""));
    assert!(extension_ui_body.contains("\"animation\":\"pulse\""));
    assert!(extension_ui_body.contains("\"when_repository_detected\":\"pulse\""));

    let (portal_ui_path, portal_ui_body) = portal_ui_endpoint();
    assert_eq!(portal_ui_path, "/api/v1/surfaces/portal/ui");
    assert!(portal_ui_body.contains("\"layout\""));
    assert!(portal_ui_body.contains("\"shell\":\"portal_shell\""));
    assert!(portal_ui_body.contains("\"dashboard\""));
    assert!(portal_ui_body.contains("\"workspaces\""));
    assert!(portal_ui_body.contains("\"executions\""));
    assert!(portal_ui_body.contains("\"agents\""));
    assert!(portal_ui_body.contains("\"badge_generator_studio\""));
    assert!(portal_ui_body.contains("\"generate_api\":\"/api/badges/generate\""));
    assert!(portal_ui_body.contains("\"repository_ready_publish\""));
    assert!(portal_ui_body.contains("\"publish_api\":\"/api/v1/repositories/publish\""));
    assert!(portal_ui_body.contains(
        "\"notice\":\"This badge updates automatically based on repository execution health.\""
    ));
    assert!(portal_ui_body.contains("\"component_registry\""));
    assert!(portal_ui_body.contains("\"rendered\""));
}

#[test]
fn surface_renderer_maps_contract_components_to_shared_design_system() {
    let components = vec![
        json!({"id": "workspace_card", "type": "card"}),
        json!({"id": "execution_table", "type": "table"}),
        json!({"id": "log_stream", "type": "log_stream"}),
        json!({"id": "topology_graph", "type": "topology"}),
        json!({"id": "health", "type": "status_indicator"}),
    ];
    let rendered = render_surface_view("workspace", &components);
    let rendered_components = rendered
        .get("components")
        .and_then(serde_json::Value::as_array)
        .expect("rendered components");
    assert_eq!(
        rendered.get("renderer"),
        Some(&json!("unified_surface_renderer"))
    );
    assert!(rendered_components
        .iter()
        .any(|entry| entry.get("component") == Some(&json!("Card"))));
    assert!(rendered_components
        .iter()
        .any(|entry| entry.get("component") == Some(&json!("Table"))));
    assert!(rendered_components
        .iter()
        .any(|entry| entry.get("component") == Some(&json!("LogsViewer"))));
    assert!(rendered_components
        .iter()
        .any(|entry| entry.get("component") == Some(&json!("TopologyGraph"))));
    assert!(rendered_components
        .iter()
        .any(|entry| entry.get("component") == Some(&json!("StatusIndicator"))));
}

#[test]
fn golden_repository_catalog_loads_required_framework_categories() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden_repos")
        .join("catalog.yaml");
    let catalog = load_golden_repository_catalog(&path).expect("load golden repository catalog");
    assert_eq!(catalog.schema_version, "2");
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.category == "node"));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.category == "python"));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.category == "rust"));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.category == "go"));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.category == "bun"));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.category == "monorepo"));
    assert!(catalog
        .repositories
        .iter()
        .all(|repo| is_pinned_commit(&repo.commit)));
    assert!(catalog
        .repositories
        .iter()
        .all(|repo| !repo.execution_profile.is_empty()));
    assert!(catalog
        .repositories
        .iter()
        .all(|repo| !repo.certification.last_verified.is_empty()));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.id == "nextjs-blog"));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.id == "fastapi-tutorial"));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.id == "django-polls"));
    assert!(catalog
        .repositories
        .iter()
        .any(|repo| repo.id == "axum-example"));
}

#[test]
fn customer_journey_runner_executes_default_suite_with_url_validation() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden_repos")
        .join("catalog.yaml");
    let catalog = load_golden_repository_catalog(&path).expect("load golden repository catalog");
    let runner = CustomerJourneyRunner::new(catalog);
    let results = runner.run_default_suite();
    assert_eq!(results.len(), 12);
    assert!(results.iter().all(|result| result.analysis_success));
    assert!(results.iter().all(|result| result.plan_success));
    assert!(results.iter().all(|result| result.runtime_success));
    assert!(results.iter().all(|result| result.url_success));
    assert!(results.iter().all(|result| result.health_success));

    let fastapi = results
        .iter()
        .find(|result| result.repository_name == "fastapi-tutorial")
        .expect("journey should include fastapi");
    assert!(fastapi
        .route_checks
        .iter()
        .any(|check| check.route == "/docs" && check.status_code == 200));

    let django = results
        .iter()
        .find(|result| result.repository_name == "django-polls")
        .expect("journey should include django");
    assert!(django
        .route_checks
        .iter()
        .any(|check| check.route == "/admin" && check.status_code == 200));

    let fallback = results
        .iter()
        .find(|result| result.journey_kind == CustomerJourneyKind::BrokenHeadCommitFallback)
        .expect("suite should include broken-head fallback journey");
    assert!(fallback.fallback_commit_success);

    let healing = results
        .iter()
        .find(|result| result.journey_kind == CustomerJourneyKind::HealingRepairAndRetry)
        .expect("suite should include healing journey");
    assert!(healing.healing_success);

    let migration = results
        .iter()
        .find(|result| result.journey_kind == CustomerJourneyKind::RuntimeMigrationWithoutUrlChange)
        .expect("suite should include runtime migration journey");
    assert!(migration.runtime_migration_preserved_url);

    let frontend = results
        .iter()
        .find(|result| result.journey_kind == CustomerJourneyKind::PortalFrontendJourney)
        .expect("suite should include portal frontend journey");
    assert!(frontend
        .route_checks
        .iter()
        .any(|check| check.route == "/" && check.status_code == 200));

    let extension = results
        .iter()
        .find(|result| result.journey_kind == CustomerJourneyKind::BrowserExtensionOverlayJourney)
        .expect("suite should include browser extension journey");
    assert!(extension.route_checks.iter().any(|check| {
        check.route == "/api/v1/surfaces/extension/ui" && check.status_code == 200
    }));

    let metrics = compute_customer_journey_metrics(&results);
    assert_eq!(metrics.repo_run_success_rate, 100.0);
    assert_eq!(metrics.healing_success_rate, 100.0);
    assert_eq!(metrics.fallback_commit_success_rate, 100.0);
    assert_eq!(metrics.url_availability_rate, 100.0);
    assert!(metrics.framework_success_rate.contains_key("nextjs"));
    assert!(metrics.framework_success_rate.contains_key("fastapi"));
    assert!(metrics.framework_success_rate.contains_key("django"));
    assert!(metrics.framework_success_rate.contains_key("axum"));
    assert!(metrics.framework_success_rate.contains_key("vue"));
    assert!(metrics
        .framework_success_rate
        .contains_key("browser-extension"));
}

#[test]
fn execution_match_engine_assigns_framework_image_with_confidence() {
    let fingerprint = RepositoryFingerprint {
        repo_id: "repo-next".to_string(),
        repo_url: "repo-next".to_string(),
        repo_hash: "repo-next".to_string(),
        lockfile_hash: Some("lock".to_string()),
        dependency_hash: Some("deps".to_string()),
        language_signature: "javascript".to_string(),
        framework_signature: Some("nextjs".to_string()),
        ..RepositoryFingerprint::default()
    };

    let matched = ExecutionMatchEngine::match_repository(&fingerprint);
    assert_eq!(matched.image.runtime, RuntimeType::Node);
    assert!(matched.image.image_id.contains("nextjs"));
    assert!(matched.confidence >= 90);
}

#[test]
fn execution_image_compiler_emits_deterministic_eis_spec() {
    let fingerprint = RepositoryFingerprint {
        repo_id: "repo-next".to_string(),
        repo_url: "repo-next".to_string(),
        repo_hash: "repo-next".to_string(),
        lockfile_hash: Some("pnpm-lock".to_string()),
        dependency_hash: Some("deps".to_string()),
        language_signature: "javascript".to_string(),
        framework_signature: Some("nextjs".to_string()),
        ..RepositoryFingerprint::default()
    };

    let compiled = ExecutionImageCompiler::compile(&fingerprint);
    assert_eq!(compiled.image_spec.spec_version, EXECUTION_IMAGE_VERSION);
    assert_eq!(compiled.image_spec.commit_hash, None);
    assert!(compiled.image_spec.deterministic_build);
    assert_eq!(compiled.image_spec.runtime, ImageRuntimeKind::Node);
    assert_eq!(compiled.image_spec.runtime_version, "20");
    assert_eq!(compiled.image_spec.framework, Some(FrameworkKind::NextJs));
    assert_eq!(
        compiled.image_spec.package_manager,
        Some(PackageManagerKind::Pnpm)
    );
    assert!(compiled
        .build_strategy
        .commands
        .contains(&"pnpm run build".to_string()));

    let compiled_again = ExecutionImageCompiler::compile(&fingerprint);
    assert_eq!(
        compiled.image_spec.caching_policy.key,
        compiled_again.image_spec.caching_policy.key
    );
    let commit_compiled = ExecutionImageCompiler::compile_for_commit(&fingerprint, "abc1234");
    assert_eq!(
        commit_compiled.image_spec.commit_hash.as_deref(),
        Some("abc1234")
    );
}

#[test]
fn execution_image_compile_endpoint_returns_compiled_spec_payload() {
    let fingerprint = RepositoryFingerprint {
        repo_id: "repo-fastapi".to_string(),
        repo_url: "repo-fastapi".to_string(),
        repo_hash: "repo-fastapi".to_string(),
        lockfile_hash: Some("uv-lock".to_string()),
        dependency_hash: Some("deps".to_string()),
        language_signature: "python".to_string(),
        framework_signature: Some("fastapi".to_string()),
        ..RepositoryFingerprint::default()
    };

    let (path, body) = execution_image_compile_endpoint(
        "https://github.com/rkendel1/rustgit-",
        "main",
        &fingerprint,
    );
    assert_eq!(path, "/execution-image/compile");
    assert!(body.contains("\"image_spec\""));
    assert!(body.contains("\"runtime\":\"python\""));
    assert!(body.contains("\"confidence\":0."));
    assert!(body.contains("\"deterministic_build\":true"));
}

#[test]
fn warm_pool_manager_tracks_prewarm_allocation_release_and_cache_binding() {
    let fingerprint = RepositoryFingerprint {
        repo_id: "repo-fastapi".to_string(),
        repo_url: "repo-fastapi".to_string(),
        repo_hash: "repo-fastapi".to_string(),
        lockfile_hash: Some("uv-lock".to_string()),
        dependency_hash: Some("deps".to_string()),
        language_signature: "python".to_string(),
        framework_signature: Some("fastapi".to_string()),
        ..RepositoryFingerprint::default()
    };
    let image = ExecutionMatchEngine::match_repository(&fingerprint).image;

    let mut manager = WarmPoolManager::default();
    manager.prewarm(&image, WarmPoolType::Cloud, 2);
    assert_eq!(
        manager.get(&image.image_id).map(|entry| entry.idle_count),
        Some(2)
    );
    assert!(manager.allocate(&image.image_id));
    assert!(manager.mark_running(&image.image_id));
    assert!(manager.release(&image.image_id));
    manager.bind_cache_layer(&fingerprint, &image);
    assert!(manager.has_cache_layer(&fingerprint, &image));

    let status = manager.status();
    assert_eq!(status.total_images, 1);
    assert_eq!(status.warm_containers, 2);
    assert_eq!(status.idle_containers, 2);
    assert_eq!(status.assigned_containers, 0);
}

#[test]
fn warm_runtime_endpoints_expose_execution_image_and_pool_status() {
    let fingerprint = RepositoryFingerprint {
        repo_id: "repo-rust".to_string(),
        repo_url: "repo-rust".to_string(),
        repo_hash: "repo-rust".to_string(),
        lockfile_hash: Some("cargo-lock".to_string()),
        dependency_hash: Some("deps".to_string()),
        language_signature: "rust".to_string(),
        framework_signature: Some("rust".to_string()),
        ..RepositoryFingerprint::default()
    };
    let mut registry = ExecutionImageRegistry::default();
    let (image_path, image_body) =
        execution_image_endpoint("repo-rust", &mut registry, &fingerprint);
    assert_eq!(image_path, "/execution-image/repo-rust");
    assert!(image_body.contains("\"repo_id\":\"repo-rust\""));
    assert!(image_body.contains("\"image\""));
    assert!(image_body.contains("\"image_spec\""));

    let image = registry
        .image_for_repo("repo-rust")
        .cloned()
        .expect("image should be registered");
    let mut pool = WarmPoolManager::default();
    let (prewarm_path, _) =
        warm_pool_prewarm_endpoint(&mut pool, &image, WarmPoolType::LocalDea, 1);
    assert_eq!(prewarm_path, "/warm-pool/prewarm");

    let (status_path, status_body) = warm_pool_status_endpoint(&pool);
    assert_eq!(status_path, "/warm-pool/status");
    assert!(status_body.contains("\"warm_containers\":1"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"POST /execution-image/compile"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"GET /execution-image/{repo_id}"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"POST /fingerprint/generate"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"GET /fingerprint/{repo_id}"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"POST /fingerprint/recompute"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"GET /warm-pool/status"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"POST /warm-pool/prewarm"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"GET /repo/{id}/commits"));
    assert!(RestApiSpec::default()
        .routes
        .contains(&"POST /execute/recover"));
}

#[test]
fn temporal_execution_router_recovers_last_known_good_commit() {
    let graph = RepositoryTimeGraph {
        repo_id: "repo-temporal".to_string(),
        commits: vec![
            CommitNode {
                commit_hash: "aaaaaaa".to_string(),
                timestamp: 3,
                urfs_snapshot: None,
                build_status: Some(BuildStatus::Failed),
                execution_result: Some(ExecutionResult {
                    started: false,
                    stable: false,
                    message: "build failed".to_string(),
                }),
            },
            CommitNode {
                commit_hash: "bbbbbbb".to_string(),
                timestamp: 2,
                urfs_snapshot: Some(RepositoryFingerprint {
                    dependency_hash: Some("deps".to_string()),
                    ..RepositoryFingerprint::default()
                }),
                build_status: Some(BuildStatus::Success),
                execution_result: Some(ExecutionResult {
                    started: true,
                    stable: true,
                    message: "ok".to_string(),
                }),
            },
        ],
        edges: vec![CommitEdge {
            from_hash: "aaaaaaa".to_string(),
            to_hash: "bbbbbbb".to_string(),
        }],
    };
    let router = TemporalExecutionRouter::default();
    let selected = router.route(&graph, "aaaaaaa", RecoveryStrategy::LastKnownGood);
    assert_eq!(selected.as_deref(), Some("bbbbbbb"));
}

#[test]
fn failure_classifier_detects_wrong_package_manager_for_pnpm_lockfile() {
    let classifier = FailureClassifier;
    let fingerprint = RepositoryFingerprint {
        build_signals: BuildSignals {
            has_lockfile: true,
            lockfile_type: Some("pnpm".to_string()),
            build_scripts: vec![],
        },
        ..RepositoryFingerprint::default()
    };
    let failure = FailureSignal {
        message: "npm ERR! install failed".to_string(),
        attempted_command: Some("npm install".to_string()),
        ..FailureSignal::default()
    };
    assert_eq!(
        classifier.classify(&failure, &fingerprint),
        FailureClass::WrongPackageManager
    );
}

#[test]
fn failure_classifier_detects_wrong_package_manager_for_npm_lockfile() {
    let classifier = FailureClassifier;
    let fingerprint = RepositoryFingerprint {
        build_signals: BuildSignals {
            has_lockfile: true,
            lockfile_type: Some("package-lock.json".to_string()),
            build_scripts: vec![],
        },
        ..RepositoryFingerprint::default()
    };
    let failure = FailureSignal {
        message: "yarn install failed".to_string(),
        attempted_command: Some("yarn install".to_string()),
        ..FailureSignal::default()
    };
    assert_eq!(
        classifier.classify(&failure, &fingerprint),
        FailureClass::WrongPackageManager
    );
}

#[test]
fn failure_classifier_detects_missing_dependency_for_python_traceback() {
    let classifier = FailureClassifier;
    let failure = FailureSignal {
        message: "ModuleNotFoundError: No module named 'fastapi'".to_string(),
        ..FailureSignal::default()
    };
    assert_eq!(
        classifier.classify(&failure, &RepositoryFingerprint::default()),
        FailureClass::MissingDependency
    );
}

#[test]
fn environment_resolver_only_generates_known_safe_defaults() {
    let resolver = EnvironmentResolver;
    let defaults = resolver.defaults_for(&["DATABASE_URL".to_string(), "SECRET_TOKEN".to_string()]);
    assert_eq!(
        defaults,
        vec![("DATABASE_URL".to_string(), "database.internal".to_string())]
    );
}

#[test]
fn healing_coordinator_recovers_after_deterministic_repair() {
    #[derive(Debug)]
    struct StubRuntime {
        applied: Vec<RepairAction>,
        result: ExecutionResult,
        healthy: bool,
    }

    impl HealingRuntime for StubRuntime {
        fn apply_repair(&mut self, action: RepairAction) -> bool {
            self.applied.push(action);
            true
        }

        fn re_execute(&mut self) -> ExecutionResult {
            self.result.clone()
        }

        fn health_check(&self) -> bool {
            self.healthy
        }
    }

    let mut coordinator = HealingCoordinator::default();
    let mut runtime = StubRuntime {
        applied: vec![],
        result: ExecutionResult {
            started: true,
            stable: true,
            message: "running".to_string(),
        },
        healthy: true,
    };
    let failure = FailureSignal {
        message: "EADDRINUSE".to_string(),
        ..FailureSignal::default()
    };
    let decision = coordinator.heal_or_escalate(
        "repo-ahes",
        &failure,
        &RepositoryFingerprint::default(),
        &mut runtime,
        &TemporalExecutionRouter::default(),
        &RepositoryTimeGraph::default(),
        "aaaaaaa",
    );
    match decision {
        HealingDecision::Recovered {
            failure_class,
            strategy,
            result,
        } => {
            assert_eq!(failure_class, FailureClass::PortConflict);
            assert!(strategy.actions.contains(&RepairAction::AllocateNewPort));
            assert!(result.stable);
        }
        _ => panic!("expected recovered decision"),
    }
    assert!(runtime.applied.contains(&RepairAction::AllocateNewPort));
    let entries = coordinator.journal.entries_for_repo("repo-ahes");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, HealingOutcome::Success);
}

#[test]
fn healing_coordinator_escalates_to_tre_after_failed_repair_validation() {
    #[derive(Debug)]
    struct StubRuntime {
        result: ExecutionResult,
    }

    impl HealingRuntime for StubRuntime {
        fn apply_repair(&mut self, _action: RepairAction) -> bool {
            true
        }

        fn re_execute(&mut self) -> ExecutionResult {
            self.result.clone()
        }

        fn health_check(&self) -> bool {
            false
        }
    }

    let graph = RepositoryTimeGraph {
        repo_id: "repo-temporal".to_string(),
        commits: vec![
            CommitNode {
                commit_hash: "aaaaaaa".to_string(),
                timestamp: 2,
                urfs_snapshot: None,
                build_status: Some(BuildStatus::Failed),
                execution_result: Some(ExecutionResult {
                    started: false,
                    stable: false,
                    message: "failed".to_string(),
                }),
            },
            CommitNode {
                commit_hash: "bbbbbbb".to_string(),
                timestamp: 1,
                urfs_snapshot: None,
                build_status: Some(BuildStatus::Success),
                execution_result: Some(ExecutionResult {
                    started: true,
                    stable: true,
                    message: "ok".to_string(),
                }),
            },
        ],
        edges: vec![CommitEdge {
            from_hash: "aaaaaaa".to_string(),
            to_hash: "bbbbbbb".to_string(),
        }],
    };
    let mut coordinator = HealingCoordinator::default();
    let mut runtime = StubRuntime {
        result: ExecutionResult {
            started: true,
            stable: false,
            message: "still unstable".to_string(),
        },
    };
    let failure = FailureSignal {
        message: "connection refused".to_string(),
        ..FailureSignal::default()
    };
    let decision = coordinator.heal_or_escalate(
        "repo-temporal",
        &failure,
        &RepositoryFingerprint::default(),
        &mut runtime,
        &TemporalExecutionRouter::default(),
        &graph,
        "aaaaaaa",
    );
    match decision {
        HealingDecision::EscalatedToTre {
            selected_commit, ..
        } => assert_eq!(selected_commit, "bbbbbbb"),
        _ => panic!("expected TRE escalation"),
    }
    let entries = coordinator.journal.entries_for_repo("repo-temporal");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, HealingOutcome::EscalatedToTre);
}

#[test]
fn temporal_endpoints_emit_commit_and_recovery_payloads() {
    let graph = RepositoryTimeGraph {
        repo_id: "repo-temporal".to_string(),
        commits: vec![CommitNode {
            commit_hash: "bbbbbbb".to_string(),
            timestamp: 2,
            urfs_snapshot: None,
            build_status: Some(BuildStatus::Success),
            execution_result: Some(ExecutionResult {
                started: true,
                stable: true,
                message: "ok".to_string(),
            }),
        }],
        edges: vec![],
    };
    let (commits_path, commits_body) = list_repo_commits_endpoint("repo-temporal", &graph);
    assert_eq!(commits_path, "/repo/repo-temporal/commits");
    assert!(commits_body.contains("\"commit_hash\":\"bbbbbbb\""));

    let (execute_path, execute_body) = execute_commit_endpoint(&TemporalExecuteRequest {
        repo: "repo-temporal".to_string(),
        commit: "bbbbbbb".to_string(),
    });
    assert_eq!(execute_path, "/execute");
    assert!(execute_body.contains("\"accepted\":true"));

    let router = TemporalExecutionRouter::default();
    let (recover_path, recover_body) = execute_recover_endpoint(
        &TemporalRecoverRequest {
            repo: "repo-temporal".to_string(),
            strategy: "last_known_good".to_string(),
        },
        &router,
        &graph,
    );
    assert_eq!(recover_path, "/execute/recover");
    assert!(recover_body.contains("\"selected_commit\":\"bbbbbbb\""));
}

#[test]
fn eidb_schema_tracks_required_postgres_tables() {
    let schema = ExecutionIntelligenceDatabase::postgres_schema().join("\n");
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS users"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS organizations"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS memberships"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repositories"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS commits"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS fingerprints"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS services"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS topologies"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS executions"));
    assert!(schema.contains("anon_user_id"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS execution_events"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS billing_events"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS runtime_images"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS warm_pool_usage"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS healing_attempts"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repository_identities"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repair_plans"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repair_outcomes"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repair_artifacts"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS url_allocations"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS workspaces"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS workspace_runtime_bindings"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS agents"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS journey_results"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS commit_execution_results"));
    assert!(schema.contains("CREATE EXTENSION IF NOT EXISTS vector"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repository_context_snapshots"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repository_questions"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repository_answers"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS repository_embeddings"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS execution_embeddings"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS execution_patterns"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS execution_contexts"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS audit_logs"));
}

#[test]
fn eidb_history_endpoints_emit_persisted_payloads() {
    let mut database = ExecutionIntelligenceDatabase::default();
    database.repositories.insert(
        "repo-eidb".to_string(),
        EidbRepositoryRecord {
            repo_id: "repo-eidb".to_string(),
            repo_url: "https://github.com/rkendel1/rustgit-example".to_string(),
            default_branch: "main".to_string(),
            first_seen: 1,
            last_seen: 2,
        },
    );
    database.commits.push(EidbCommitRecord {
        commit_hash: "aaaaaaa".to_string(),
        repository_id: "repo-eidb".to_string(),
        author_date: 10,
        message: "initial".to_string(),
        parent_commit: None,
    });
    database.record_execution(EidbExecutionRecord {
        execution_id: "exec-1".to_string(),
        org_id: Some("org-1".to_string()),
        user_id: Some("user-1".to_string()),
        anon_user_id: None,
        workspace_id: "ws-1".to_string(),
        repository_id: "repo-eidb".to_string(),
        commit_hash: "aaaaaaa".to_string(),
        started_at: 11,
        completed_at: Some(12),
        status: "success".to_string(),
        execution_tier: "CLOUD".to_string(),
    });
    database.record_execution_event(EidbExecutionEventRecord {
        execution_id: "exec-1".to_string(),
        event_type: "STARTED".to_string(),
        created_at: 11,
    });
    database.record_billing_event(EidbBillingEventRecord {
        event_id: "bill-1".to_string(),
        org_id: "org-1".to_string(),
        user_id: "user-1".to_string(),
        workspace_id: "ws-1".to_string(),
        execution_id: "exec-1".to_string(),
        event_type: "EXECUTION_COMPLETED".to_string(),
        runtime_type: "DEA_LOCAL".to_string(),
        resource_usage: json!({
            "duration_seconds": 60.0,
            "healing_cycles": 1,
            "warm_pool_hits": 0,
        }),
        cost_units: 2.5,
        timestamp: 12,
    });
    database.record_healing_attempt(EidbHealingAttemptRecord {
        repository_id: "repo-eidb".to_string(),
        execution_id: "exec-1".to_string(),
        failure_class: "WrongPackageManager".to_string(),
        repair_strategy: "switch-pnpm".to_string(),
        success: true,
        created_at: 12,
    });
    database.record_url_allocation(EidbUrlAllocationRecord {
        workspace_url: "https://workspace-1.trythissoftware.com".to_string(),
        execution_id: "exec-1".to_string(),
        created_at: 11,
        released_at: None,
    });
    database.record_commit_execution_result(EidbCommitExecutionResultRecord {
        commit_hash: "aaaaaaa".to_string(),
        success: true,
        startup_time_ms: 4200.0,
        recorded_at: 12,
    });

    let (repo_history_path, repo_history_body) =
        repository_history_endpoint("repo-eidb", &database);
    assert_eq!(repo_history_path, "/repositories/repo-eidb/history");
    assert!(repo_history_body.contains("\"commit_hash\":\"aaaaaaa\""));

    let (execution_history_path, execution_history_body) =
        execution_history_endpoint("exec-1", &database);
    assert_eq!(execution_history_path, "/executions/exec-1/history");
    assert!(execution_history_body.contains("\"event_type\":\"STARTED\""));
    assert!(execution_history_body.contains("\"cost_units\":2.5"));
    assert!(execution_history_body.contains("workspace-1.trythissoftware.com"));

    let (healing_path, healing_body) = repository_healing_history_endpoint("repo-eidb", &database);
    assert_eq!(healing_path, "/repositories/repo-eidb/healing");
    assert!(healing_body.contains("\"failure_class\":\"WrongPackageManager\""));

    let (last_good_path, last_good_body) =
        repository_last_good_commit_endpoint("repo-eidb", &database);
    assert_eq!(last_good_path, "/repositories/repo-eidb/last-good");
    assert!(last_good_body.contains("\"commit_hash\":\"aaaaaaa\""));
}

#[test]
fn repository_intelligence_endpoint_emits_identity_and_actions() {
    let mut database = ExecutionIntelligenceDatabase::default();
    database.repositories.insert(
        "repo-id".to_string(),
        EidbRepositoryRecord {
            repo_id: "repo-id".to_string(),
            repo_url: "https://github.com/octocat/hello-world".to_string(),
            default_branch: "main".to_string(),
            first_seen: 1,
            last_seen: 2,
        },
    );
    database.record_execution(EidbExecutionRecord {
        execution_id: "exec-1".to_string(),
        org_id: None,
        user_id: None,
        anon_user_id: Some("anon".to_string()),
        workspace_id: "ws-1".to_string(),
        repository_id: "repo-id".to_string(),
        commit_hash: "aaaaaaa".to_string(),
        started_at: 1,
        completed_at: Some(2),
        status: "success".to_string(),
        execution_tier: "WASM".to_string(),
    });
    database.record_healing_attempt(EidbHealingAttemptRecord {
        repository_id: "repo-id".to_string(),
        execution_id: "exec-1".to_string(),
        failure_class: "Dependency".to_string(),
        repair_strategy: "pin_version".to_string(),
        success: true,
        created_at: 3,
    });

    let (path, body) = repository_intelligence_endpoint("repo-id", &database);
    assert_eq!(path, "/api/repositories/repo-id/intelligence");
    assert!(body.contains("\"github_owner\":\"octocat\""));
    assert!(body.contains("\"github_repo\":\"hello-world\""));
    assert!(body.contains("\"execution_score\":100.0"));
    assert!(body.contains("\"healing_score\":100.0"));
    assert!(body.contains("\"actions\""));
    assert!(body.contains("\"launch\":\"/seed/{owner}/{repo}\""));
    assert!(body.contains("\"heal\":\"/repositories/repo-id/healing\""));
    assert!(body.contains("\"adopt\":\"/api/repositories/repo-id/adopt\""));
}

#[test]
fn repository_cognitive_endpoints_emit_digital_twin_signals() {
    let mut database = ExecutionIntelligenceDatabase::default();
    database.repositories.insert(
        "repo-id".to_string(),
        EidbRepositoryRecord {
            repo_id: "repo-id".to_string(),
            repo_url: "https://github.com/octocat/hello-world".to_string(),
            default_branch: "main".to_string(),
            first_seen: 1,
            last_seen: 2,
        },
    );
    database.record_execution(EidbExecutionRecord {
        execution_id: "exec-1".to_string(),
        org_id: None,
        user_id: None,
        anon_user_id: Some("anon".to_string()),
        workspace_id: "ws-1".to_string(),
        repository_id: "repo-id".to_string(),
        commit_hash: "aaaaaaa".to_string(),
        started_at: 1,
        completed_at: Some(2),
        status: "failed".to_string(),
        execution_tier: "WASM".to_string(),
    });
    database.record_execution(EidbExecutionRecord {
        execution_id: "exec-2".to_string(),
        org_id: None,
        user_id: Some("user-1".to_string()),
        anon_user_id: None,
        workspace_id: "ws-2".to_string(),
        repository_id: "repo-id".to_string(),
        commit_hash: "bbbbbbb".to_string(),
        started_at: 3,
        completed_at: Some(7),
        status: "success".to_string(),
        execution_tier: "CLOUD".to_string(),
    });
    database.record_healing_attempt(EidbHealingAttemptRecord {
        repository_id: "repo-id".to_string(),
        execution_id: "exec-1".to_string(),
        failure_class: "Dependency".to_string(),
        repair_strategy: "regenerate-lockfile".to_string(),
        success: true,
        created_at: 4,
    });

    let (twin_path, twin_body) = repository_twin_endpoint("repo-id", &database);
    assert_eq!(twin_path, "/repositories/repo-id/twin");
    assert!(twin_body.contains("\"runtime_topology\""));
    assert!(twin_body.contains("\"risk_graph\""));
    assert!(twin_body.contains("\"behavior_profile\""));

    let (behavior_path, behavior_body) = repository_behavior_endpoint("repo-id", &database);
    assert_eq!(behavior_path, "/repositories/repo-id/behavior");
    assert!(behavior_body.contains("\"behavior_fingerprint\""));

    let (architecture_path, architecture_body) =
        repository_architecture_endpoint("repo-id", &database);
    assert_eq!(architecture_path, "/repositories/repo-id/architecture");
    assert!(architecture_body.contains("\"service_graph\""));

    let (timeline_path, timeline_body) = repository_timeline_endpoint("repo-id", &database);
    assert_eq!(timeline_path, "/repositories/repo-id/timeline");
    assert!(timeline_body.contains("\"timeline\""));

    let (predictions_path, predictions_body) =
        repository_predictions_endpoint("repo-id", &database);
    assert_eq!(predictions_path, "/repositories/repo-id/predictions");
    assert!(predictions_body.contains("\"predicted_failure_probability\""));

    let (recommendations_path, recommendations_body) =
        repository_recommendations_endpoint("repo-id", &database);
    assert_eq!(
        recommendations_path,
        "/repositories/repo-id/recommendations"
    );
    assert!(recommendations_body.contains("\"recommended_actions\""));

    let (blast_radius_path, blast_radius_body) =
        repository_blast_radius_endpoint("repo-id", &database);
    assert_eq!(blast_radius_path, "/repositories/repo-id/blast-radius");
    assert!(blast_radius_body.contains("\"risk_level\""));

    let (dna_path, dna_body) = repository_dna_endpoint("repo-id", &database);
    assert_eq!(dna_path, "/repositories/repo-id/dna");
    assert!(dna_body.contains("\"runtime_topology\""));

    let (risk_path, risk_body) = repository_risk_endpoint("repo-id", &database);
    assert_eq!(risk_path, "/repositories/repo-id/risk");
    assert!(risk_body.contains("\"execution_risk\""));
    assert!(risk_body.contains("\"security_drift\""));

    let (memory_path, memory_body) = repository_memory_endpoint("repo-id", &database);
    assert_eq!(memory_path, "/repositories/repo-id/memory");
    assert!(memory_body.contains("\"successful_repairs\":1"));
    assert!(memory_body.contains("\"entries\""));

    let (simulate_path, simulate_body) =
        repository_simulate_endpoint("repo-id", "dependency drift");
    assert_eq!(simulate_path, "/repositories/repo-id/simulate");
    assert!(simulate_body.contains("\"scenario\":\"dependency drift\""));

    let (infer_path, infer_body) = repository_infer_endpoint("repo-id", "explain execution drift");
    assert_eq!(infer_path, "/repositories/repo-id/infer");
    assert!(infer_body.contains("\"inference\""));

    let (compare_path, compare_body) = repository_compare_endpoint("repo-id", "repo-b");
    assert_eq!(compare_path, "/repositories/repo-id/compare");
    assert!(compare_body.contains("\"similarity\":0.94"));

    let (predict_path, predict_body) = repository_predict_endpoint("repo-id");
    assert_eq!(predict_path, "/repositories/repo-id/predict");
    assert!(predict_body.contains("\"prediction\""));

    let (explain_path, explain_body) = repository_explain_endpoint("repo-id", "risk");
    assert_eq!(explain_path, "/repositories/repo-id/explain");
    assert!(explain_body.contains("\"topic\":\"risk\""));
}

#[test]
fn repository_ask_endpoint_returns_execution_aware_answer_with_evidence() {
    let mut database = ExecutionIntelligenceDatabase::default();
    database.repositories.insert(
        "repo-id".to_string(),
        EidbRepositoryRecord {
            repo_id: "repo-id".to_string(),
            repo_url: "https://github.com/octocat/hello-world".to_string(),
            default_branch: "main".to_string(),
            first_seen: 1,
            last_seen: 2,
        },
    );
    database.record_execution(EidbExecutionRecord {
        execution_id: "exec-1".to_string(),
        org_id: None,
        user_id: None,
        anon_user_id: Some("anon".to_string()),
        workspace_id: "ws-1".to_string(),
        repository_id: "repo-id".to_string(),
        commit_hash: "aaaaaaa".to_string(),
        started_at: 1,
        completed_at: Some(2),
        status: "failed".to_string(),
        execution_tier: "WASM".to_string(),
    });
    database.record_healing_attempt(EidbHealingAttemptRecord {
        repository_id: "repo-id".to_string(),
        execution_id: "exec-1".to_string(),
        failure_class: "WrongPackageManager".to_string(),
        repair_strategy: "switch-pnpm".to_string(),
        success: true,
        created_at: 3,
    });

    let (path, body) =
        repository_ask_endpoint("repo-id", "Why is this repository failing?", &database);
    assert_eq!(path, "/api/repositories/repo-id/ask");
    assert!(body.contains("\"answer\""));
    assert!(body.contains("\"evidence\""));
    assert!(body.contains("\"related_failures\""));
    assert!(body.contains("\"related_healings\""));
}

#[test]
fn intelligence_feedback_loop_endpoints_emit_retrieval_learning_and_context_payloads() {
    let mut database = ExecutionIntelligenceDatabase::default();
    database.record_execution(EidbExecutionRecord {
        execution_id: "exec-1".to_string(),
        org_id: None,
        user_id: None,
        anon_user_id: Some("anon".to_string()),
        workspace_id: "ws-1".to_string(),
        repository_id: "repo-1".to_string(),
        commit_hash: "aaaaaaa".to_string(),
        started_at: 10,
        completed_at: Some(20),
        status: "success".to_string(),
        execution_tier: "WASM".to_string(),
    });
    database.record_execution_context(EidbExecutionContextRecord {
        execution_id: "exec-1".to_string(),
        similar_execution_ids: vec![],
        retrieved_patterns: vec![],
        generated_plan: "npm install".to_string(),
        chosen_plan: "pnpm install".to_string(),
    });

    let (similar_path, similar_body) = intelligence_similar_endpoint("fp-1", &database);
    assert_eq!(similar_path, "/intelligence/similar");
    assert!(similar_body.contains("\"similar_executions\""));

    let (learn_path, learn_body) = intelligence_learn_endpoint(
        &IntelligenceLearnRequest {
            execution_id: "exec-1".to_string(),
            repository_id: "repo-1".to_string(),
            commit_sha: "aaaaaaa".to_string(),
            fingerprint_hash: "fp-1".to_string(),
            generated_plan: "npm install".to_string(),
            chosen_plan: "pnpm install".to_string(),
            status: "pnpm lockfile mismatch".to_string(),
            duration_seconds: Some(12),
            cost_units: Some(1.5),
            repair: Some("switch-pnpm".to_string()),
        },
        &mut database,
    );
    assert_eq!(learn_path, "/intelligence/learn");
    assert!(learn_body.contains("WrongPackageManager"));
    assert_eq!(database.execution_embeddings.len(), 1);
    assert_eq!(database.execution_patterns.len(), 1);

    let (optimize_path, optimize_body) = intelligence_optimize_endpoint(
        &IntelligenceOptimizeRequest {
            execution_id: "exec-2".to_string(),
            fingerprint_hash: "fp-1".to_string(),
            generated_plan: "npm install && npm run build".to_string(),
            failure_type: Some("WrongPackageManager".to_string()),
        },
        &mut database,
    );
    assert_eq!(optimize_path, "/intelligence/optimize");
    assert!(optimize_body.contains("\"optimized_plan\""));
    assert_eq!(database.execution_contexts.len(), 2);

    let (context_path, context_body) = intelligence_context_endpoint("exec-2", &database);
    assert_eq!(context_path, "/intelligence/context");
    assert!(context_body.contains("\"execution_id\":\"exec-2\""));
}

#[test]
fn eidb_last_good_commit_falls_back_to_successful_execution_status() {
    let mut database = ExecutionIntelligenceDatabase::default();
    database.commits.push(EidbCommitRecord {
        commit_hash: "bbbbbbb".to_string(),
        repository_id: "repo-eidb".to_string(),
        author_date: 10,
        message: "run".to_string(),
        parent_commit: None,
    });
    database.record_execution(EidbExecutionRecord {
        execution_id: "exec-2".to_string(),
        org_id: Some("org-1".to_string()),
        user_id: Some("user-1".to_string()),
        anon_user_id: None,
        workspace_id: "ws-2".to_string(),
        repository_id: "repo-eidb".to_string(),
        commit_hash: "bbbbbbb".to_string(),
        started_at: 11,
        completed_at: Some(12),
        status: "Succeeded".to_string(),
        execution_tier: "DEA".to_string(),
    });
    assert_eq!(
        database.last_good_commit_for_repository("repo-eidb"),
        Some("bbbbbbb")
    );
}

#[test]
fn execution_meter_computes_cost_breakdown() {
    let mut meter = ExecutionMeter::new(
        "exec-meter-1",
        "org-meter-1",
        "user-meter-1",
        "ws-meter-1",
        RuntimeTier::DockerLocal,
    );
    meter.heartbeat(0.4, 256.0);
    meter.heartbeat(0.8, 512.0);
    meter.record_retry();
    meter.record_healing_cycle();
    meter.record_warm_pool_hit();

    let cost = meter.complete_with_elapsed(Duration::from_secs(120));
    assert!(cost.duration_cost > 0.0);
    assert_eq!(cost.runtime_cost, 2.0);
    assert_eq!(cost.retry_penalty, 0.25);
    assert_eq!(cost.healing_cost, 1.0);
    assert!(cost.warm_pool_discount > 0.0);
    assert!(cost.total_cost_units > 0.0);
}

#[test]
fn billing_endpoints_emit_usage_summary_and_invoice_payloads() {
    let mut database = ExecutionIntelligenceDatabase::default();
    database.record_billing_event(EidbBillingEventRecord {
        event_id: "bill-usage-1".to_string(),
        org_id: "org-usage-1".to_string(),
        user_id: "user-usage-1".to_string(),
        workspace_id: "ws-usage-1".to_string(),
        execution_id: "exec-usage-1".to_string(),
        event_type: "EXECUTION_COMPLETED".to_string(),
        runtime_type: "DEA_LOCAL".to_string(),
        resource_usage: json!({
            "duration_seconds": 30.0,
            "healing_cycles": 0,
            "warm_pool_hits": 1,
        }),
        cost_units: 1.2,
        timestamp: 20,
    });
    database.record_billing_event(EidbBillingEventRecord {
        event_id: "bill-usage-2".to_string(),
        org_id: "org-usage-1".to_string(),
        user_id: "user-usage-1".to_string(),
        workspace_id: "ws-usage-1".to_string(),
        execution_id: "exec-usage-2".to_string(),
        event_type: "EXECUTION_HEALING_ATTEMPTED".to_string(),
        runtime_type: "CLOUD_FALLBACK".to_string(),
        resource_usage: json!({
            "duration_seconds": 90.0,
            "healing_cycles": 2,
            "warm_pool_hits": 0,
        }),
        cost_units: 4.8,
        timestamp: 21,
    });

    let (usage_path, usage_body) = billing_usage_endpoint("org-usage-1", &database);
    assert_eq!(usage_path, "/billing/usage?org_id=org-usage-1");
    assert!(usage_body.contains("\"free_tier_usage\""));
    assert!(usage_body.contains("\"total_cost_units\":6.0"));

    let (summary_path, summary_body) = billing_summary_endpoint(&database);
    assert_eq!(summary_path, "/billing/summary");
    assert!(summary_body.contains("\"runtime_distribution_costs\""));
    assert!(summary_body.contains("\"healing_costs\":4.8"));

    let (invoice_path, invoice_body) = billing_invoice_endpoint("org-usage-1", &database);
    assert_eq!(invoice_path, "/billing/invoice");
    assert!(invoice_body.contains("\"total_cost_units\":6.0"));
    assert!(invoice_body.contains("exec-usage-1"));
}

#[test]
fn fingerprint_endpoints_expose_urfs_payload() {
    let fingerprint = RepositoryFingerprint {
        spec_version: "1.0".to_string(),
        repo_id: "repo-id".to_string(),
        repo_url: "https://github.com/rkendel1/rustgit-".to_string(),
        languages: vec![LanguageProfile {
            language: Language::Rust,
            confidence: 0.9,
            files_detected: vec!["src/lib.rs".to_string()],
        }],
        frameworks: vec![FrameworkProfile {
            framework: "Rust".to_string(),
            version: None,
            confidence: 0.8,
            detection_signals: vec!["Cargo.toml".to_string()],
        }],
        package_managers: vec!["cargo".to_string()],
        services: vec![ServiceFingerprint {
            service_name: "api".to_string(),
            service_type: ServiceType::Backend,
            root_path: ".".to_string(),
            runtime_hint: RuntimeKind::Rust,
            framework: Some("Rust".to_string()),
            entry_file: Some("src/main.rs".to_string()),
            build_context: BuildContext {
                install_command: Some("cargo fetch".to_string()),
                build_command: Some("cargo build".to_string()),
                package_manager: Some("cargo".to_string()),
            },
        }],
        entrypoints: vec![EntryPoint {
            path: "Cargo.toml".to_string(),
            command: "cargo run".to_string(),
            confidence: 0.9,
        }],
        dependency_graph: DependencyGraph {
            nodes: vec![DependencyNode {
                id: "api".to_string(),
            }],
            edges: vec![],
        },
        runtime_signals: RuntimeSignals {
            rust_detected: true,
            ..RuntimeSignals::default()
        },
        build_signals: BuildSignals {
            has_lockfile: true,
            lockfile_type: Some("cargo".to_string()),
            build_scripts: vec!["build".to_string()],
        },
        infra_signals: InfraSignals::default(),
        confidence: 0.88,
        confidence_model: ConfidenceModel {
            overall: 0.88,
            framework_confidence: 0.85,
            runtime_confidence: 0.9,
            topology_confidence: 0.9,
        },
        repo_hash: "hash".to_string(),
        lockfile_hash: Some("lock".to_string()),
        dependency_hash: Some("deps".to_string()),
        language_signature: "rust".to_string(),
        framework_signature: Some("rust".to_string()),
    };
    let (generate_path, generate_body) = fingerprint_generate_endpoint(&fingerprint);
    assert_eq!(generate_path, "/fingerprint/generate");
    assert!(generate_body.contains("\"spec_version\":\"1.0\""));
    assert!(generate_body.contains("\"service_type\":\"backend\""));

    let (get_path, get_body) = fingerprint_get_endpoint("repo-id", &fingerprint);
    assert_eq!(get_path, "/fingerprint/repo-id");
    assert!(get_body.contains("\"repo_id\":\"repo-id\""));

    let (recompute_path, recompute_body) = fingerprint_recompute_endpoint(&fingerprint);
    assert_eq!(recompute_path, "/fingerprint/recompute");
    assert!(recompute_body.contains("\"status\":\"recomputed\""));
}

#[test]
fn github_clone_extra_header_includes_bearer_token_for_github_https_repos() {
    let header = github_clone_extra_header_with_token(
        "https://github.com/rkendel1/new_vue-healed4.git",
        Some("ghp_test123"),
    )
    .expect("expected auth header");

    assert!(header.starts_with("Authorization: Bearer "));
    assert!(header.ends_with("ghp_test123"));
}

#[test]
fn github_clone_extra_header_ignores_non_github_urls() {
    let header =
        github_clone_extra_header_with_token("https://gitlab.com/group/repo.git", Some("token"));
    assert!(header.is_none());
}

#[test]
fn github_clone_extra_header_ignores_empty_token() {
    let header = github_clone_extra_header_with_token(
        "https://github.com/rkendel1/new_vue-healed4.git",
        Some("   "),
    );
    assert!(header.is_none());
}

#[test]
fn github_clone_error_reason_surfaces_auth_guidance_for_github_username_prompt_failure() {
    let reason = github_clone_error_reason(
        "https://github.com/rkendel1/new_vue-healed4.git",
        "fatal: could not read Username for 'https://github.com': No such device or address",
    );
    assert!(reason.contains("GitHub authentication is required"));
    assert!(reason.contains("RUSTGIT_GITHUB_TOKEN"));
}

#[test]
fn github_clone_error_reason_preserves_non_auth_stderr() {
    let stderr = "fatal: repository 'https://github.com/rkendel1/missing.git/' not found";
    let reason = github_clone_error_reason("https://github.com/rkendel1/missing.git", stderr);
    assert_eq!(reason, stderr);
}

#[test]
fn execution_routing_mode_is_declared_by_provider_not_guessed_from_labels() {
    assert!(matches!(
        WasmExecutionProvider.transport(),
        ExecutionRoutingMode::Wasm
    ));
    assert!(matches!(
        DockerExecutionProvider.transport(),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        NodeRuntimeProvider.transport(),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        GoExecutionProvider.transport(),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        PythonExecutionProvider.transport(),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        JavaExecutionProvider.transport(),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        RustRuntimeProvider.transport(),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        StaticRuntimeProvider.transport(),
        ExecutionRoutingMode::Local
    ));

    assert!(matches!(
        transport_for_provider_id("WasmExecutionProvider"),
        ExecutionRoutingMode::Wasm
    ));
    assert!(matches!(
        transport_for_provider_id("DockerExecutionProvider"),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        transport_for_provider_id("NodeRuntimeProvider"),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        transport_for_provider_id("GoExecutionProvider"),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        transport_for_provider_id("PythonExecutionProvider"),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        transport_for_provider_id("JavaExecutionProvider"),
        ExecutionRoutingMode::Local
    ));
    assert!(matches!(
        transport_for_provider_id("SomeFutureCloudThing"),
        ExecutionRoutingMode::Local
    ));
}

#[test]
fn runtime_status_never_carries_endpoint_outside_local_mode() {
    const DEFAULT_TEST_PID: u32 = 1;
    let runtime_root = temp_dir("runtime-status-invariant");
    let local_repo = temp_dir("runtime-status-invariant-repo");
    fs::write(
        local_repo.join("package.json"),
        r#"{"scripts":{"dev":"node server.js"},"dependencies":{}}"#,
    )
    .expect("write package.json");
    fs::write(
            local_repo.join("server.js"),
            "require('http').createServer((_, res) => res.end('ok')).listen(process.env.PORT || 3000);\n",
        )
        .expect("write server.js");

    let manager = WorkspaceManager::new(&runtime_root);
    let workspace = manager
        .launch(local_repo.to_string_lossy().as_ref())
        .expect("launch workspace");
    {
        let mut workspaces = manager.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get_mut(&workspace.id)
            .expect("workspace record should exist");
        let requested_port = record
            .workspace
            .ports
            .first()
            .map(|port| port.port)
            .unwrap_or(3000);
        let pid = record
            .child_process
            .as_ref()
            .map(|child| child.id())
            .unwrap_or(DEFAULT_TEST_PID);
        let mut runtime = ExecutionTruth::new(workspace.id.clone(), requested_port, pid);
        runtime.update_from_event(ExecutionTruthEvent::ProcessAlive(true));
        runtime.update_from_event(ExecutionTruthEvent::ObservedPort(Some(requested_port)));
        runtime.update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Running));
        record.runtime = Some(runtime);
    }

    let status = manager
        .runtime_status(&workspace.id)
        .expect("runtime status should resolve");
    let handle = status
        .execution_handle
        .expect("launched workspace should have an execution handle");

    match handle.routing_mode {
        ExecutionRoutingMode::Local => {
            if let Some(endpoint) = handle.endpoint.as_deref() {
                assert!(
                    endpoint.starts_with("http://127.0.0.1:"),
                    "Local endpoint must be loopback-derived, got: {endpoint}"
                );
            }
        }
        ExecutionRoutingMode::Wasm
        | ExecutionRoutingMode::Remote
        | ExecutionRoutingMode::Hybrid => {
            assert!(
                handle.endpoint.is_none(),
                "non-Local routing mode must never carry an HTTP endpoint; \
                     got mode={:?} endpoint={:?}. If this fails, a Remote/Hybrid \
                     provider has started populating endpoint — re-derive the \
                     per-mode invariants and proxy validation before merging.",
                handle.routing_mode,
                handle.endpoint
            );
        }
    }

    manager.stop(&workspace.id).expect("stop workspace");
}

#[test]
fn launch_runtime_provider_matches_spawn_authority() {
    let runtime_root = temp_dir("launch-runtime-provider");
    let local_repo = temp_dir("launch-runtime-provider-repo");
    fs::write(
        local_repo.join("package.json"),
        r#"{"scripts":{"dev":"node server.js"},"dependencies":{}}"#,
    )
    .expect("write package.json");
    fs::write(
            local_repo.join("server.js"),
            "require('http').createServer((_, res) => res.end('ok')).listen(process.env.PORT || 3000);\n",
        )
        .expect("write server.js");

    let manager = WorkspaceManager::new(&runtime_root);
    let workspace = manager
        .launch(local_repo.to_string_lossy().as_ref())
        .expect("launch workspace");

    {
        let workspaces = manager.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get(&workspace.id)
            .expect("workspace record should exist");
        assert_eq!(
            record
                .runtime
                .as_ref()
                .map(|runtime| runtime.provider_selected.as_str()),
            Some("local-supervised-process")
        );
    }

    manager.stop(&workspace.id).expect("stop workspace");
}
