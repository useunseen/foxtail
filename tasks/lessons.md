# Lessons

## 2026-03-11

- Do not elevate local-only mock-service admin surfaces into required security fixes without checking whether the deployment model intentionally assumes loopback or trusted local use.

## 2026-03-23

- When the user provides a writing sample for tone, learn the naturalness and pacing from it but do not mirror the sample's rhetorical structure or phrasing too closely. Translate the lesson into a fresh voice.

## 2026-05-13

- When a Make target should recover from a missing local binary, depend on the binary file path rather than a phony `build` target if the desired behavior is "build only when absent."

## 2026-08-09

- A verification harness must never rewrite or re-digest a committed consumer
  manifest to make a cross-repository receipt ready; preserve the exact
  contract, report the downstream fingerprint/source-refresh blocker, and leave
  contract refresh ownership with the consumer repository.
