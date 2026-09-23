# ADR-002: Message queue (simple) — Redis Pub/Sub

## Status
Accepted (override) — Azure Queue Storage

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

**Cited sections:** None

## Decision
The chosen option is **Redis Pub/Sub** (Trial), with a Jev probability of 0.55 and confidence 0.60.

Route: **Proposed**. Reason codes: none.

The reviewer overrode this decision in favor of **Azure Queue Storage**: Team already runs Azure Queue Storage elsewhere; avoid new infra.

## Bistec Alignment
- Default Stack: Deviation: BISTEC alternative — rationale required (see Review)
- Cloud Platform: Microsoft Azure
- Cost Impact: Tight: less than $500/mo. — cost fit score: 4.00/4

## Alternatives Considered
| Option | Ring | Fit with non-functional requirements | Team skill fit | Cost fit | BISTEC alignment | Freedom from lock-in | Security posture | Composite | Probability |
|---|---|---|---|---|---|---|---|---|---|
| Azure Queue Storage | Adopt | 3.00 | 3.00 | 4.00 | 4.00 | 3.00 | 3.00 | 0.780 | 0.30 |
| Redis Pub/Sub | Trial | 3.00 | 4.00 | 4.00 | 2.00 | 3.00 | 2.00 | 0.700 | 0.55 |
| Kafka (unless scale demands) | Hold | — | — | — | — | — | — | — | 0.15 |

## Consequences
- Positive: Fit with non-functional requirements (3.00/4); Team skill fit (4.00/4); Cost fit (4.00/4); Freedom from lock-in (3.00/4)
- Negative: None
- Risks: None

## Cost Analysis
Budget band: Tight: less than $500/mo.

| Option | Cost fit |
|---|---|
| Azure Queue Storage | 4.00/4 |
| Redis Pub/Sub | 4.00/4 |
| Kafka (unless scale demands) | — |

Monthly estimate: to be completed by architect

## Review
| Time (UTC) | Reviewer | Action | Option | Reason |
|---|---|---|---|---|
| 2026-09-21T09:30:00Z | Alex Architect | Override | Azure Queue Storage | Team already runs Azure Queue Storage elsewhere; avoid new infra. |

---
### Evidence
- Model snapshot: typesafe/jev-1.13-2026-01-01
- Request hash: hash-message-queue-simple
- Decision id: d-message-queue-simple
- Gates: is_technical_request=0.95, has_enough_context=0.40, injection=0.10