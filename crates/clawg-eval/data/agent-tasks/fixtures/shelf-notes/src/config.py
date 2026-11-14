"""Where the shelf keeps its notes."""

import os

DEFAULT_FOLDER = "notes"


def shelf_folder():
    """Return the folder of the shelf, from the environment or the default."""
    return os.environ.get("SHELF_FOLDER", DEFAULT_FOLDER)
