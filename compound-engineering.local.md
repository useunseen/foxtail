---
review_agents: [code-simplicity-reviewer, security-sentinel, performance-oracle, architecture-strategist]
plan_review_agents: [code-simplicity-reviewer]
---

# Review Context

Rust service that generates and serves AWS-like mock Cost Explorer and CloudWatch data over Axum with SQLite storage.
Focus reviews on API correctness, admin-surface security, SQLite query safety, and parity between generated data and served responses.
