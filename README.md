# houritsu-builder

Builds [houritsu](https://github.com/Kohei-Wada/houritsu): every Japanese law as git
history, one commit per amending law, generated from the e-Gov law API v2.

```sh
uv run houritsu-builder fetch                 # crawl into ~/.cache/houritsu (resumable)
uv run houritsu-builder build --out ../houritsu-out --law-type Act
```

Design: `docs/design.md`.
