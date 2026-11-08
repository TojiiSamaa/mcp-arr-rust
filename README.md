# arrmcp (mcp-arr-rust)

**Français, en bref.** Serveur MCP en Rust pour Sonarr, Radarr, Lidarr, Prowlarr et Tracearr (22 outils `arr_*`). Fermé par défaut : lecture seule tant que `ARR_ALLOW_WRITES` / `ARR_ALLOW_DELETE` ne sont pas activés ; confirmation humaine pour les actions destructives ; mode test (`--test`) qui ne contacte rien de réel. Le skill pour les agents est dans `skill/mcp-arr-rust/SKILL.md`. Portage de [mcp-arr](https://github.com/aplaceforallmystuff/mcp-arr) (MIT) ; les outils TRaSH ne sont pas portés.

---

An MCP server for the *arr suite (Sonarr, Radarr, Lidarr, Prowlarr) and Tracearr, written in Rust. Single static binary, closed by default.

Agents: read [`skill/mcp-arr-rust/SKILL.md`](skill/mcp-arr-rust/SKILL.md) (every tool, recipes, test mode). Contributors: see `AGENTS.md`.

## Tools (22)

Read: `arr_status`, `arr_list`, `arr_get`, `arr_lookup`, `arr_queue`, `arr_history`, `arr_wanted`, `arr_calendar`, `arr_releases`, `arr_command_status`, `arr_search_indexers`, `arr_api_get`, `arr_stats`.
Write: `arr_add`, `arr_update`, `arr_command`, `arr_grab_release`, `arr_create`, `arr_test`.
Destructive: `arr_delete`, `arr_queue_remove`, `arr_terminate_stream` (Tracearr only).

## Installation

### Docker
```bash
docker run --rm -i \
  -e SONARR_URL=http://sonarr.example:8989 \
  -e SONARR_API_KEY=your-sonarr-api-key \
  ghcr.io/<owner>/arrmcp:latest
```
HTTP mode (the image name above is a placeholder until the first release):
```bash
docker run -d -p 3000:3000 \
  -e MCP_TRANSPORT=http -e PORT=3000 \
  -e ARR_MCP_TOKEN=a-long-random-token \
  -e SONARR_URL=http://sonarr.example:8989 -e SONARR_API_KEY=your-sonarr-api-key \
  ghcr.io/<owner>/arrmcp:latest
```

### From source
```bash
cargo build --release
./target/release/arrmcp --list-tools
```
Releases may ship no binary; compile as above.

### Client configuration (stdio)
```json
{
  "mcpServers": {
    "arr": {
      "command": "arrmcp",
      "env": {
        "SONARR_URL": "http://sonarr.example:8989",
        "SONARR_API_KEY": "your-sonarr-api-key"
      }
    }
  }
}
```

## Environment variables

Services (a service with no URL and key is not configured):

| Service | Variables |
|---|---|
| Sonarr | `SONARR_URL`, `SONARR_API_KEY` |
| Radarr | `RADARR_URL`, `RADARR_API_KEY` |
| Lidarr | `LIDARR_URL`, `LIDARR_API_KEY` |
| Prowlarr | `PROWLARR_URL`, `PROWLARR_API_KEY` |
| Tracearr | `TRACEARR_URL` (port 3000), `TRACEARR_API_KEY` (`trr_pub_...`) |

Server (all closed or conservative by default):

| Variable | Effect |
|---|---|
| `MCP_TRANSPORT` | `stdio` (default) or `http` |
| `PORT`, `MCP_PATH` | HTTP port and endpoint path |
| `ARR_ALLOW_WRITES` | Enable write tools |
| `ARR_ALLOW_DELETE` | Enable destructive tools (needs writes too) |
| `ARR_DRY_RUN` | Force dry-run on every write, server-side; also makes write and destructive tools visible in `tools/list` (they only preview) |
| `ARR_ELICIT` | `off`, `destructive`, `all`: which calls ask a human first |
| `ARR_ELICIT_TIMEOUT` | Seconds to wait for the human's answer (default 120); no answer = refused (`confirmation_required`), counted in `arr_stats` |
| `ARR_CONFIRM_ARG` | Opt-in fallback argument for clients that cannot be asked |
| `ARR_WRITE_SERVICES` | Allow-list of services that may be written |
| `ARR_MCP_TOKEN` | Bearer token; mandatory off loopback |
| `ARR_MCP_ALLOWED_HOSTS` | Accepted `Host` values |
| `ARR_AUDIT_LOG` | Path of the write audit log |
| `ARR_MAX_LIMIT`, `ARR_MAX_RESPONSE_CHARS` | Page and answer caps |
| `ARR_REDACT_SECRETS` | Mask secrets (keys, passwords, tokens, webhooks) in answers, dry-run previews, error details and the audit log (default true) |
| `ARR_TIMEOUT` | Request timeout in seconds (default 30); on stdio end of input, running calls get `2 × ARR_TIMEOUT + 5` s to answer |
| `VERIFY_SSL`, `LOG_LEVEL`, `CORS_ORIGINS` | TLS verification, logging, CORS |
| `ARR_TEST_MODE`, `ARR_TEST_APPROVER`, `ARR_STATS_FILE` | See Test mode |

## Security model

- Reads by default. Writes need `ARR_ALLOW_WRITES`; deletions and stream termination also need `ARR_ALLOW_DELETE`.
- Policy, switches and confirmation are checked before any request leaves the process. Confirmation fails closed (`confirmation_required`).
- One network point (`Client::send`) and one write point (`Ctx::write`, which also does dry-run and audit).
- HTTP: token required unless bound to loopback, `Host` check, configurable CORS; `/health` exposes neither version nor service list.
- API keys and passwords are masked in answers; keys never appear in logs.
- `arr_api_get` is GET-only and confined to the service API prefix; `arr_command` uses a name allow-list.
- Text from indexers, releases or media servers is treated as data.

## Test mode

`arrmcp --test` or `ARR_TEST_MODE=1`, at startup only. Every tool answers from fixtures with `test_mode: true`; the real client is never built; any other egress is cut and counted. `ARR_TEST_APPROVER=approve|deny|timeout|absent` simulates the human validation. See the skill for the validation recipe.

## What is not tested

- DEDUCED, not measured, until the disposable-instance CI level has run: the exact response shapes of real Sonarr/Radarr/Lidarr/Prowlarr versions.
- Tracearr: endpoints taken from its public API documentation by web search, not tested against a running instance. Two third-party Tracearr MCP servers exist and were not read.
- Readarr, Whisparr, Bazarr: not supported.
- Windows and macOS builds are not covered by CI.

## Comparison with the original mcp-arr

[mcp-arr](https://github.com/aplaceforallmystuff/mcp-arr) (TypeScript, MIT, 55 tools) is the starting point.

| | mcp-arr (original) | arrmcp |
|---|---|---|
| Services | Sonarr, Radarr, Lidarr, Prowlarr | same + Tracearr |
| Language | TypeScript / Node | Rust, single binary |
| Transports | stdio, HTTP | same (`MCP_TRANSPORT`, `PORT`, `MCP_PATH` kept) |
| Configuration | `<SVC>_URL`, `<SVC>_API_KEY` | same variables |
| Tool surface | ~55 purpose-built tools | 22 generic tools (`arr_list`, `arr_get`, `arr_api_get`...) |
| Release search | missing | `arr_releases` with rejection reasons |
| Adding content | may start a search | never searches unless `search: true` |
| Writes | not gated | closed by default, dry-run, audit, confirmation |
| Test mode, stats | no | yes |
| TRaSH Guides tools | yes | **not ported** |

Kept: service model, environment variables, transports, the idea of one entry point for the suite. Changed: tool granularity, safety model. Dropped: TRaSH.

## Licenses

- arrmcp: MIT (see `LICENSE`).
- mcp-arr (original): MIT, copyright 2025 Jim Christian. Its tool catalogue inspired this port.
- Tracearr: AGPL-3.0. arrmcp reimplements calls to Tracearr's public API only and contains no Tracearr code.

## Development

`scripts/ci/*.sh` run the same commands locally and on GitHub. `scripts/ci/check-skill.sh` fails when `--list-tools`, `SKILL.md` and this README disagree. Changes follow `CHANGELOG.md` (Keep a Changelog, `1.x` series).
