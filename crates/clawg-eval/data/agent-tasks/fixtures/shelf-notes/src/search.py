"""Searches the notes of a shelf."""

from pathlib import Path


def find(folder, word):
    """Return the names of the notes whose text holds word."""
    hits = []
    for path in sorted(Path(folder).glob("*.txt")):
        if word.lower() in path.read_text().lower():
            hits.append(path.stem)
    return hits
