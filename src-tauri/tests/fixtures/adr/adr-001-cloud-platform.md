# ADR-001: Cloud platform — Microsoft Azure

## Status
Accepted

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

**Cited sections:** S1, S2

## Decision
The chosen option is **Microsoft Azure** (Adopt), with a Jev probability of 0.85 and confidence 0.92.

Route: **Proposed**. Reason codes: none.

## Bistec Alignment
- Default Stack: Follows the BISTEC default
- Cloud Platform: Microsoft Azure
- Cost Impact: Tight: less than $500/mo. — cost fit score: 3.00/4

## Alternatives Considered
| Option | Ring | Fit with non-functional requirements | Team skill fit | Cost fit | BISTEC alignment | Freedom from lock-in | Security posture | Composite | Probability |
|---|---|---|---|---|---|---|---|---|---|
| Microsoft Azure | Adopt | 4.00 | 3.00 | 3.00 | 4.00 | 2.00 | 4.00 | 0.860 | 0.85 |
| Hetzner Cloud | Trial | 2.00 | 2.00 | 4.00 | 3.00 | 3.00 | 3.00 | 0.625 | 0.10 |
| Azure + Hetzner (hybrid) | Trial | 3.00 | 2.00 | 3.00 | 3.00 | 2.00 | 3.00 | 0.675 | 0.05 |

## Consequences
- Positive: Fit with non-functional requirements (4.00/4); Team skill fit (3.00/4); Cost fit (3.00/4); BISTEC alignment (4.00/4); Security posture (4.00/4)
- Negative: None
- Risks: None

## Cost Analysis
Budget band: Tight: less than $500/mo.

| Option | Cost fit |
|---|---|
| Microsoft Azure | 3.00/4 |
| Hetzner Cloud | 4.00/4 |
| Azure + Hetzner (hybrid) | 3.00/4 |

Monthly estimate: to be completed by architect

## Review
| Time (UTC) | Reviewer | Action | Option | Reason |
|---|---|---|---|---|
| 2026-09-20T10:00:00Z | Alex Architect | Accept | — | — |

---
### Evidence
- Model snapshot: typesafe/jev-1.13-2026-01-01
- Request hash: hash-cloud-platform
- Decision id: d-cloud-platform
- Gates: is_technical_request=0.95, has_enough_context=0.40, injection=0.10