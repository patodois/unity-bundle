#!/usr/bin/env python3
"""An independent survey of the bundles of an installed game (Nocturne HD by default).

It shares no code with the crate (it has its own small LZ4 decoder), reads only the header, the
block info and the first block of every bundle, by seeking, and prints the facts that
docs/FORMATS.md states about the container. usage: python3 -I tools/survey.py [GAME_DIR]
"""
import os, struct, sys, collections
def lz4(src, n):
    out = bytearray(); i = 0
    while i < len(src):
        t = src[i]; i += 1
        l = t >> 4
        if l == 15:
            while True:
                b = src[i]; i += 1; l += b
                if b != 255: break
        out += src[i:i+l]; i += l
        if i >= len(src): break
        off = src[i] | src[i+1] << 8; i += 2
        m = t & 15
        if m == 15:
            while True:
                b = src[i]; i += 1; m += b
                if b != 255: break
        m += 4
        if off >= m:
            s = len(out) - off; out += out[s:s+m]
        else:
            for _ in range(m): out.append(out[-off])
    assert len(out) == n
    return bytes(out)
root = os.path.expanduser(sys.argv[1] if len(sys.argv) > 1 else "~/Games/smt3hd")
files = []
for d, _, fs in os.walk(os.path.join(root, "smt3hd_Data/StreamingAssets/PC")):
    files += [os.path.join(d, f) for f in fs]
for d, _, fs in os.walk(os.path.join(root, "dlc")):
    files += [os.path.join(d, f) for f in fs]
C = collections.Counter()
S = collections.defaultdict(set)
for path in sorted(files):
    with open(path, "rb") as f:
        head = f.read(256)
        if not head.startswith(b"UnityFS\0"):
            C["not UnityFS"] += 1; continue
        p = 8
        ver = struct.unpack(">I", head[p:p+4])[0]; p += 4
        e = head.index(0, p); player = head[p:e].decode(); p = e + 1
        e = head.index(0, p); unity = head[p:e].decode(); p = e + 1
        size, csz, usz, flags = struct.unpack(">QIII", head[p:p+20]); p += 20
        S["format version"].add(ver); S["player"].add(player); S["unity"].add(unity); S["flags"].add(hex(flags))
        fsz = os.path.getsize(path)
        C["size field == file size" if size == fsz else "size field != file size"] += 1
        f.seek(p); info = lz4(f.read(csz), usz)
        C["hash zero" if info[:16] == bytes(16) else "hash NOT zero"] += 1
        q = 16
        nb, = struct.unpack(">i", info[q:q+4]); q += 4
        blocks = [struct.unpack(">IIH", info[q+10*k:q+10*k+10]) for k in range(nb)]; q += 10*nb
        ne, = struct.unpack(">i", info[q:q+4]); q += 4
        ents = []
        for k in range(ne):
            o, s, fl = struct.unpack(">qqI", info[q:q+20]); q += 20
            e = info.index(0, q); n = info[q:e].decode(); q = e + 1; ents.append((o, s, fl, n))
        C["info fully consumed" if q == len(info) else "info has trailing bytes"] += 1
        data = p + csz
        tot_c = sum(b[1] for b in blocks)
        C["blocks end at EOF" if data + tot_c == fsz else "blocks do not end at EOF"] += 1
        for u, c, fl in blocks:
            S["block flags"].add(hex(fl)); C["block flags %#x" % fl] += 1
            if fl == 0 and u != c: C["stored block with u != c"] += 1
        S["entry flags"].update(hex(e[2]) for e in ents)
        for e in ents:
            C["entries with flags %#x" % e[2]] += 1
            C["entries named *.%s" % (e[3].rsplit(".", 1)[-1] if "." in e[3] else "(none)")] += 1
        S["entry suffix"].update((e[3].rsplit(".", 1)[-1] if "." in e[3] else "(none)") for e in ents)
        C["entries named CAB-*"] += sum(1 for e in ents if e[3].startswith("CAB-"))
        C["entries"] += len(ents)
        # the first block: the serialized header
        u, c, fl = blocks[0]
        f.seek(data); raw = f.read(c)
        first = lz4(raw, u) if fl & 0x3f in (2, 3) else raw
        ser = [e for e in ents if e[2] & 4]
        if not ser:
            C["no serialized entry"] += 1; continue
        assert ser[0][0] == 0
        ms, fsz2, sv, do = struct.unpack(">IIII", first[:16]); end = first[16]
        S["serialized version"].add(sv); S["serialized endianness byte"].add(end)
        C["%s: Unity %s, serialized format %d" % ("dlc" if os.sep + "dlc" + os.sep in path else "base game", unity, sv)] += 1
        C["serialized: file_size == entry size" if fsz2 == ser[0][1] else "serialized: file_size != entry size"] += 1
        C["serialized: data_offset == round16(20 + metadata_size)" if do == (20 + ms + 15)//16*16 else "serialized: data_offset other"] += 1
        C["serialized: reserved zero" if first[17:20] == bytes(3) else "serialized: reserved nonzero"] += 1
        ents_sorted = sorted(ents)
        at = 0; ok = True
        for o, s, fl2, n in ents_sorted:
            ok &= (o == at); at = o + s
        ok &= at == sum(b[0] for b in blocks)
        C["entries contiguous, cover stream" if ok else "entries NOT contiguous"] += 1
        C["serialized entry is first in directory" if ents[0][2] & 4 else "serialized entry not first"] += 1
        C["bundles"] += 1
for k in sorted(C): print("%-60s %d" % (k, C[k]))
for k in sorted(S): print("%-30s %s" % (k, sorted(S[k], key=str)))
