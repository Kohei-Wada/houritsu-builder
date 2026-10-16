import re
from dataclasses import dataclass, field

from houritsu_builder.cache import Cache


def kebab(law_type: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "-", law_type).lower()


@dataclass(frozen=True)
class Revision:
    law_id: str
    law_num: str
    law_type: str
    law_info: dict = field(compare=False, hash=False)
    rev: dict = field(compare=False, hash=False)

    @property
    def revision_id(self) -> str:
        return self.rev["law_revision_id"]

    @property
    def title(self) -> str:
        return self.rev.get("law_title") or self.law_id

    @property
    def amendment_law_id(self) -> str:
        return self.rev.get("amendment_law_id") or self.law_id

    @property
    def enforced(self) -> str | None:
        return self.rev.get("amendment_enforcement_date")

    @property
    def promulgated(self) -> str | None:
        return self.rev.get("amendment_promulgate_date")

    @property
    def upcoming(self) -> bool:
        return self.rev.get("current_revision_status") == "UnEnforced"

    @property
    def repealed(self) -> bool:
        return self.rev.get("current_revision_status") == "Repeal"

    @property
    def path(self) -> str:
        return f"laws/{kebab(self.law_type)}/{self.law_id}.md"


@dataclass
class Group:
    amendment_law_id: str
    enforced: str | None
    revisions: list[Revision]

    @property
    def upcoming(self) -> bool:
        return any(r.upcoming for r in self.revisions)

    @property
    def commit_date(self) -> str:
        first = self.revisions[0]
        return self.enforced or first.promulgated or "1970-01-02"

    def message(self) -> str:
        first = self.revisions[0].rev
        title = first.get("amendment_law_title") or self.revisions[0].title
        lines = [
            f"{title} 施行",
            "",
            f"改正法令: {first.get('amendment_law_num') or '不明'}",
            f"公布日: {first.get('amendment_promulgate_date') or '不明'}",
            f"施行日: {self.enforced or '未定'}",
            "",
            "変更された法令:",
            *[f"- {r.title}（{r.law_num}）{'（廃止）' if r.repealed else ''}" for r in self.revisions],
            "",
            f"Amendment-Law-Id: {self.amendment_law_id}",
            f"Enforced: {self.enforced or 'unknown'}",
        ]
        return "\n".join(lines) + "\n"


def load_revisions(cache: Cache, law_type: str | None = None) -> tuple[list[Revision], list[str], list[str]]:
    revisions, excluded, skipped = [], [], []
    not_found = cache.not_found()
    for path in cache.revision_files():
        doc = cache.read(path)
        info = doc["law_info"]
        if law_type and info.get("law_type") != law_type:
            continue
        revs = [Revision(info["law_id"], info.get("law_num") or "", r.get("law_type") or info["law_type"], info, r) for r in doc["revisions"]]
        if not revs:
            continue
        missing = {r.revision_id for r in revs if not r.repealed and not cache.data_path(r.revision_id).exists()}
        if missing - not_found:
            excluded.append(info["law_id"])
            continue
        skipped += [r.revision_id for r in revs if r.revision_id in missing]
        revisions += [r for r in revs if r.revision_id not in missing]
    return revisions, excluded, skipped


def _drop_redundant_repeals(revisions: list[Revision]) -> list[Revision]:
    kept, alive = [], set()
    for r in sorted(revisions, key=lambda r: (r.enforced or r.promulgated or "", r.amendment_law_id)):
        if r.repealed:
            if r.law_id not in alive:
                continue
            alive.discard(r.law_id)
        else:
            alive.add(r.law_id)
        kept.append(r)
    return kept


def plan(revisions: list[Revision]) -> tuple[list[Group], list[Group]]:
    groups: dict[tuple, Group] = {}
    for r in _drop_redundant_repeals(revisions):
        key = (r.enforced, r.amendment_law_id)
        groups.setdefault(key, Group(r.amendment_law_id, r.enforced, [])).revisions.append(r)
    for g in groups.values():
        g.revisions.sort(key=lambda r: r.path)
    ordered = sorted(groups.values(), key=lambda g: (g.commit_date, g.amendment_law_id))
    return [g for g in ordered if not g.upcoming], [g for g in ordered if g.upcoming]
