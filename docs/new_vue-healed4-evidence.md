## new_vue-healed4 analyze + run evidence

Repository target:

```text
https://github.com/rkendel1/new_vue-healed4.git
```

Analyze request payload used by the portal:

```json
{
  "repo_url": "https://github.com/rkendel1/new_vue-healed4.git"
}
```

Analyze endpoint order used by the portal:

```text
POST /api/v1/repositories/analyze
POST /api/repositories/analyze
POST /api/v1/workspaces
POST /api/workspaces
POST /workspaces
```

CLI run artifact:

```bash
$ /usr/bin/time -f 'elapsed_seconds=%e' cargo run --bin wasm-workspace-cli -- launch https://github.com/rkendel1/new_vue-healed4.git
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.18s
Running `target/debug/wasm-workspace-cli launch 'https://github.com/rkendel1/new_vue-healed4.git'`
Cloning into '/home/runner/work/rustgit/rustgit/.wasm-runtime/workspaces/ws-1782016119080-0/repo'...
fatal: could not read Username for 'https://github.com': No such device or address
launch failed: command failed: git clone exited with status exit status: 128
elapsed_seconds=0.39
```
