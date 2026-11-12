# Changelog

All notable changes are documented here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow the `1.x` series. The release workflow reads the section of the released version as the release notes and fails if it is missing.

## [Unreleased]

## [1.1.2] - 2026-11-12

### Added
- `arr_grab_release` accepts, for Sonarr only, a manual series/episode mapping for releases whose series Sonarr cannot infer from the title (`Unable to find matching series and episodes, will need to be manually provided`, e.g. `Unknown Series` on a season pack): `seriesId` alone (Sonarr reads the episodes from the title), or `seriesId` + `episodeIds` + `quality` + `languages` (the last two copied from the `arr_releases` row) sent with `shouldOverride`, as the Sonarr interface does. The values are validated before any request (positive ids, 1 to 500 episodes, all four together, other services refuse them); without these fields the request is unchanged. Test: `grab_with_a_manual_sonarr_mapping_succeeds_where_the_bare_grab_fails` (#9).

## [1.1.1] - 2026-11-10

### Added
- `ARR_SEARCH_TIMEOUT` (seconds, default 120): live indexer searches (`arr_releases`, `arr_search_indexers`) get the larger of it and `ARR_TIMEOUT` instead of failing after 30 s when the indexers need longer; `0` keeps `ARR_TIMEOUT` for them. The stdio drain at end of input follows the larger of the two (#6).

### Fixed
- A `timeout` error now states its limit in seconds, the variable that sets it and the phase (connection, fixed at 10 s, or total time); its hint says the MCP client has its own timeout, separate from arrmcp's, to keep at least as long (#6).

## [1.1.0] - 2026-11-10

### Added
- `deploy/` kit for pull-based deployment: `pull-deploy.sh` pulls `ghcr.io/tojiisamaa/mcp-arr-rust:latest` (`TAG=x.y.z` pins a version), does nothing when the running digest is current and healthy, checks `/health` (and an MCP `initialize` when `ARR_MCP_TOKEN` is set) after the swap and rolls back to the previous digest on failure; systemd timer and service, compose example, `--a-blanc` dry run.
- `compact: true` on `arr_list`, `arr_queue`, `arr_history`, `arr_wanted`, `arr_calendar` and `arr_releases`: keeps the useful keys only (explicit `fields` still win). Measured on synthetic 5 000-item libraries: 58 KB to 5 KB for movies, 66 KB to 8.7 KB for the queue.
- Read-only resources: `remotepathmapping`, `task` (`system/task`), `update` on the four *arr; `importlistexclusion` on Sonarr, Lidarr and Prowlarr; `indexerstatus` on Prowlarr. Tracearr `activity` and `health` are routed to the v1 API.

### Fixed
- Oversized answers whose rows sit under `result.items` / `result.records` were cut as raw text; they are now cut by rows with `has_more` / `next_offset`.
- `arr_api_get` refuses `manualimport` (lists files of any host folder), Prowlarr `indexer/*/download` and `indexer/*/newznab` (side effects) and the Tracearr `events` stream (never ends).
- Removed the Tracearr `media` list (404 on 3.1.7).

## [1.0.0]

### Added
- Rust port of mcp-arr (crate `arrmcp`): 22 tools prefixed `arr_` for Sonarr, Radarr, Lidarr and Prowlarr.
- Tracearr support (reads and `arr_terminate_stream`).
- `arr_releases` (release search with rejection reasons), `arr_api_get` (confined GET on the rest of the API), `arr_stats`.
- Closed-by-default policy: writes and deletions behind separate switches, server-side dry-run, admin confirmation that fails closed, audit log, secret redaction, per-service write allow-list.
- stdio and Streamable HTTP transports; token required off loopback; `Host` check.
- Test mode (`--test` / `ARR_TEST_MODE=1`) with mock backend, network cut-off, simulated approver.
- Portable agent skill (`skill/mcp-arr-rust/SKILL.md`), `AGENTS.md`, `scripts/ci/check-skill.sh`.

### Changed
- `arr_add` never starts a search unless `search: true` is given.
- A missing object is answered with `found:false`, not a tool error.

### Security
- Secret redaction widened to a documented, tested rule: any key containing `token`, `secret`, `password`, `passwd`, `passphrase`, `passkey`, `webhook`, `apikey` or `credential` (any case), `key`/`psk`, or ending with the word `Key` (not `keyword`, `ratingKey`, `sortKey`); also the `{name, value}` setting fields and `{propertyName, attemptedValue}` validation errors.
- Redaction now also covers dry-run previews (`would_send`), error `details` and the audit log; a JSON error body is never copied into the error message.
- `arr_update` (and `arr_create`/`arr_test` bodies) refuse the `[redacted]` mask before any request, so a read object copied back can never overwrite a real secret.
- `arr_api_get` refuses the endpoints that read the host disk (`filesystem`, `system/backup`, `log/file`, `mediacover`) before any request.
- `arr_terminate_stream` checks that Tracearr is configured and writable (`ARR_WRITE_SERVICES`) before asking a human; the last-line check in the write path treats it as destructive (`ARR_ALLOW_DELETE`).
- Human confirmation waits at most `ARR_ELICIT_TIMEOUT` seconds (default 120), then fails closed with `confirmation_required`, counted in `arr_stats`.
- Proof A (static test-mode check) no longer stops at a file's `#[cfg(test)]` line: only the test item itself is skipped, so a network call added after a test module is caught (mutation case added; found by the public CI refusal test).
- CI: the release workflow runs the full check suite first (`workflow_call`), and every Docker image (build, CI scripts, disposable instances) is pinned by digest.

### Fixed
- Tracearr, checked against a disposable v3.1.7 instance: `arr_list` `watched-media` and `top-media` require `filters.media_type` (`movie`, `show`, `episode` / `movie`, `show`) and are refused before any request without it (the service answered 400); `arr_terminate_stream` accepts only a session UUID (the `id` of `streams`), anything else is refused before any request (the service answered 400 "Invalid session ID").
- stdio: when the input closes, every running call still answers (bounded by `2 × ARR_TIMEOUT + 5` s) instead of being cut after 2 s.
- Test mode: a path outside the fixtures is counted as `egress_blocked`, not as a simulated request.
- Server instructions and skill: exact list of tools without `service`; `RssSync` is sonarr/radarr only; no `outputSchema` claim.

### Not ported
- TRaSH Guides tools of the original mcp-arr.
