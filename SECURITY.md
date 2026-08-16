# Security Policy

## Supported Versions

`reqsmith` is currently in initial development (pre-1.0). Security fixes are
provided for the latest `0.1.x` release only.

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub
issues.**

If you believe you've found a security vulnerability in `reqsmith` (for
example, unsafe handling of credentials, environment variables, or
plugin/WASM sandboxing issues), please report it privately using one of the
following channels:

1. **GitHub Private Vulnerability Reporting** (preferred): open a report via
   the "Security" tab of this repository
   (`https://github.com/nabobery/reqsmith/security/advisories/new`). This
   creates a private advisory that only maintainers can see.
2. **Email**: send details to **avinash@onarrival.travel**.

Please include as much of the following as you can:

- A description of the vulnerability and its potential impact.
- Steps to reproduce, or a minimal proof-of-concept.
- Affected version(s) or commit.

### What to expect

- We aim to acknowledge new reports within **5 business days**.
- We'll work with you to understand and validate the issue, and aim to
  provide an initial assessment (fix timeline, severity) within **14 days**.
- We'll credit reporters in the release notes/advisory unless you'd prefer
  to remain anonymous.

## Handling sensitive data in reports and examples

`reqsmith` is an HTTP client that routinely handles authentication tokens, API
keys, cookies, and request/response bodies. When filing issues, discussions,
or pull requests (including security reports made through public channels
by mistake):

- **Never paste real tokens, credentials, secrets, or captured
  request/response bodies from a live system into a public issue, PR, or
  discussion.**
- Redact or replace sensitive values with clearly synthetic placeholders
  (e.g. `Authorization: Bearer <REDACTED>`), or construct a minimal
  reproduction using fake data.
- If you accidentally post real credentials anywhere in this repository,
  rotate/revoke them immediately in addition to asking a maintainer to
  remove the post.

For genuine vulnerability reports where sharing a real capture is
unavoidable, use the private reporting channels above, not a public issue.
