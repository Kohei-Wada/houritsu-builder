# houritsu

Every Japanese law (法令) as git history. One commit is one amending law coming into
force: `git log laws/act/415/415AC0000000057.md` is the amendment history of the Personal
Information Protection Act, and `git show <commit>` is what one amendment changed.

> **Before v1.0, history may be rebuilt and force-pushed.**

## Layout

- `laws/<law type>/<first 3 characters of the ID>/<e-Gov law ID>.md`, with metadata in YAML
  frontmatter. The first 3 characters are the era and year of promulgation (`415` = 平成15年).
- Tables are HTML (merged cells kept). Figures and 様式 are links to the attachment on e-Gov,
  on a line of their own, or as an HTML link inside a sentence or a table cell. About 3 in
  10 figures have no attachment there and show `（図：e-Gov に画像なし）` instead.
- The table of contents (目次) is left out: it only repeats the headings.
- `main`: enforced revisions. `upcoming`: promulgated but not yet in force.
- Commit date = enforcement date (00:00 JST). Trailers: `Amendment-Law-Id`, `Enforced`;
  for an amending Act, also `Bill` (the Diet bill) and `Bill-Url` (its text on the House of
  Representatives site, where the bill's 理由 is). `Skipped-Revision` names a revision
  e-Gov has no text for; the commit's diff covers it too.
  git cannot record dates before 1970, so earlier ones are recorded as 1970-01-02; the real
  date is in the `Enforced` trailer and the frontmatter.
- An amendment whose enforcement day is not fixed yet has `enforced: "undetermined"` and
  `Enforced: undetermined`, and is the last commit on `upcoming` (dated 2117-12-31, the
  placeholder e-Gov uses). `enforcement_comment` in the frontmatter words the day as the
  amending law does (e.g. 政令で定める日); with it, an `upcoming` date is e-Gov's estimate.
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
- **A law enters at its oldest e-Gov revision.** Checking out an old date shows no file for
  a law e-Gov covers only later, not its older text: about 2,800 laws at 2010-01-01 against
  9,000 now.
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

## Following one article

Heading depth depends on the chapters around an article, so match any depth:

```sh
git log -L '/^##*  *第十六条/,/^##*  *第十七条/:laws/act/415/415AC0000000057.md'
```

This takes the first match, the main text. For an article of a 附則, start the range at
that 附則's heading, which names its amending law: `/^## 附　則（令和二年/,/^## 附　則/`.

## 出典 / Source

出典：e-Gov法令検索（https://laws.e-gov.go.jp/）、PDL1.0（https://www.digital.go.jp/resources/open_data/public_data_license_v1.0）

e-Gov法令検索の法令データ（法令API v2）を [houritsu-builder](https://github.com/Kohei-Wada/houritsu-builder)（Kohei-Wada）が markdown に加工して作成。
条文の文字は変更していない（markdown のための見出し記号とエスケープ、図へのリンクを加えた）。正本は官報。改正履歴は e-Gov が提供する範囲に限る。

議案との対応：[衆議院議案データベース](https://github.com/smartnews-smri/house-of-representatives)（smartnews-smri、MIT）。議案番号と本文ページへのリンクのみ使用。

The characters of the statute text are unchanged; headings, markdown escapes and figure
links are added. This is not the official text; the official text is the 官報. Revision
history is limited to what e-Gov provides.

## License

[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), which PDL1.0 §1.7 allows for
the source data. Attribute as in the section above. The statute text itself is not
copyrighted (著作権法第13条).

No warranty: this is not legal advice, and the text may be out of date or wrong. Check
e-Gov or the 官報 before relying on it.

## Lint

```sh
uvx rumdl@0.2.79 check
```
