"""Script-independent spelling skeletons for Malayalam, Manglish and English text.

The 8-word separation check compares text in one script only, so a Manglish
transliteration of a Malayalam test message passes it (owner review of SFT-V4
batch 5). ``skeleton`` romanizes Malayalam script and then folds the spelling
variation of informal romanization (long vowels, aspirates, doubled letters,
w/v, zh/l…), so "Ravile shwasam muttal" and "രാവിലെ ശ്വാസം മുട്ടൽ" both
become "ravile svasam mutal".

It is a coarse phonetic key for leak detection, not a transliteration scheme.
Translation between languages is not detected.
"""

from __future__ import annotations

import re

VIRAMA = "്"

INDEPENDENT_VOWELS = {
    "അ": "a", "ആ": "aa", "ഇ": "i", "ഈ": "ii", "ഉ": "u", "ഊ": "uu", "ഋ": "ru",
    "എ": "e", "ഏ": "ee", "ഐ": "ai", "ഒ": "o", "ഓ": "oo", "ഔ": "au",
}
VOWEL_SIGNS = {
    "ാ": "aa", "ി": "i", "ീ": "ii", "ു": "u", "ൂ": "uu",
    "ൃ": "ru", "െ": "e", "േ": "ee", "ൈ": "ai", "ൊ": "o",
    "ോ": "oo", "ൌ": "au", "ൗ": "au",
}
CONSONANTS = {
    "ക": "k", "ഖ": "kh", "ഗ": "g", "ഘ": "gh", "ങ": "ng",
    "ച": "ch", "ഛ": "chh", "ജ": "j", "ഝ": "jh", "ഞ": "nj",
    "ട": "t", "ഠ": "th", "ഡ": "d", "ഢ": "dh", "ണ": "n",
    "ത": "th", "ഥ": "th", "ദ": "d", "ധ": "dh", "ന": "n",
    "പ": "p", "ഫ": "ph", "ബ": "b", "ഭ": "bh", "മ": "m",
    "യ": "y", "ര": "r", "ല": "l", "വ": "v", "ശ": "sh", "ഷ": "sh", "സ": "s",
    "ഹ": "h", "ള": "l", "ഴ": "zh", "റ": "r",
}
CHILLU = {"ൺ": "n", "ൻ": "n", "ർ": "r", "ൽ": "l", "ൾ": "l", "ൿ": "k"}
OTHER = {"ം": "m", "ഃ": "h", "‌": "", "‍": ""}

# Applied in order to lowercase Latin text.
FOLDS = [
    (r"zh", "l"), (r"w", "v"), (r"ph", "f"), (r"[kgcjtdpb]h", lambda m: m.group()[0]),
    (r"sh", "s"), (r"ch", "c"), (r"nj", "n"), (r"ng", "n"), (r"ee", "i"), (r"oo", "u"),
    (r"(.)\1+", r"\1"), (r"h", ""), (r"y(?=[aeiou])", "y"),
]


def romanize(text: str) -> str:
    out = []
    chars = list(text)
    for i, char in enumerate(chars):
        nxt = chars[i + 1] if i + 1 < len(chars) else ""
        if char in CONSONANTS:
            out.append(CONSONANTS[char])
            if nxt not in VOWEL_SIGNS and nxt != VIRAMA:
                out.append("a")
        elif char in VOWEL_SIGNS:
            out.append(VOWEL_SIGNS[char])
        elif char == VIRAMA:
            # A word-final virama is the half-u ("samvruthokaram"): "veedu".
            if not nxt or not ("ഀ" <= nxt <= "ൿ"):
                out.append("u")
        elif char in INDEPENDENT_VOWELS:
            out.append(INDEPENDENT_VOWELS[char])
        elif char in CHILLU:
            out.append(CHILLU[char])
        elif char in OTHER:
            out.append(OTHER[char])
        else:
            out.append(char)
    return "".join(out)


def skeleton_word(word: str) -> str:
    for pattern, repl in FOLDS:
        word = re.sub(pattern, repl, word)
    return word


def skeleton(text: str) -> list[str]:
    """Folded words of ``text`` in a common Latin spelling."""
    words = re.findall(r"[a-z0-9]+", romanize(text).casefold())
    return [skeleton_word(word) for word in words]


def skeleton_ngrams(text: str, n: int) -> set[tuple[str, ...]]:
    words = skeleton(text)
    return {tuple(words[i : i + n]) for i in range(len(words) - n + 1)}
