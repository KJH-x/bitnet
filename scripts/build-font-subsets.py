#!/usr/bin/env python3
"""Rebuild the two bundled desktop font subsets using fontTools 4.62.1."""

import argparse
import hashlib
import json
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont


PROJECT = Path(__file__).resolve().parent.parent
UPSTREAM = "https://github.com/adobe-fonts/source-han-sans"
COMMIT = "a69eda09a2fb57836e055bddb0a9a61b162ad1e6"
SOURCES = {
    "Regular": "a2125a726118e672952ebc914bdf3e7ef4b8f7c19f56c1044b747a02d107a111",
    "Medium": "b6d06ae567c25e347e5f04026179277aab7c2bf495dc4ea88a67a95e365deba7",
}
LICENSE_SHA256 = "0983abed9371adc8958c321bf8d8036324b4a2e8cc324682b0fe0c699e63a009"


def character_set():
    ranges = [(0x20, 0x7F), (0xA0, 0x100), (0x2000, 0x2070), (0x2190, 0x2200),
              (0x3000, 0x3040), (0xFF01, 0xFF5F), (0xFFE0, 0xFFE7)]
    characters = {chr(codepoint) for start, end in ranges for codepoint in range(start, end)}
    for high in range(0xB0, 0xD8):
        for low in range(0xA1, 0xFF):
            try:
                character = bytes([high, low]).decode("gb2312")
            except UnicodeDecodeError:
                continue
            if "\u4e00" <= character <= "\u9fff":
                characters.add(character)
    return characters


def rename(font, style):
    family = "BitNet Han Sans SC" + (" Medium" if style == "Medium" else "")
    postscript = f"BitNetHanSansSC-{style}"
    names = {
        1: family, 2: "Regular", 3: f"2.001;BitNet;{postscript}-3755",
        4: family, 6: postscript, 16: family, 17: "Regular",
        18: family, 21: family, 22: "Regular", 25: postscript,
    }
    for record in font["name"].names:
        if record.nameID in names:
            record.string = names[record.nameID].encode(record.getEncoding())
    for name_id in (1, 2, 4, 6, 16, 17):
        font["name"].setName(names[name_id], name_id, 3, 1, 0x409)
    cff = font["CFF "].cff
    cff.fontNames = [postscript]
    top = cff.topDictIndex[0]
    top.FamilyName = family
    top.FullName = family
    return family


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source_dir", type=Path, help="Directory with the original 2.001R OTF files")
    parser.add_argument("--output-dir", type=Path, default=PROJECT / "crates/bitnet-gui/assets/fonts")
    args = parser.parse_args()
    if fontTools.__version__ != "4.62.1":
        parser.error("Rebuilding checked-in assets requires fontTools 4.62.1")
    output = args.output_dir.resolve()
    if not output.is_relative_to(PROJECT):
        parser.error("Output must be inside this project")
    license_file = output / "LICENSE.txt"
    if not license_file.is_file() or digest(license_file) != LICENSE_SHA256:
        parser.error("Place the original 2.001R LICENSE.txt in the output directory")
    for style, expected in SOURCES.items():
        source = args.source_dir / f"SourceHanSansSC-{style}.otf"
        if not source.is_file() or digest(source) != expected:
            parser.error(f"Source SHA256 mismatch for {source.name}")
    options = subset.Options()
    options.layout_features = ["*"]
    options.desubroutinize = True
    options.name_IDs = ["*"]
    options.drop_tables += ["DSIG"]
    options.recalc_timestamp = False
    characters = character_set()
    artifacts = []
    for style, expected in SOURCES.items():
        source = args.source_dir / f"SourceHanSansSC-{style}.otf"
        with TTFont(source, recalcTimestamp=False) as font:
            subsetter = subset.Subsetter(options=options)
            subsetter.populate(text="".join(sorted(characters)))
            subsetter.subset(font)
            family = rename(font, style)
            filename = f"BitNetHanSansSC-{style}-3755.otf"
            destination = output / filename
            font.save(destination)
        artifacts.append({
            "file": filename, "family": family, "style": style,
            "sha256": digest(destination), "bytes": destination.stat().st_size,
            "source_file": source.name, "source_sha256": expected,
            "source_url": f"{UPSTREAM.replace('github.com', 'raw.githubusercontent.com')}/{COMMIT}/OTF/SimplifiedChinese/{source.name}",
        })
    record = {
        "name": "BitNet Han Sans SC", "upstream_name": "Source Han Sans SC",
        "version": "2.001", "repository": UPSTREAM, "commit": COMMIT, "spdx": "OFL-1.1",
        "license_file": "LICENSE.txt", "license_sha256": LICENSE_SHA256,
        "license_url": f"{UPSTREAM.replace('github.com', 'raw.githubusercontent.com')}/{COMMIT}/LICENSE.txt",
        "subset": "ASCII, Latin-1, punctuation, arrows, fullwidth forms, GB2312 level 1",
        "fonttools": fontTools.__version__, "files": artifacts,
    }
    (output / "manifest.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"bytes": sum(item["bytes"] for item in artifacts), "files": artifacts}))


if __name__ == "__main__":
    main()
