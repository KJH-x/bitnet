# Desktop font assets

The GUI embeds two subsets of Adobe Source Han Sans SC 2.001 from upstream release
`2.001R`, commit `a69eda09a2fb57836e055bddb0a9a61b162ad1e6`.

| File | Runtime family | Original weight |
| --- | --- | --- |
| `BitNetHanSansSC-Regular-3755.otf` | `BitNet Han Sans SC` | Regular, 400 |
| `BitNetHanSansSC-Medium-3755.otf` | `BitNet Han Sans SC Medium` | Medium, 500 |

Both files contain ASCII, Latin-1, punctuation, arrows, fullwidth forms and GB2312
level-one characters. The GUI adds them to Iced's font database. System fonts
provide the remaining characters.

Subsetting changes the original fonts. The primary family, full and PostScript
names use `BitNet Han Sans SC` to comply with the reserved name `Source`. Original
copyright, author and license metadata are preserved. `LICENSE.txt` is Adobe's
original SIL Open Font License 1.1 file from the same release.

`manifest.json` records each source URL, source and output SHA-256, output size,
family name and the tool version. `scripts/generate-notices.py` verifies the
bundled files and includes the OFL text in the distribution notices. Desktop
packages also include it as `FONT-OFL.txt`.

To regenerate, install fontTools 4.62.1 in a development environment and place the
original `SourceHanSansSC-Regular.otf` and `SourceHanSansSC-Medium.otf` in a
directory. Then run from the project root

```text
python scripts/build-font-subsets.py <original-font-directory>
```

The script validates the source hashes before generating either file. Font
subsetting is a development step. Regular builds use the checked-in assets.

The upstream source and license are available at
[Adobe Source Han Sans 2.001R](https://github.com/adobe-fonts/source-han-sans/tree/a69eda09a2fb57836e055bddb0a9a61b162ad1e6).
