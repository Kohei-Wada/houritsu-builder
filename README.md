# houritsu-builder

Builds [houritsu](https://github.com/Kohei-Wada/houritsu): every Japanese law as git
history, one commit per amending law, generated from the e-Gov law API v2.

```sh
cargo build --release
./target/release/houritsu-builder fetch                 # what changed since the last run; --full refetches everything
./target/release/houritsu-builder build --out ../houritsu-out --law-type Act
```

Design: `docs/design.md`.

Lint runs `uvx rumdl@0.2.79`, so `uv` must be on `PATH`. `cargo test` needs it too.
