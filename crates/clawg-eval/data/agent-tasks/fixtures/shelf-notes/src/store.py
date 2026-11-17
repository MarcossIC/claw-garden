"""Reads and writes the notes of a shelf."""

from pathlib import Path

from errors import NoteNotFound
from notes import Note


def save(folder, note):
    """Write a note into the folder of the shelf."""
    path = Path(folder) / note.filename()
    path.write_text(note.body)
    return path


def load(folder, title):
    """Read the note called title from the folder of the shelf."""
    path = Path(folder) / Note(title).filename()
    if not path.exists():
        raise NoteNotFound(title)
    return Note(title, path.read_text())
