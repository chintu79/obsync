# Security Policy

Obsync moves your vault between devices over your LAN. Data safety and
transport security are the top priorities of this project.

## Supported Versions

Only the latest tagged release receives security fixes. Backports to older
releases are handled case-by-case.

| Version | Supported          |
| ------- | ------------------ |
| latest  | :white_check_mark: |
| older   | :x:                |

## Reporting a Vulnerability

Please **do not** open a public issue for security problems. Report privately:

- **Email:** create an issue using GitHub's "Report a vulnerability" flow at
  <https://github.com/chintu79/obsync/security/advisories> (recommended), or
  contact the maintainers directly via the repository's owner.

Include as much as you can:

- The affected component (`core`, `httpd`, `android`, `protocol`)
- A minimal reproduction (vault layout, pairing state, network topology)
- The impact you believe this has (data loss? remote code execution? spoofing?)

You'll receive a response within 72 hours with next steps. We ask that you
give us time to fix and release before publicly disclosing.

## What we take seriously

- Data loss or vault corruption from the sync engine
- A malicious peer gaining unauthorized access to a vault
- Secrets/keys exposure in transport, pairing, or on disk
- Anything that lets an unapproved device read or write vault content

## How we handle fixes

1. Acknowledge the report within 72 hours.
2. Fix on a private branch, verify, and release a patched tag.
3. Credit the reporter (unless you prefer anonymity) in the release notes.
