#!/usr/bin/env python3
"""Compares the common-strings table and the class id table of this crate with the public tables
they were checked against (read-only; nothing from them is copied into the crate by this script).

usage: python3 -I tools/check_tables.py RABEX_CHECKOUT ASSETSTUDIO_CommonString.cs ASSETSTUDIO_ClassIDType.cs

RABEX_CHECKOUT is a checkout of github.com/jakobhellermann/RustyAssetBundleEXtractor-based fork at
revision 56dd8a4d8d5048b6ee7664eadc89d98343c66002; the two AssetStudio files are from
github.com/Perfare/AssetStudio at revision d158e864b556b5970709c2a52e47944d53aa98a2.
"""
import os, re, sys

here = os.path.dirname(os.path.abspath(__file__))
rabex, as_common, as_class = sys.argv[1:4]

mine = open(os.path.join(here, "..", "src", "common_strings.rs")).read()
mine = {int(o): n for n, o in re.findall(r'"([^"]*?)\\0",\s+// (\d+)', mine)}
rs = open(os.path.join(rabex, "src", "commonstring.rs")).read()
rabex_c = {int(o): n for o, n in re.findall(r'\((\d+), "((?:[^"\\]|\\.)*)"\)', rs)}
asc = open(as_common, encoding="utf-8-sig").read()
as_c = {int(o): n for o, n in re.findall(r'\{\s*(\d+)\s*,\s*"((?:[^"\\]|\\.)*)"\s*\}', asc)}
print("common strings: this crate %d entries, rabex %d, AssetStudio %d" % (len(mine), len(rabex_c), len(as_c)))
for name, src in (("rabex", rabex_c), ("AssetStudio", as_c)):
    bad = [(o, n, src.get(o)) for o, n in sorted(mine.items()) if src.get(o) != n]
    print("  entries of this crate not identical in %s: %s" % (name, bad if bad else "none"))
    print("  entries of %s past this crate's table: %d (first: %s)" % (name, len([o for o in src if o not in mine]), min((o, n) for o, n in src.items() if o not in mine)))

ids = open(os.path.join(here, "..", "src", "class_id.rs")).read()
ids = {int(i): n for i, n in re.findall(r'^\s+\((\d+), "(\w+)"\),', ids, re.M)}
cs = open(os.path.join(rabex, "src", "objects", "class_id.rs")).read()
consts = {n: int(v) for n, v in re.findall(r"pub const (\w+): ClassId = ClassId\((-?\d+)\);", cs)}
rabex_i = {consts[c]: n for c, n in re.findall(r'\(ClassId::(\w+), "([^"]+)"\)', cs) if c in consts}
as_i = {int(v): n for n, v in re.findall(r"^\s*(\w+)\s*=\s*(-?\d+)\s*,", open(as_class, encoding="utf-8-sig").read(), re.M)}
print("class ids: this crate %d names, rabex %d, AssetStudio %d" % (len(ids), len(rabex_i), len(as_i)))
for name, src in (("rabex", rabex_i), ("AssetStudio", as_i)):
    bad = [(i, n, src.get(i)) for i, n in sorted(ids.items()) if src.get(i) != n]
    print("  names of this crate not identical in %s: %s" % (name, bad if bad else "none"))
both = [i for i in ids if rabex_i.get(i) == ids[i] and as_i.get(i) == ids[i]]
print("  names confirmed by both: %d of %d" % (len(both), len(ids)))
