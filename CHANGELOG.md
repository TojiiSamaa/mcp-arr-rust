# Changelog

All notable changes are documented here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow the `1.x` series. The release workflow reads the section of the released version as the release notes and fails if it is missing.

## [Unreleased]

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
