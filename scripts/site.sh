#!/usr/bin/env bash
# Assemble the public documentation site for GitHub Pages: the user guide(s) (mdBook)
# plus the public API reference (rustdoc), under one project site served at
# https://londey.github.io/systemrs/. The site is built into target/site/ and uploaded
# by the Pages workflow as an artifact — nothing built is ever committed. Run
# `just site`, then open target/site/index.html (or serve target/site/) to preview
# exactly what CI publishes.
#
# Layout (mirrors the published URL structure):
#   target/site/index.html  -> landing page
#   target/site/guide/      -> the intro guide        (/systemrs/guide/)
#   target/site/api/        -> the API reference       (/systemrs/api/)
# A future migration guide drops in as target/site/migration/ with one more build+copy.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

out="${1:-target/site}"
rm -rf "$out"
mkdir -p "$out"

# --- The intro user guide -> /guide ---
mdbook build doc/guide                       # renders to target/guide (see book.toml)
cp -r target/guide "$out/guide"

# (Future: the migration guide -> /migration)
# mdbook build doc/migration
# cp -r target/migration "$out/migration"

# --- The public API reference (rustdoc, public items only) -> /api ---
# Note: public docs deliberately omit --document-private-items (unlike scripts/doc.sh,
# which is the dev/CI doc check).
cargo doc --no-deps
# rustdoc emits no root index for a workspace; redirect to the facade crate so /api/
# lands somewhere useful. All rustdoc links are relative, so this works at any subpath.
printf '<!doctype html><meta http-equiv="refresh" content="0; url=systemrs/index.html">\n' \
    >target/doc/index.html
cp -r target/doc "$out/api"

# --- Landing page -> / ---
cat >"$out/index.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>SystemRS Documentation</title>
<style>
  :root { color-scheme: light dark; }
  body { font: 16px/1.6 system-ui, sans-serif; max-width: 42rem; margin: 4rem auto; padding: 0 1.25rem; }
  h1 { margin-bottom: 0.25rem; }
  p.lead { color: #777; margin-top: 0; }
  ul { list-style: none; padding: 0; }
  li { margin: 1rem 0; padding: 1rem 1.25rem; border: 1px solid #8884; border-radius: 8px; }
  a { text-decoration: none; font-weight: 600; font-size: 1.1rem; }
  .desc { display: block; font-weight: 400; font-size: 0.95rem; color: #888; margin-top: 0.25rem; }
</style>
</head>
<body>
<h1>SystemRS</h1>
<p class="lead">A Rust, TLM-only equivalent of SystemC for transaction-level digital twins.</p>
<ul>
  <li><a href="guide/">The SystemRS Guide</a>
    <span class="desc">An introductory, example-driven tutorial — start here.</span></li>
  <li><a href="api/">API Reference</a>
    <span class="desc">The rustdoc for the public API.</span></li>
</ul>
<p><a href="https://github.com/londey/systemrs">Source on GitHub</a></p>
</body>
</html>
HTML

printf 'Site assembled at %s (guide/, api/, index.html).\n' "$out"
