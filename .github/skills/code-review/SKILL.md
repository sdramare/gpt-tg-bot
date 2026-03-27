---
name: code-review
description: "Use when: reviewing current workspace code for defects, risks, regressions, and maintainability issues. Produces only a findings list with major and minor issues; no summary, no questions."
---

# Code Review

Review the current workspace code only.

Do not review pull requests, commits, branches, diffs, or external repositories.

## Output Contract

Return only one of the following:

1. If one or more major issues exist:
- `Major`
- List each major issue as one bullet.
- `Minor`
- List each minor issue as one bullet, or `- None` if there are no minor issues.

2. If no major issues exist:
- `looks good`

## Rules

- No summary section.
- No open questions.
- No recommendations section.
- No extra commentary before or after the findings.
- Keep findings concrete and code-focused.
- Prefer bug risk, security, correctness, reliability, and test coverage gaps.

## Review Workflow

1. Inspect the relevant workspace files.
2. Identify correctness and behavior risks first.
3. Separate findings by severity:
- Major: can cause failures, incorrect behavior, data/security impact, or significant regressions.
- Minor: readability, maintainability, small robustness gaps, or low-risk test gaps.
4. Emit output strictly following the Output Contract.
