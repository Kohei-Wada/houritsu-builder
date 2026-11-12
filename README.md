# houritsu-builder

Builds [houritsu](https://github.com/Kohei-Wada/houritsu): every Japanese law as git
history, one commit per amending law, generated from the e-Gov law API v2.

```sh
cargo build --release
./target/release/houritsu-builder fetch                 # what changed since the last run; --full refetches everything
./target/release/houritsu-builder build --out ../houritsu-out --law-type Act
```

Design: `docs/design.md`.

## Verifying a law file

`verify` checks that a file in a houritsu checkout is what e-Gov serves now. It refetches the
revision named in the file's frontmatter (`law_id`, `revision_id`) from the e-Gov law API v2,
renders it with the build's code and compares the bytes. No API key is needed, and it writes
nothing to the cache or the checkout.

```sh
cd houritsu
houritsu-builder verify laws/act/415/415AC0000000057.md     # a file
houritsu-builder verify 415AC0000000057                     # a law ID, looked up in --repo (default .)
houritsu-builder verify 423M60000008087 --revision 423M60000008087_20160729_428M60000008077
```

`--revision` checks that revision from the history of HEAD instead of the file as it is. The
output is `OK <file>` (exit 0), or `MISMATCH <file>: <reason>` and a short diff from the file to
e-Gov's version (exit 1). Errors exit 2. The reason is one of:

- `local content differs`: the file is not as committed in the checkout.
- `e-Gov updated since the build`: e-Gov updated the revision after the checkout got its HEAD
  commit (from the reflog), so the build predates the update, or e-Gov no longer lists it.
- `the commit differs from e-Gov`: e-Gov's last update is older than that. Either the commit was
  changed, or the build used text fetched before the update: a daily fetch can miss an update to
  an old revision until the weekly full one.

## License

MIT. For the houritsu data it builds, see the License section of its README.
