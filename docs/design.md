# houritsu: Japanese laws as git history — design

Date: 2026-10-16
Status: approved in conversation, pending spec review

## Goal

Publish every Japanese law (法令) as a git repository in which each commit is the
enforcement of one amending law. `git log` becomes the history of how the law
actually changed, `git diff` shows what an amendment changed, and tools such as a
future kokkai-cli can link a commit back to the bill and Diet debate behind it.

Prior art: [bundestag/gesetze](https://github.com/bundestag/gesetze) (German
federal law in git). [kissge/elaws-history](https://github.com/kissge/elaws-history)
snapshots the full e-Gov XML daily but has no per-amendment, human-readable history.

## Non-goals

- Rewriting or normalising statute text. The text is reproduced character for
  character; prose linters (textlint) never run on it.
- History before e-Gov's revision data starts (around 2017 for the laws sampled).
- Search, a web UI, or bill linkage. Those are separate tools that consume this data.

## Repositories

| Repository | Contents | Visibility |
| --- | --- | --- |
| `Kohei-Wada/houritsu` | Generated law markdown, README, rumdl config. Nothing else. | private until v1.0 and a terms-of-use check |
| `Kohei-Wada/houritsu-builder` | Fetch and build code, tests, this spec. | private |

Keeping the generator out of the data repository keeps `git log` in `houritsu`
purely the history of the law.

## Data source

e-Gov law API v2 (`https://laws.e-gov.go.jp/api/2`), no key required. Verified on
2026-10-16:

- `GET /laws?limit=&offset=` lists laws; `total_count` was 9,566.
- `GET /law_revisions/{law_id}` lists every revision of one law with
  `law_revision_id`, `amendment_law_id/num/title`, `amendment_promulgate_date`,
  `amendment_enforcement_date`, `current_revision_status`
  (`CurrentEnforced` / `PreviousEnforced` / `UnEnforced`) and `repeal_status`.
  The Personal Information Protection Act had 35 revisions back to 2017-03-01,
  including two scheduled for 2027 and 2028.
- `GET /law_data/{law_revision_id}?law_full_text_format=xml&response_format=json`
  returns the full text of one revision as base64 XML.

Sampled size (60 random laws): mean 5.1 revisions per law (median 2,
max 52), about 49,000 revisions in total, mean 100 KiB per revision, about 4.7 GiB
of XML, mean latency 1.2 s. A full serial crawl with 1 s spacing takes about 35 h.

## Architecture

```text
e-Gov law API v2
   │  fetch   (~35 h once, then daily deltas)
   ▼
raw XML cache  ~/.cache/houritsu/   (~4.7 GiB)
   │  build   (offline, deterministic)
   ▼
markdown + git fast-import stream
   │  git fast-import
   ▼
houritsu repository  →  GitHub
```

The builder is Rust (first written in Python; the Rust port produced byte-identical
history for all 53,432 revisions and renders about 6× faster per core). It has two
commands. Lint runs the pinned rumdl through `uvx`.

### fetch

1. List all laws.
2. For each law that may have changed (below), fetch its revision list and store it.
3. For each revision not yet cached, fetch the full text and store the raw response.

One file per response, so an interrupted run resumes where it stopped; laws never
fetched go first. Two workers fetch one law each; a worker waits 0.1 s between its
requests. Measured on 2026-10-17: responses take 0.1–0.2 s, so the single 1 s-spaced
client first used was ~90 % sleep; one worker at 0.1 s gave 110 revisions/min, four gave
only 175/min while e-Gov's empty-body failures rose from about 0.2 % to 1 %. Failures
are retried with exponential backoff from 2 s; revisions that still fail are recorded
and retried on the next run.

A run refetches a law's revision list only when it may have changed: the law is new,
the newest revision as of now or as of 2099-12-31 (`/laws?asof=`) is not in the cached
list with the same `updated`, a cached `UnEnforced` revision is due, or a full text
failed last time (other than with 404). A new amending
law found in a refetched list is looked up with `/laws?amendment_law_id=`, which lists
every law it amends, so a revision inserted mid-history is not missed. A run with
`--full` refetches every list and catches anything these checks miss; run it weekly.
Checked on 2026-10-17: both law lists take 41 s, and 0 of 8,611 cached laws were stale.

### build

Reads only the cache and rebuilds the whole history from scratch every time. Rendering
is reused: `rendered.json` in the cache maps each revision to the blob of the last linted
build, keyed by the revision's data file and metadata and by a hash of the sources rendering
depends on (render, fidelity, frontmatter, build, the rumdl config, `Cargo.lock`) and
the rumdl version, so a fetch-only change keeps them. A revision whose key is unchanged takes its blob from the
previous repository (borrowed through git alternates, then copied in by `git repack -a`)
and is neither rendered nor linted again. Workers write new blobs as loose objects, so
`git fast-import` only builds trees and commits.

1. Group revisions by amending law and enforcement date. One group is one commit:
   "this amending law came into force on this date and changed these files".
2. Order groups by enforcement date, then by amending law ID, so the order is total
   and stable.
3. Render each changed law to markdown and emit the commit through
   `git fast-import`.
4. Revisions with status `UnEnforced` go on branch `upcoming`, built on top of `main`.
   `main` holds enforced revisions only, so it never contains future-dated commits.

A law's earliest available revision is added in the commit of its own amending law,
the same as any later revision.

A repeal is a commit like any other. e-Gov lists repealed laws, and the repeal is a
revision with status `Repeal` whose amending law is the repealing law and whose
enforcement date is the repeal date. That revision deletes the file in the
repealing law's commit, so `HEAD` holds only laws in force and `git log -- <file>`
still shows the whole life of a law. Laws repealed but kept in force for transition
(`remain_in_force`) are deleted too. A repeal of a law with no earlier revision in
the data adds nothing, and later `Repeal` revisions of a deleted law are ignored.

## Repository layout (houritsu)

```text
laws/
  act/415/415AC0000000057.md
  cabinet-order/…
  ministerial-ordinance/…
  constitution/  rule/  …
README.md
.rumdl.toml
```

- Directory: the e-Gov `law_type`, lower-kebab-cased (the first type when e-Gov lists
  several, e.g. `Act,CabinetOrder`), then the first 3 characters of the law ID: the era
  and year of promulgation (`415` = 平成15年). GitHub lists at most 1,000 files per
  directory; this keeps every directory under about 220.
- File name: the e-Gov law ID. Titles can change on amendment; the ID does not.
- No index of titles: the frontmatter carries title, kana and law number, so an index is
  derived by tools outside this repository. An index here would change with every
  retitling and mix non-law changes into `git log`. The README shows `git grep` by title.

## File format

### Frontmatter

Only values that come from the API.

```yaml
---
law_id: 415AC0000000057
law_num: 平成十五年法律第五十七号
title: 個人情報の保護に関する法律
title_kana: こじんじょうほうのほごにかんするほうりつ
abbrev: 個人情報保護法
law_type: Act
category: 憲法
revision_id: 415AC0000000057_20261001_507AC0000000070
amendment:
  law_id: 507AC0000000070
  law_num: 令和七年法律第七十号
  title: 日本学術会議法
  promulgated: 2025-06-18
  enforced: 2026-10-01
repeal_status: None
source: https://laws.e-gov.go.jp/law/415AC0000000057
---
```

Keys are stable across builds; tools may rely on them.

### Body

- Part, chapter, section and subsection titles become headings. Articles become
  headings of the form `第一条（目的）`. Heading depth follows each law's own
  structure and always increases one level at a time (rumdl MD001).
- Paragraphs (項) become paragraphs; items (号) and sub-items become nested lists.
- Supplementary provisions (附則) and appended tables (別表) are included from the
  first version. Complex tables are first rendered as text and improved later.
- Statute text is not changed: full-width digits, brackets and spacing are kept.

## Commit format

```text
個人情報の保護に関する法律等の一部を改正する法律 施行

改正法令: 令和八年法律第五十六号
公布日: 2026-07-17
施行日: 2027-01-17

変更された法令:
- 個人情報の保護に関する法律（平成十五年法律第五十七号）

Amendment-Law-Id: 508AC0000000056
Enforced: 2027-01-17
```

- The last two lines are git trailers (`git log --format='%(trailers)'`) for tools.
- Author and committer dates: enforcement date, 00:00 +09:00. git rejects dates before 1970
  (`git commit` refuses them; a negative time from fast-import fails `git fsck` with
  `badDate`, checked with git 2.56 on 2026-10-17), so earlier dates become 1970-01-02;
  the `Enforced` trailer and frontmatter keep the real date.
- Author: the amending law's number with an empty email (`令和八年法律第五十六号 <>`), so
  `git shortlog` groups by amending law; committer: `houritsu-builder`. No address
  implies e-Gov wrote the commit; the source is credited in the README and frontmatter.

## Quality checks

Each check runs in tests and on every build; any failure fails the build.

- **Text fidelity**: the text extracted from the XML equals the text extracted from
  the generated markdown, character for character. This enforces "never change the
  statute text".
- **rumdl** (a markdownlint-compatible linter, pinned) on every generated file, with
  the config shipped in `houritsu` so consumers can run the same check. It replaced
  markdownlint-cli2, which took 170 s against rumdl's 13 s on the Act history and found
  the same (file, rule) pairs on 417 mutated files.
- **Determinism**: two builds from the same cache produce the same final commit hash.
- **Unknown XML elements** are rendered as plain text, never dropped, and counted
  in a build report so conversion rules can be added most-frequent first.

## Tests

- Golden tests: a few real law XML fixtures (small, with tables, with many
  supplementary provisions) compared against expected markdown.
- The quality checks above, run on the fixtures.

## Changing the format

- Until v1.0, history may be rebuilt from the cache and force-pushed. The README says so.
- From v1.0, history is never rewritten; corrections land as a new commit at HEAD,
  dated the day of the correction.

## Operations

- Repositories cloned with ghq under `~/ghq/github.com/Kohei-Wada/`.
- Cache at `~/.cache/houritsu/`.
- A systemd user timer runs fetch → build → push daily, once the format is stable.

## Milestones

1. Builder with fetch; start the full crawl in the background.
2. Build for Acts (法律) only from the partial cache, with all quality checks green;
   push to private `houritsu`.
3. Extend the build to all law types.
4. Daily timer.
5. Before going public: check the e-Gov terms of use (attribution, disclaimer that
   the official text is the 官報).
   Checked 2026-10-17: e-Gov 法令検索 content is under PDL1.0 (CC BY 4.0 compatible;
   https://laws.e-gov.go.jp/terms/). It requires a source line, a statement that the data
   was edited and by whom, and not presenting the result as made by the government. The
   README carries the source line in e-Gov's format, the edit statement, and the 官報
   disclaimer; houritsu is offered under the same terms. Statute text itself is not
   copyrighted (著作権法第13条).

## Open questions

- How far back revision history goes for older laws. The sample suggests 2017.
- Whether some laws exceed GitHub's file-size limits (largest sampled: 812 KiB of
  XML, well under).
