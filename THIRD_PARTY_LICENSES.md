# Third-party licenses

Roost itself is licensed under MIT OR Apache-2.0 (see `LICENSE-MIT` and `LICENSE-APACHE`). This file lists material bundled into the application that has a different license, and explains how to inspect the Rust dependency licenses.

## Bundled fonts

The GUI binary embeds these fonts. Both are licensed under the SIL Open Font License, Version 1.1. The full license texts, including each copyright notice, are kept next to the font files and must ship with any redistribution of the fonts.

| Font | Files embedded | License text |
|---|---|---|
| Hanken Grotesk | `Hanken-400.ttf`, `Hanken-600.ttf` | [`OFL-HankenGrotesk.txt`](crates/roost-gui/assets/OFL-HankenGrotesk.txt) |
| JetBrains Mono | `JetBrainsMono-400.ttf`, `JetBrainsMono-500.ttf` | [`OFL-JetBrainsMono.txt`](crates/roost-gui/assets/OFL-JetBrainsMono.txt) |

The font files are in `crates/roost-gui/assets/`. egui also bundles its own default fonts through the `epaint_default_fonts` crate (declared license: `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`).

## Rust dependencies

Cargo dependencies keep their own licenses. The set of licenses allowed in the dependency tree is enforced by `deny.toml` (checked by `cargo deny check licenses` and the `deny` workflow). To list the licenses of the current tree:

```sh
cargo metadata --format-version 1 | python3 -c "import json,sys; [print(p['name'], p['license']) for p in json.load(sys.stdin)['packages']]"
```

Where a dependency offers a choice of licenses (for example `MIT OR Apache-2.0`), this project uses it under the permissive option.

A binary distribution should include the license notices required by its dependencies. Generating a combined notice file for releases (for example with `cargo-about`) is not set up yet.
