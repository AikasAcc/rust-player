# Third-party notices

Rust Player is released under the MIT License (`LICENSE`). It is built from the
Rust ecosystem and links third-party crates.

`Cargo.lock` records the resolved package names, versions, sources and checksums;
it does **not** record license expressions. Declared license expressions come from
each crate's own manifest and can be inspected with `cargo metadata --locked`.
Licenses that apply to a built binary can depend on the selected features and the
final dependency graph, so this notice does not assign a single license to the
whole application.

The direct dependencies are eframe/egui, rfd, CPAL, Symphonia, rtrb, Rubato,
Lofty, image, Serde/serde_json, tempfile, walkdir, rand, ureq, encoding_rs, url,
Rhai, ttf-parser, raw-window-handle, the Windows crate, bs1770, ape-decoder,
dsd-reader, dsd-pcm, id3 and wavpack; winresource is a build dependency on
Windows. Their transitive dependencies are resolved in `Cargo.lock`. The
`wavpack` crate builds the bundled reference libwavpack, which is distributed
under the BSD-3-Clause license; its upstream license text is collected with the
other crate license texts during packaging.

Each dependency remains subject to its own terms. This notice does not replace
the license texts distributed by the upstream projects. The release script
generates `THIRD-PARTY-LICENSES.txt` from the local Cargo package sources used
for that build: it copies LICENSE/LICENCE/COPYING/NOTICE files verbatim where
they exist, and lists the declared license plus the crates.io URL where a crate
does not ship a license file in its package source. No license text is invented
here. The complete upstream terms, including current versions, remain available
from the crate source or its crates.io record.
