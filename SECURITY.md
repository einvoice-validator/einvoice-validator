# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately, through GitHub's private vulnerability reporting:
[open a report](https://github.com/einvoice-validator/einvoice-validator/security/advisories/new)
(the "Report a vulnerability" button of the repository's Security tab). Do not open a public
issue for a vulnerability.

Useful in a report: the affected crate and version (or commit), a minimal input that triggers
the problem, and what happens (crash, hang, excessive memory, a wrong validation result).

The report stays private between you and the maintainers until a fix is released; you are
credited in the advisory unless you prefer otherwise.

## Scope

Before 1.0.0, fixes are made on the `main` branch and released as a new version of the
latest release; older versions are not patched. Validation code parses untrusted XML documents, so crashes, unbounded resource use and
validation results that differ from the official rule sets are all in scope. Vulnerabilities
in the official validation artefacts themselves belong to their publishers.
