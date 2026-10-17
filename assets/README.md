# houritsu

Every Japanese law (法令) as git history. One commit is one amending law coming into
force: `git log laws/act/415/415AC0000000057.md` is the amendment history of the Personal
Information Protection Act, and `git show <commit>` is what one amendment changed.

> **Before v1.0, history may be rebuilt and force-pushed.**

## Layout

- `laws/<law type>/<first 3 characters of the ID>/<e-Gov law ID>.md`, with metadata in YAML
  frontmatter. The first 3 characters are the era and year of promulgation (`415` = 平成15年).
- `main`: enforced revisions. `upcoming`: promulgated but not yet in force.
- Commit date = enforcement date (00:00 JST). Trailers: `Amendment-Law-Id`, `Enforced`.
  git cannot record dates before 1970, so earlier ones are recorded as 1970-01-02; the real
  date is in the `Enforced` trailer and the frontmatter.
- A repealed law's file is deleted in the repealing law's commit; its history stays in
  `git log -- <file>`.

## Finding a law

File names are e-Gov law IDs, which stay the same when a title changes. Search the
frontmatter by title:

```sh
git grep -l '^title: "個人情報の保護に関する法律"' -- laws
```

## 出典 / Source

出典：e-Gov法令検索（https://laws.e-gov.go.jp/）、PDL1.0（https://www.digital.go.jp/resources/open_data/public_data_license_v1.0）

e-Gov法令検索の法令データ（法令API v2）を [houritsu-builder](https://github.com/Kohei-Wada/houritsu-builder)（Kohei-Wada）が markdown に加工して作成。
条文は変更していないが、正本は官報。改正履歴は e-Gov が提供する範囲に限る。

The statute text is reproduced unchanged, but this is not the official text; the official
text is the 官報. Revision history is limited to what e-Gov provides.

## License

Same terms as the source data: PDL1.0, compatible with CC BY 4.0. Attribute as in the
section above. The statute text itself is not copyrighted (著作権法第13条).

## Lint

```sh
uvx rumdl@0.2.79 check
```
