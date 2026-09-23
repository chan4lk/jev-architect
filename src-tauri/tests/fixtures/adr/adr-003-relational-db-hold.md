# ADR-003: Relational DB (budget) — MySQL

## Status
Proposed (AI) — pending approval

## Date
2026-09-23

## Context
A small internal claims-processing tool for policyholder self-service.

| Dimension | Value |
|---|---|
| Scale | Small: fewer than 1,000 users. |
| Budget | Tight: less than $500/mo. |
| Timeline | Urgent: less than 4 weeks. |
| Team size | SoloPair: a solo developer or a pair. |
| Compliance | GDPR. |
| Data sensitivity | Confidential: sensitive business or personal data. |

**Constraints:**
- Budget capped at $400/month (S1)

**Non-functional requirements:**
- Must handle 500 concurrent users at peak (S2)

**Cited sections:** S3

## Decision
The chosen option is **MySQL** (Hold), with a Jev probability of 0.45 and confidence 0.40.

Route: **Needs architect**. Reason codes: hold option, low confidence.

## Bistec Alignment
- Default Stack: Breach: BISTEC says avoid — architect approval required
- Cloud Platform: Microsoft Azure
- Cost Impact: Tight: less than $500/mo. — cost fit score: —

## Alternatives Considered
| Option | Ring | Fit with non-functional requirements | Team skill fit | Cost fit | BISTEC alignment | Freedom from lock-in | Security posture | Composite | Probability |
|---|---|---|---|---|---|---|---|---|---|
| PostgreSQL (Hetzner) | Adopt | 3.00 | 2.00 | 4.00 | 4.00 | 3.00 | 3.00 | 0.740 | 0.30 |
| SQLite (tiny apps) | Trial | 1.00 | 2.00 | 4.00 | 3.00 | 2.00 | 1.00 | 0.430 | 0.25 |
| MySQL | Hold | — | — | — | — | — | — | — | 0.45 |

## Consequences
- Positive: None
- Negative: None
- Risks: hold option; low confidence

## Cost Analysis
Budget band: Tight: less than $500/mo.

| Option | Cost fit |
|---|---|
| PostgreSQL (Hetzner) | 4.00/4 |
| SQLite (tiny apps) | 4.00/4 |
| MySQL | — |

Monthly estimate: to be completed by architect

## Review
No reviews yet

---
### Evidence
- Model snapshot: typesafe/jev-1.13-2026-01-01
- Request hash: hash-relational-db-budget
- Decision id: d-relational-db-budget
- Gates: is_technical_request=0.95, has_enough_context=0.40, injection=0.10