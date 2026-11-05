# Icon font

`FluentSystemIcons-Regular.ttf` is the unmodified Regular icon font from
[Microsoft Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons).
Fira Code remains the text font, shared from the workspace's `assets/fonts`.

- Upstream commit: `9bfae3d782fda4c8764f2128cec4d0dda160b283`
- Source paths: `fonts/FluentSystemIcons-Regular.ttf`,
  `fonts/FluentSystemIcons-Regular.json`, and `LICENSE`
- Font SHA-256: `af1b71ab7bb475ae30b8c87172551503f8de6cf733548fe7ddbda15fc08dea34`
- Licence: MIT; the complete copyright and permission notice is in
  `LICENSE-FluentSystemIcons.txt`.

The JSON file preserves the upstream glyph-to-codepoint mapping. The settings
cogwheel uses `ic_fluent_settings_24_regular`, U+F6AA, drawn with the icon font.
These private-use codepoints are specific to this font, not ordinary Unicode
gear characters.

`cargo xtask bundle` includes `LICENSE-FluentSystemIcons.txt` in VST3
`Contents/Resources` and beside the single-file CLAP bundles. Include this notice
with standalone distributions as well as the shared text font's `OFL.txt`.
