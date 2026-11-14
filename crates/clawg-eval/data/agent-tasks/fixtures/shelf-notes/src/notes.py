"""A note and how it is written to a file."""

from dataclasses import dataclass, field


@dataclass
class Note:
    """A note with a title, a body and its tags."""

    title: str
    body: str = ""
    tags: list = field(default_factory=list)

    def filename(self):
        """Return the file name the note is stored under."""
        return self.title.lower().replace(" ", "-") + ".txt"
