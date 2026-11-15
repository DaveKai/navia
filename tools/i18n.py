#!/usr/bin/env python3
"""Navia translations: one catalog in `locales/`, shared by GPUI and the Mac app.

  python3 -I tools/i18n.py extract   Rebuild locales/en.json from the keys the code uses.
  python3 -I tools/i18n.py check     Same keys and placeholders in every language.
  python3 -I tools/i18n.py sync      Copy the catalog into the Mac app's Resources.
  python3 -I tools/i18n.py scan      List user-visible text in the code that isn't translated.
  python3 -I tools/i18n.py stats     Counts and untranslated keys per language.

Keys are the English text. A value is a string, or a plural object with the CLDR categories
("one" and "other", plus "many" where a language has it). Placeholders are `{name}`.
`key||context` separates words that need different translations; the English shown is the part before `||`.
"""
import glob, json, os, re, shutil, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOCALES = os.path.join(ROOT, "locales")
LANGS = ["fr", "es", "de"]
APPLE_RES = os.path.join(ROOT, "apps/navia-apple/Navia/Resources/Locales")

SOURCES = {
    # The core's Songs filters and smart playlist editor also write catalog keys.
    "rust": sorted(glob.glob(os.path.join(ROOT, "apps/navia-gpui/src/*.rs"))) + [os.path.join(ROOT, "crates/navia-core/src/song_filters.rs")] + sorted(glob.glob(os.path.join(ROOT, "crates/navia-core/src/smart_editor/*.rs"))),
    "swift": sorted(glob.glob(os.path.join(ROOT, "apps/navia-apple/Navia/**/*.swift"), recursive=True)),
}
# Files whose strings never reach the user.
SKIP_FILES = {"sample.rs", "catalog.rs", "testing.rs", "SampleModels.swift", "Diagnostics.swift", "CoreCheck.swift"}
FN = {
    "rust": {"key": ["tr", "trf", "n_"], "plural": ["trn", "trnf"]},
    "swift": {"key": ["L", "N_"], "plural": ["Ln"]},
}
STR = r'("(?:[^"\\\n]|\\.)*"|\'(?:[^\'\\\n]|\\.)*\'|`(?:[^`\\]|\\.)*`)'

PLURAL_RE = re.compile(r"\{count\}")
PH = re.compile(r"\{([A-Za-z_][A-Za-z0-9_]*)\}")


def unescape(lit):
    q = lit[0]
    body = lit[1:-1]
    def repl(m):
        s = m.group(0)
        if s.startswith("\\u"):
            hexa = s[3:-1] if s[2:3] == "{" else s[2:]
            return chr(int(hexa, 16))
        return {"\\n": "\n", "\\t": "\t", '\\"': '"', "\\'": "'", "\\\\": "\\", "\\`": "`", "\\$": "$"}.get(s, s)
    return re.sub(r"\\u\{[0-9a-fA-F]+\}|\\u[0-9a-fA-F]{4}|\\.", repl, body), q


def strip_test_mods(src):
    """Drops `#[cfg(test)]` modules so test-only strings aren't extracted."""
    i = src.find("#[cfg(test)]")
    return src if i < 0 else src[:i]


def read(path):
    with open(path, encoding="utf8") as f:
        s = f.read()
    return strip_test_mods(s) if path.endswith(".rs") else s


def strip_comments(src, kind):
    return re.sub(r"(?m)^\s*//.*$", "", src)


def extract_keys():
    """Every key the code translates, with a string or plural dict as its value."""
    keys, problems = {}, []
    for kind, files in SOURCES.items():
        for path in files:
            if os.path.basename(path) in SKIP_FILES:
                continue
            src = strip_comments(read(path), kind)
            for fn in FN[kind]["key"]:
                for m in re.finditer(r"\b" + fn + r"\(\s*" + STR, src):
                    key, q = unescape(m.group(1))
                    if kind == "swift" and "\\(" in m.group(1):
                        problems.append(f"{rel(path)}: interpolation in a key: {key}")
                    keys.setdefault(key, key.split("||")[0])
            for fn in FN[kind]["plural"]:
                for m in re.finditer(r"\b" + fn + r"\(\s*" + STR + r"\s*,\s*" + STR, src):
                    one, _ = unescape(m.group(1))
                    other, _ = unescape(m.group(2))
                    keys[other] = {"one": one, "other": other.split("||")[0]}
    return keys, problems


def rel(p):
    return os.path.relpath(p, ROOT)


def dump(path, data):
    def fmt(v):
        return json.dumps(v, ensure_ascii=False)
    lines = [f" {fmt(k)}: {fmt(data[k])}" for k in sorted(data, key=lambda s: s.lower())]
    with open(path, "w", encoding="utf8") as f:
        f.write("{\n" + ",\n".join(lines) + "\n}\n")


def load(lang):
    path = os.path.join(LOCALES, f"{lang}.json")
    if not os.path.exists(path):
        return {}
    with open(path, encoding="utf8") as f:
        return json.load(f)


def forms(value):
    return value if isinstance(value, dict) else {"": value}


def cmd_extract():
    keys, problems = extract_keys()
    for p in problems:
        print("PROBLEM", p)
    os.makedirs(LOCALES, exist_ok=True)
    old = load("en")
    # Keys no longer in the code are dropped from every language.
    dump(os.path.join(LOCALES, "en.json"), keys)
    removed = sorted(set(old) - set(keys))
    added = sorted(set(keys) - set(old))
    print(f"en: {len(keys)} keys, {len(added)} added, {len(removed)} removed")
    for r in removed[:40]:
        print("  -", r)
    for lang in LANGS:
        data = load(lang)
        pruned = {k: v for k, v in data.items() if k in keys}
        dump(os.path.join(LOCALES, f"{lang}.json"), pruned)
    return 0


def cmd_check():
    en = load("en")
    bad = 0
    def err(msg):
        nonlocal bad
        bad += 1
        print("ERROR", msg)
    code, problems = extract_keys()
    for p in problems:
        err(p)
    if set(code) != set(en):
        for k in sorted(set(code) - set(en))[:50]:
            err(f"en.json is missing a key the code uses: {k!r}")
        for k in sorted(set(en) - set(code))[:50]:
            err(f"en.json has a key no code uses: {k!r}")
    for k, v in en.items():
        if isinstance(v, str) and v != k.split("||")[0]:
            err(f"en value differs from its key: {k!r} = {v!r}")
    for lang in LANGS:
        data = load(lang)
        for k in sorted(set(en) - set(data)):
            err(f"{lang}: missing key {k!r}")
        for k in sorted(set(data) - set(en)):
            err(f"{lang}: extra key {k!r}")
        for k in en:
            if k not in data:
                continue
            ev, v = en[k], data[k]
            if isinstance(ev, dict) != isinstance(v, dict):
                err(f"{lang}: {k!r} must be {'plural' if isinstance(ev, dict) else 'a plain string'}")
                continue
            ef = forms(ev)
            for name, text in forms(v).items():
                if not isinstance(text, str) or not text.strip():
                    err(f"{lang}: {k!r} form {name!r} is empty")
                    continue
                target = ef.get(name, ef.get("other", ef.get("")))
                want = set(PH.findall(target)) | set(PH.findall(ef.get("other", target)))
                got = set(PH.findall(text))
                # The singular may spell out the count, as in "Un titre", but nothing else may differ.
                if name in ("one",):
                    if got - want or (want - got) - {"count"}:
                        err(f"{lang}: {k!r} form {name!r} placeholders {sorted(got)} vs {sorted(want)}")
                elif got != want:
                    err(f"{lang}: {k!r} form {name!r} placeholders {sorted(got)} vs {sorted(want)}")
            if isinstance(v, dict):
                needed = {"one", "other"}
                if not needed.issubset(v):
                    err(f"{lang}: {k!r} plural needs {sorted(needed)}")
    if os.path.isdir(APPLE_RES):
        for lang in ["en", *LANGS]:
            a, b = os.path.join(LOCALES, f"{lang}.json"), os.path.join(APPLE_RES, f"{lang}.json")
            if not os.path.exists(b) or open(a, "rb").read() != open(b, "rb").read():
                err(f"apple Resources copy of {lang}.json is out of date (run sync)")
    print("OK" if not bad else f"{bad} problems")
    return 1 if bad else 0


def cmd_sync():
    os.makedirs(APPLE_RES, exist_ok=True)
    for lang in ["en", *LANGS]:
        shutil.copy(os.path.join(LOCALES, f"{lang}.json"), os.path.join(APPLE_RES, f"{lang}.json"))
    print("synced to", rel(APPLE_RES))
    return 0


def cmd_stats():
    en = load("en")
    for lang in LANGS:
        d = load(lang)
        missing = [k for k in en if k not in d]
        print(f"{lang}: {len(d)}/{len(en)} ({len(missing)} missing)")
    return 0


ALLOW = os.path.join(ROOT, "locales/scan-allow.txt")


def cmd_scan():
    allow = set()
    if os.path.exists(ALLOW):
        with open(ALLOW, encoding="utf8") as f:
            allow = {l.rstrip("\n") for l in f if l.strip() and not l.startswith("#")}
    total = 0
    for kind, files in SOURCES.items():
        for path in files:
            if os.path.basename(path) in SKIP_FILES:
                continue
            src = strip_comments(read(path), kind)
            wrap = FN[kind]["key"] + FN[kind]["plural"]
            # The second English form of a plural call is translated too.
            seconds = set()
            for fn in FN[kind]["plural"]:
                for m in re.finditer(r"\b" + fn + r"\(\s*" + STR + r"\s*,\s*" + STR, src):
                    seconds.add(m.start(2))
            pattern = r'("(?:[^"\\\n]|\\.)*")'
            for m in re.finditer(pattern, src):
                before = src[max(0, m.start() - 12):m.start()]
                if re.search(r"\b(" + "|".join(wrap) + r")\(\s*$", before) or m.start() in seconds:
                    continue
                text, q = unescape(m.group(1))
                if text.strip() in allow or not looks_visible(text, kind):
                    continue
                line = src.count("\n", 0, m.start()) + 1
                print(f"{rel(path)}:{line}: {text}")
                total += 1
    print(total, "candidates")
    return 0


def looks_visible(text, kind):
    if not re.search(r"[A-Za-z]{2}", text) or "\n" in text:
        return False
    if re.fullmatch(r"[A-Za-z0-9_.:\-/]+", text) and not re.match(r"[A-Z]", text):
        return False  # identifiers, paths, keys
    if re.match(r"^[#.\d%(]", text) or text.startswith(("http", "--")):
        return False
    if re.fullmatch(r"[a-z-]+(\.[a-z-]+)*", text):
        return False
    return bool(re.match(r"^[A-Z{\\]", text.strip())) or (" " in text and bool(re.search(r"[a-z]{3}", text)))


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("extract", "check", "sync", "scan", "stats"):
        print(__doc__)
        return 2
    return {"extract": cmd_extract, "check": cmd_check, "sync": cmd_sync, "scan": cmd_scan, "stats": cmd_stats}[sys.argv[1]]()


if __name__ == "__main__":
    sys.exit(main())
