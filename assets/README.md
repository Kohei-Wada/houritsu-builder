# houritsu

Every Japanese law (法令) as git history. One commit is one amending law coming into
force: `git log laws/act/415/415AC0000000057.md` is the amendment history of the Personal
Information Protection Act, and `git show <commit>` is what one amendment changed.

> **Before v1.0, history may be rebuilt and force-pushed.**

## Layout

- `laws/<law type>/<first 3 characters of the ID>/<e-Gov law ID>.md`, with metadata in YAML
  frontmatter. The first 3 characters are the era and year of promulgation (`415` = 平成15年).
- Tables are HTML (merged cells kept). Figures and 様式 are links to the attachment on e-Gov;
  about 3 in 10 figures have no attachment there and are missing.
- `main`: enforced revisions. `upcoming`: promulgated but not yet in force.
- Commit date = enforcement date (00:00 JST). Trailers: `Amendment-Law-Id`, `Enforced`;
  for an amending Act, also `Bill` (the Diet bill) and `Bill-Url` (its text on the House of
  Representatives site, where the bill's 理由 is).
  git cannot record dates before 1970, so earlier ones are recorded as 1970-01-02; the real
  date is in the `Enforced` trailer and the frontmatter.
- A repealed law's file is deleted in the repealing law's commit; its history stays in
  `git log -- <file>`.

## Limits

- **History starts around 2015, not at enactment.** e-Gov's revision data ramps up from
  about 2015 (a few hundred commits a year before, about two thousand a year from 2019), so
  a time series of commit dates measures e-Gov's coverage as much as legislation.
- **`git blame` dates the coverage, not the wording.** A law's oldest commit holds every
  line that existed before coverage began: most of 刑法 blames to its 2016 commit. Read it
  as "already there by then".
- **Many laws have little history.** About a third of laws have a single commit.
- **Commit SHAs are not stable before v1.0.** A rebuild keeps them only while the data
  and the builder are unchanged; a builder change, or e-Gov adding a revision dated before
  existing commits, rewrites every later SHA. Refer to a revision by its `law_id` and
  `revision_id` (both in the frontmatter) instead.

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

議案との対応：[衆議院議案データベース](https://github.com/smartnews-smri/house-of-representatives)（smartnews-smri、MIT）。議案番号と本文ページへのリンクのみ使用。

The statute text is reproduced unchanged, but this is not the official text; the official
text is the 官報. Revision history is limited to what e-Gov provides.

## License

Same terms as the source data: PDL1.0, compatible with CC BY 4.0. Attribute as in the
section above. The statute text itself is not copyrighted (著作権法第13条).

## Lint

```sh
uvx rumdl@0.2.79 check
```
