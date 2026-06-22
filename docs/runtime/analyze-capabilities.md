# Analyze Intelligence Capabilities (Code-Grounded)

## What Analyze detects

| Item | Detected | Persisted | Used by Run | Used by Heal | Used by Retry | Evidence |
|---|---:|---:|---:|---:|---:|---|
| Framework | Yes | Yes | Indirect | No | No | `detect_framework` + payload (`src/analyze/framework_detector.rs:13-106`, `src/analyze/analyzer.rs:39-45`, `98-108`) |
| Runtime | Yes | Yes | Indirect | No | No | `detect_runtime` (`src/analyze/runtime_detector.rs:34-73`) |
| Package Manager | Yes | Yes | Yes (run command resolution from `.execution.json`) | Yes (healed command generation) | Indirect | (`src/analyze/runtime_detector.rs:27-31`, `src/analyze/manifest_builder.rs:173-179`, `src/lib.rs:8000-8010`) |
| Dockerfile / compose / devcontainer | Yes | Yes | Yes (docker/devcontainer command can become start command) | No | No | (`src/analyze/manifest_builder.rs:99-106`, `305-312`, `275-283`) |
| Node Version | Yes | Yes | No direct runtime usage found | No | No | `infer_node_version` + payload (`src/analyze/manifest_builder.rs:215-273`, `179`) |
| Environment Variables | Yes | Yes | Via overrides and runtime env model | No direct auto-heal from this field | No | (`src/analyze/manifest_builder.rs:364-411`, `138-140`) |
| Start Commands | Yes | Yes | Yes (loaded from `.execution.json`) | Yes (`apply_safe_command_heals`) | No | (`src/analyze/manifest_builder.rs:297-362`, `189-213`, `src/lib.rs:8000-8010`) |
| Build Commands | Yes | Yes | Indirect | No | No | (`src/analyze/manifest_builder.rs:135-137`, `285-295`) |
| Ports | Partially (command placeholder, runtime probes in launch) | Partial | Yes (launch sets `PORT`) | No | No | (`src/analyze/manifest_builder.rs:209`, `src/lib.rs:13273`, `12491-12523`) |
| Monorepo/workspace topology | Yes (in repository analyzer path) | Yes | Indirect | Indirect | Indirect | (`src/lib.rs:15347-15350`, `14518-14557`) |
| Language | Yes | Yes | Indirect | No | No | (`src/analyze/framework_detector.rs:9-11`, `src/analyze/analyzer.rs:104-108`) |
| Lockfiles | Yes | Yes | Yes (install command and package manager inference) | No | Yes (pnpm retry path) | (`src/analyze/runtime_detector.rs:8-22`, `src/lib.rs:15367-15389`, `13145-13199`) |
| `execution-plan` / `runtime-capabilities` / `launch-plan` artifacts | Yes | Yes (files) | Launch plan consumed by clients; runtime launch still uses runtime analyzer path | No | No | (`src/analyze/analyzer.rs:150-175`) |

## Analyze outputs consumed by Run/Heal/Retry
- **Run**: runtime launch can read persisted `.execution.json` start command (`src/lib.rs:13069-13077`, `8000-8010`).
- **Heal**: analyze manifest applies safe command heals (`hostInjection`, `portInjection`) before persisting (`src/analyze/manifest_builder.rs:130-134`, `189-213`).
- **Retry**: runtime launcher includes install retry logic (pnpm lockfile retry), independent of analyze service call (`src/lib.rs:13145-13199`).

## Assumptions / unknowns
- “Used by Heal/Retry” is interpreted as code paths in this repo that mutate command strategy or rerun execution, not external services.
