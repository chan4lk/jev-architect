# Claims Portal Assessment — Report

**Date:** 2026-09-23

## Summary

- Decisions: 3 (Proposed: 2, Needs architect: 1)
- Review status: Accepted 1, Accepted (override) 1, Rejected 0, Unreviewed 1
- Total Jev cost: $3.46
- Input tokens: 12000

> Low context: the brief does not have enough context for confident decisions (has_enough_context < 0.5).

> Truncated: some evidence was dropped to fit the token budget.

## Brief

A small internal claims-processing tool for policyholder self-service.

| Dimension | Value |
|---|---|
| Scale | Small: fewer than 1,000 users. |
| Budget | Tight: less than $500/mo. |
| Timeline | Urgent: less than 4 weeks. |
| Team size | SoloPair: a solo developer or a pair. |
| Compliance | GDPR. |
| Data sensitivity | Confidential: sensitive business or personal data. |

**Requirements:**
- Support policyholder login via Microsoft 365 SSO (S1)

**Non-functional requirements:**
- Must handle 500 concurrent users at peak (S2)

**Constraints:**
- Budget capped at $400/month (S1)

**Team skills:**
- Team knows <b>C#</b> and Azure

**Mentioned technologies:** Azure, PostgreSQL

## Decisions

| Type | Choice | Ring | Confidence | Route | Status |
|---|---|---|---|---|---|
| Cloud platform | Microsoft Azure | Adopt | 0.92 | Proposed | Accepted |
| Message queue (simple) | Redis Pub/Sub | Trial | 0.60 | Proposed | Accepted (override) |
| Relational DB (budget) | MySQL | Hold | 0.40 | Needs architect | Proposed (AI) — pending approval |


### Cloud platform

- Chosen: **Microsoft Azure** (Adopt)
- Confidence: 0.92
- Route: Proposed
- Reasons: none

- Model snapshot: typesafe/jev-1.13-2026-01-01
- Cited sections: S1, S2

Probabilities:
- Microsoft Azure (Adopt): 0.85
- Hetzner Cloud (Trial): 0.10
- Azure + Hetzner (hybrid) (Trial): 0.05

| Option | Ring | Fit with non-functional requirements | Team skill fit | Cost fit | BISTEC alignment | Freedom from lock-in | Security posture | Composite |
|---|---|---|---|---|---|---|---|---|
| Microsoft Azure | Adopt | 4.00 | 3.00 | 3.00 | 4.00 | 2.00 | 4.00 | 0.860 |
| Hetzner Cloud | Trial | 2.00 | 2.00 | 4.00 | 3.00 | 3.00 | 3.00 | 0.625 |
| Azure + Hetzner (hybrid) | Trial | 3.00 | 2.00 | 3.00 | 3.00 | 2.00 | 3.00 | 0.675 |


### Message queue (simple)

- Chosen: **Redis Pub/Sub** (Trial)
- Confidence: 0.60
- Route: Proposed
- Reasons: none

- Model snapshot: typesafe/jev-1.13-2026-01-01
- Cited sections: none

Probabilities:
- Azure Queue Storage (Adopt): 0.30
- Redis Pub/Sub (Trial): 0.55
- Kafka (unless scale demands) (Hold): 0.15

| Option | Ring | Fit with non-functional requirements | Team skill fit | Cost fit | BISTEC alignment | Freedom from lock-in | Security posture | Composite |
|---|---|---|---|---|---|---|---|---|
| Azure Queue Storage | Adopt | 3.00 | 3.00 | 4.00 | 4.00 | 3.00 | 3.00 | 0.780 |
| Redis Pub/Sub | Trial | 3.00 | 4.00 | 4.00 | 2.00 | 3.00 | 2.00 | 0.700 |
| Kafka (unless scale demands) | Hold | — | — | — | — | — | — | — |


### Relational DB (budget)

- Chosen: **MySQL** (Hold)
- Confidence: 0.40
- Route: Needs architect
- Reasons: hold option, low confidence

- Model snapshot: typesafe/jev-1.13-2026-01-01
- Cited sections: S3

Probabilities:
- PostgreSQL (Hetzner) (Adopt): 0.30
- SQLite (tiny apps) (Trial): 0.25
- MySQL (Hold): 0.45

| Option | Ring | Fit with non-functional requirements | Team skill fit | Cost fit | BISTEC alignment | Freedom from lock-in | Security posture | Composite |
|---|---|---|---|---|---|---|---|---|
| PostgreSQL (Hetzner) | Adopt | 3.00 | 2.00 | 4.00 | 4.00 | 3.00 | 3.00 | 0.740 |
| SQLite (tiny apps) | Trial | 1.00 | 2.00 | 4.00 | 3.00 | 2.00 | 1.00 | 0.430 |
| MySQL | Hold | — | — | — | — | — | — | — |


## Not Applicable

| Type | Probability |
|---|---|
| Document DB | 0.20 |


---
Thresholds and weights are uncalibrated defaults until the golden-set run (FR-17) has been done against the live API.