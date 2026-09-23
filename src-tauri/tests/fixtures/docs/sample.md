# Customer Self-Service Portal — Requirements Overview

## Overview

This document captures the requirements for a new customer self-service portal for a regional insurer client of BISTEC Global. Policyholders will use the portal to view policy documents, file and track claims, and update contact and billing details without calling the contact centre.

## Scope and Users

The portal must support approximately 20,000 registered policyholders at launch, with headroom for growth as the insurer migrates further policy lines onto the platform. Peak concurrent usage is expected around monthly billing cycles and renewal periods, so the system should handle bursty traffic without manual intervention.

## Authentication

Corporate staff and agents will sign in with the insurer's existing Microsoft 365 tenant via single sign-on (SSO); policyholders will use a separate, portal-specific login. The authentication design should reuse BISTEC's default Microsoft identity platform integration rather than a bespoke identity provider.

## Budget and Timeline

The client has stated a tight budget for this engagement and prefers to reuse managed cloud services over self-hosted infrastructure wherever the cost difference is significant. A first release is targeted within one quarter, covering policy viewing and claims status only; billing updates and document uploads can follow in a later phase.

## Team and Delivery

The insurer's in-house engineering team is a .NET team with several years of experience running ASP.NET Core services in Azure, and would maintain the solution after handover. BISTEC will pair with the client team during delivery so that ownership can transfer smoothly at the end of the engagement.

## Compliance

The portal processes personal and policy data for EU-resident customers, so the solution must meet GDPR requirements for data minimisation, consent, and the right to erasure. Audit logging of access to policy and claims records is required for the insurer's compliance reporting.

## Non-Functional Requirements

The portal should remain available during the insurer's business hours with no planned downtime for routine deployments. Response times for common actions such as viewing a policy or checking claim status should stay under two seconds under normal load.
