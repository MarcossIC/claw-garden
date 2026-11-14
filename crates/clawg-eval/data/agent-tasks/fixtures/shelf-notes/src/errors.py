"""The errors the shelf raises."""


class ShelfError(Exception):
    """Base class of every error of the shelf."""


class NoteNotFound(ShelfError):
    """Raised when a note id does not exist on the shelf."""
