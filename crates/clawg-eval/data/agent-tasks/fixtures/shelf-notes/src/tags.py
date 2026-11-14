"""Tags of a note."""


def normalize(tag):
    """Return a tag in lowercase without surrounding spaces."""
    return tag.strip().lower()


def merge(first, second):
    """Return the tags of both lists once each, in order."""
    seen = []
    for tag in list(first) + list(second):
        tag = normalize(tag)
        if tag not in seen:
            seen.append(tag)
    return seen
