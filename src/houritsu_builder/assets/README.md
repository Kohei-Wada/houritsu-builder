# houritsu

Every Japanese law (法令) as git history. One commit is one amending law coming into
force: `git log laws/act/415AC0000000057.md` is the amendment history of the Personal
Information Protection Act, and `git show <commit>` is what one amendment changed.

> **Before v1.0, history may be rebuilt and force-pushed.**

## Layout

- `laws/<law type>/<e-Gov law ID>.md`, with metadata in YAML frontmatter.
- `main`: enforced revisions. `upcoming`: promulgated but not yet in force.
- Commit date = enforcement date (00:00 JST). Trailers: `Amendment-Law-Id`, `Enforced`.

## Source

Generated from the [e-Gov law API](https://laws.e-gov.go.jp/) by
[houritsu-builder](https://github.com/Kohei-Wada/houritsu-builder). The statute text is
reproduced unchanged, but this is not the official text; the official text is the 官報.
Revision history is limited to what e-Gov provides.

## Lint

```sh
npx --yes markdownlint-cli2@0.23.3
```
