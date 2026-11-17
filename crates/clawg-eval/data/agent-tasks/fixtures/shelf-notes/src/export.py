"""Exports a shelf to one document."""

from pathlib import Path


def to_markdown(folder):
    """Return every note of the folder as one Markdown text."""
    parts = []
    for path in sorted(Path(folder).glob("*.txt")):
        parts.append("## " + path.stem + "\n\n" + path.read_text())
    return "\n".join(parts)
